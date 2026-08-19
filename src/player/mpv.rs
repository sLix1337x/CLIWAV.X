use crate::error::{ClimusicError, Result};
use crate::player::eq;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

pub struct MpvPlayer {
    mpv_path: String,
    pipe_path: String,
    child: Option<Child>,
    /// Last-set playback options, re-applied after a mid-session restart —
    /// a freshly respawned mpv would otherwise play at default volume 100
    /// and lose loop-file (which wedges Repeat-Track, since the app trusts
    /// mpv to loop internally in that mode).
    volume: Option<u8>,
    loop_file: Option<bool>,
    /// Run mpv's WASAPI output in exclusive mode (bit-perfect local
    /// playback, bypasses the Windows mixer) — see `PlayerConfig::audio_exclusive`.
    audio_exclusive: bool,
    /// Last-applied 10-band EQ gains, re-sent after a mid-session mpv
    /// restart the same way `volume`/`loop_file` are — a respawned mpv
    /// starts with an empty `af` chain otherwise.
    eq_gains: Option<eq::Gains>,
}

impl MpvPlayer {
    pub fn new(mpv_path: impl Into<String>, audio_exclusive: bool) -> Self {
        let mpv_path = mpv_path.into();
        let pid = std::process::id();
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pipe_path = mpv_pipe_path(pid, ts);
        Self {
            mpv_path,
            pipe_path,
            child: None,
            volume: None,
            loop_file: None,
            audio_exclusive,
            eq_gains: None,
        }
    }

    /// Start the persistent mpv process if not already running.
    pub async fn start(&mut self) -> Result<()> {
        if self.is_running().await {
            return Ok(());
        }

        // The pipe name embeds pid+nanoseconds, so collisions with other
        // climusic instances — or the user's own unrelated mpv processes —
        // are impossible. (An earlier version force-killed every mpv.exe on
        // the machine here, including ones the user had open for other
        // purposes.)
        let ipc_arg = format!("--input-ipc-server={}", self.pipe_path);
        let mut args = vec![
            "--no-video".to_string(),
            "--idle".to_string(),
            // Without this, mpv unloads the file at end-of-track and every
            // playback property (eof-reached, time-pos, ...) becomes
            // "property unavailable" — which is what made autoplay-on-EOF
            // impossible to detect. keep-open pauses on the finished file
            // instead, leaving eof-reached=true readable.
            "--keep-open=yes".to_string(),
            // mpv pushes its title to the Windows audio session / volume
            // mixer — the default title is the raw media URL, which for a
            // signed CDN stream is a wall of query parameters. Pin it to
            // the app name instead.
            "--title=CLIWAV.X".to_string(),
            ipc_arg,
        ];
        if self.audio_exclusive {
            // WASAPI exclusive mode bypasses the Windows audio mixer, so
            // local lossless files play at their true native sample rate
            // instead of being resampled to the mixer's shared format.
            args.push("--audio-exclusive=yes".to_string());
        }
        let mut cmd = Command::new(&self.mpv_path);
        cmd.args(&args)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            ClimusicError::Player(format!(
                "failed to start mpv from '{}': {e}. Is mpv installed and on PATH?",
                self.mpv_path
            ))
        })?;

        // Closing a terminal tab/window kills cliwavx.exe abruptly (no
        // Drop, no graceful shutdown), which used to leave mpv running —
        // and playing — as an orphaned process. Binding it to a
        // kill-on-close job object makes Windows tear it down as soon as
        // our own process handle closes, by any means (normal exit, panic,
        // or the terminal force-killing us).
        //
        // Deliberately Windows-only rather than unimplemented elsewhere:
        // Unix already has this. mpv is spawned into our process group, so
        // closing the terminal delivers SIGHUP to the whole group and mpv
        // exits with us — the very behaviour job objects exist to emulate.
        // Graceful exits are covered on both platforms by `main` calling
        // `player.stop()` before it restores the terminal.
        #[cfg(windows)]
        job::kill_with_current_process(&child);

        // Wait for the IPC pipe to actually appear instead of a fixed sleep —
        // a flat 500ms both wasted time on fast machines and raced ahead of
        // mpv on slow ones (the first command then failed with
        // file-not-found and aborted app startup).
        let mut pipe_ready = false;
        for _ in 0..100 {
            if !self.is_child_running(&mut child).await {
                break; // exited early — report stderr below
            }
            if ipc_ping(&self.pipe_path).await {
                pipe_ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        if !pipe_ready {
            // Only safe to read stderr to EOF when the child has exited;
            // a still-running mpv holds the pipe open and this would hang.
            let stderr = if self.is_child_running(&mut child).await {
                String::from("(mpv still running, but created no IPC pipe within 5s)")
            } else {
                match child.stderr.take() {
                    Some(mut err) => {
                        let mut buf = String::new();
                        use tokio::io::AsyncReadExt;
                        let _ = err.read_to_string(&mut buf).await;
                        buf
                    }
                    None => String::from("(no stderr captured)"),
                }
            };
            let _ = child.start_kill();
            return Err(ClimusicError::Player(format!(
                "mpv failed to start. stderr: {stderr}"
            )));
        }

        // mpv logs to stderr continuously; if nothing drains the pipe, the
        // OS buffer fills (~64 KB) and mpv blocks on write, silently
        // freezing all IPC mid-session.
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!("mpv: {line}");
                }
            });
        }

        self.child = Some(child);

        // Re-apply remembered options after a mid-session restart. Raw IPC
        // on purpose: the public setters route through ensure_started →
        // start(), which would make this function recursively async.
        if let Some(volume) = self.volume {
            send_and_read(
                json!({"command": ["set_property", "volume", volume.clamp(0, 100) as f64]}),
                &self.pipe_path,
            )
            .await?;
        }
        if let Some(loop_file) = self.loop_file {
            let value = if loop_file { json!("inf") } else { json!("no") };
            send_and_read(
                json!({"command": ["set_property", "loop-file", value]}),
                &self.pipe_path,
            )
            .await?;
        }
        if let Some(gains) = self.eq_gains {
            send_and_read(
                json!({"command": ["af", "set", eq::build_af_graph(&gains)]}),
                &self.pipe_path,
            )
            .await?;
        }
        Ok(())
    }

    async fn is_child_running(&self, child: &mut Child) -> bool {
        match child.try_wait() {
            Ok(None) => true,
            _ => false,
        }
    }

    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
        Ok(())
    }

    async fn is_running(&mut self) -> bool {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(None) => return true,
                _ => return false,
            }
        }
        false
    }

    /// Load a URL/file and start playing it. If `append` is true, add to queue instead.
    pub async fn load(&mut self, url: &str, append: bool) -> Result<()> {
        let mode = if append { "append-play" } else { "replace" };
        self.command(json!({"command": ["loadfile", url, mode]})).await?;
        if !append {
            // With --keep-open, mpv force-pauses when a track reaches its end,
            // and loadfile does NOT clear that flag — the track after an
            // auto-advance would start paused without this. The unpause must
            // come AFTER loadfile: unpausing the OLD, at-EOF file first makes
            // mpv resume it, instantly re-hit EOF and re-pause, and the new
            // file then inherits pause=yes — every auto-advanced track starts
            // paused and never reaches EOF, so autoplay looks dead.
            self.set_property("pause", false).await?;
        }
        Ok(())
    }

    pub async fn pause(&mut self) -> Result<()> {
        self.set_property("pause", true).await
    }

    pub async fn play(&mut self) -> Result<()> {
        self.set_property("pause", false).await
    }

    pub async fn toggle_pause(&mut self) -> Result<()> {
        self.command(json!({"command": ["cycle", "pause"]})).await?;
        Ok(())
    }

    pub async fn stop_playback(&mut self) -> Result<()> {
        self.command(json!({"command": ["stop"]})).await?;
        Ok(())
    }

    pub async fn next(&mut self) -> Result<()> {
        self.command(json!({"command": ["playlist-next"]})).await?;
        Ok(())
    }

    pub async fn previous(&mut self) -> Result<()> {
        self.command(json!({"command": ["playlist-prev"]})).await?;
        Ok(())
    }

    pub async fn set_volume(&mut self, volume: u8) -> Result<()> {
        let vol = volume.clamp(0, 100) as f64;
        self.set_property("volume", vol).await?;
        self.volume = Some(volume.clamp(0, 100));
        Ok(())
    }

    /// Enable/disable mpv's own single-file repeat (used for track-loop mode).
    pub async fn set_loop_file(&mut self, on: bool) -> Result<()> {
        let value = if on { json!("inf") } else { json!("no") };
        self.set_property("loop-file", value).await?;
        self.loop_file = Some(on);
        Ok(())
    }

    /// Apply the full 10-band EQ chain in one go. Uses `af set` (a wholesale
    /// filter-chain replace) rather than adding the filter once and sending
    /// incremental `af-command` deltas per band: empirically, `af-command`
    /// does not work reliably against this filter (tested against a live
    /// mpv v0.41 instance — it fails even for textbook runtime-commandable
    /// ffmpeg filters like `volume`'s own `volume` command, regardless of
    /// `target` glob). A full `af set` replace does work, but it can trigger
    /// a `playback-restart` event (i.e. a very small audible blip) — callers
    /// should debounce band edits rather than calling this per-keystroke.
    pub async fn set_eq(&mut self, gains: eq::Gains) -> Result<()> {
        self.command(json!({"command": ["af", "set", eq::build_af_graph(&gains)]}))
            .await?;
        self.eq_gains = Some(gains);
        Ok(())
    }

    /// True once the current file has played to the end and mpv is idling on it.
    pub async fn is_eof_reached(&mut self) -> Result<bool> {
        let resp = self
            .command(json!({"command": ["get_property", "eof-reached"]}))
            .await?;
        Ok(resp.get("data").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    /// mpv's actual pause state — authoritative over any app-side mirror
    /// flag (mpv can pause on its own, e.g. keep-open pausing at EOF).
    pub async fn is_paused(&mut self) -> Result<bool> {
        let resp = self
            .command(json!({"command": ["get_property", "pause"]}))
            .await?;
        Ok(resp.get("data").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    pub async fn seek(&mut self, seconds: f64) -> Result<()> {
        self.command(json!({"command": ["seek", seconds, "relative"]}))
            .await?;
        Ok(())
    }

    pub async fn get_position(&mut self) -> Result<f64> {
        let resp = self
            .command(json!({"command": ["get_property", "time-pos"]}))
            .await?;
        Ok(resp.get("data").and_then(|v| v.as_f64()).unwrap_or(0.0))
    }

    pub async fn get_duration(&mut self) -> Result<f64> {
        let resp = self
            .command(json!({"command": ["get_property", "duration"]}))
            .await?;
        Ok(resp.get("data").and_then(|v| v.as_f64()).unwrap_or(0.0))
    }

    pub async fn get_metadata(&mut self) -> Result<Value> {
        self.command(json!({"command": ["get_property", "metadata"]}))
            .await
    }

    async fn set_property<T: serde::Serialize>(&mut self, name: &str, value: T) -> Result<()> {
        self.command(json!({"command": ["set_property", name, value]}))
            .await?;
        Ok(())
    }

    async fn command(&mut self, payload: Value) -> Result<Value> {
        self.ensure_started().await?;
        send_and_read(payload, &self.pipe_path).await
    }

    async fn ensure_started(&mut self) -> Result<()> {
        if !self.is_running().await {
            // mpv died mid-session (crash, killed externally) — restart it
            // instead of erroring on every command until the app exits.
            self.start().await?;
        }
        Ok(())
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        // Known and accepted: start_kill can't be awaited in Drop, so the
        // child isn't reaped here — Drop runs at process exit and the OS
        // cleans up. Documented won't-fix from the audit round.
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }
}

/// Hard deadline for one IPC round-trip: a hung mpv must surface as an
/// error in the status bar, not freeze the event loop forever.
const IPC_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(windows)]
fn mpv_pipe_path(pid: u32, ts: u128) -> String {
    format!(r"\\.\pipe\climusic-mpv-{pid}-{ts}")
}

#[cfg(unix)]
fn mpv_pipe_path(pid: u32, ts: u128) -> String {
    // A real filesystem socket path — reusing the Windows pipe name here
    // littered the process's cwd with a stray socket file.
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    format!("{dir}/climusic-mpv-{pid}-{ts}.sock")
}

#[cfg(windows)]
async fn ipc_ping(path: &str) -> bool {
    tokio::net::windows::named_pipe::ClientOptions::new()
        .open(path)
        .is_ok()
}

#[cfg(unix)]
async fn ipc_ping(path: &str) -> bool {
    tokio::net::UnixStream::connect(path).await.is_ok()
}

#[cfg(windows)]
async fn send_and_read(payload: Value, path: &str) -> Result<Value> {
    match tokio::time::timeout(IPC_TIMEOUT, send_and_read_inner(payload, path)).await {
        Ok(result) => result,
        // Known and accepted (documented won't-fix): if the timeout fires
        // mid-command, mpv may still have executed it — the resulting state
        // ambiguity is bounded and self-corrects on the next command. Full
        // request_id correlation would close this for marginal gain.
        Err(_) => Err(ClimusicError::Player(format!(
            "mpv did not answer within {}s (hung?)",
            IPC_TIMEOUT.as_secs()
        ))),
    }
}

#[cfg(unix)]
async fn send_and_read(payload: Value, path: &str) -> Result<Value> {
    match tokio::time::timeout(IPC_TIMEOUT, send_and_read_inner(payload, path)).await {
        Ok(result) => result,
        Err(_) => Err(ClimusicError::Player(format!(
            "mpv did not answer within {}s (hung?)",
            IPC_TIMEOUT.as_secs()
        ))),
    }
}

#[cfg(windows)]
async fn send_and_read_inner(payload: Value, path: &str) -> Result<Value> {
    use tokio::net::windows::named_pipe::ClientOptions;
    let mut client = ClientOptions::new()
        .open(path)
        .map_err(|e| ClimusicError::Player(format!("failed to connect to mpv IPC pipe: {e}")))?;

    let mut request = payload.to_string();
    request.push('\n');
    client.write_all(request.as_bytes()).await?;

    let mut reader = BufReader::new(client);
    read_response(&mut reader).await
}

#[cfg(unix)]
async fn send_and_read_inner(payload: Value, path: &str) -> Result<Value> {
    use tokio::net::UnixStream;
    let mut stream = UnixStream::connect(path)
        .await
        .map_err(|e| ClimusicError::Player(format!("failed to connect to mpv IPC socket: {e}")))?;

    let mut request = payload.to_string();
    request.push('\n');
    stream.write_all(request.as_bytes()).await?;

    let mut reader = BufReader::new(stream);
    read_response(&mut reader).await
}

/// Read lines until the command's response arrives. mpv broadcasts
/// asynchronous events (`{"event": ...}`) on every IPC connection, and one
/// can land ahead of our response — e.g. `end-file` fires exactly when a
/// track finishes, which is precisely when autoplay sends its next command.
/// Responses are distinguishable: they always carry an `error` field.
async fn read_response<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<Value> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Err(ClimusicError::Player(
                "mpv closed the IPC connection without responding".into(),
            ));
        }
        let value: Value = serde_json::from_str(&line)?;
        if let Some(error) = value.get("error").and_then(|e| e.as_str()) {
            if error != "success" {
                return Err(ClimusicError::Player(format!("mpv error: {line}")));
            }
            return Ok(value);
        }
    }
}

/// Ties mpv's lifetime to ours via a Windows job object, so it can't outlive
/// cliwavx.exe as an orphaned, still-playing process — which is what used to
/// happen when a terminal tab/window was closed: Windows kills the console
/// process abruptly (no unwinding, `Drop for MpvPlayer` never runs), but a
/// plain child process has no OS-level link to its parent and just keeps
/// running. A job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` fixes that
/// at the kernel level: Windows closes our handle to the job the moment our
/// process ends, by any means, and that closure kills every process still
/// assigned to it.
#[cfg(windows)]
mod job {
    use std::sync::OnceLock;
    use tokio::process::Child;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    // The handle is intentionally never closed: it must stay open for the
    // life of the process so Windows only reclaims it (triggering the kill)
    // when cliwavx.exe itself exits. Stored as `isize` rather than `HANDLE`
    // (`*mut c_void`) purely so the `OnceLock` is `Send + Sync`.
    static KILL_ON_CLOSE_JOB: OnceLock<isize> = OnceLock::new();

    /// Best-effort: on any failure this silently leaves mpv unprotected
    /// rather than blocking playback over what's ultimately a cleanup nicety.
    pub fn kill_with_current_process(child: &Child) {
        let job = *KILL_ON_CLOSE_JOB.get_or_init(create_kill_on_close_job);
        let Some(process) = child.raw_handle() else {
            return; // already exited before we could protect it
        };
        if job == 0 {
            return;
        }
        unsafe {
            let _ = AssignProcessToJobObject(job as HANDLE, process as HANDLE);
        }
    }

    /// Returns 0 (null) on failure, matched by callers instead of unwrapping.
    fn create_kill_on_close_job() -> isize {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return 0;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                return 0;
            }
            job as isize
        }
    }
}
