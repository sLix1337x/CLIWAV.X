use crate::error::{ClimusicError, Result};
use crate::sources::{best_thumbnail, run_yt_dlp, TrackSource, UnifiedTrack};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;

const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
/// Hard deadline for any single yt-dlp invocation (see `run_yt_dlp`).
const YT_DLP_TIMEOUT: Duration = Duration::from_secs(30);

pub struct YouTubeSource {
    yt_dlp_path: String,
    cookies_from_browser: String,
    url_cache: HashMap<String, (String, Instant)>,
}

impl YouTubeSource {
    pub fn new(yt_dlp_path: impl Into<String>, cookies_from_browser: impl Into<String>) -> Self {
        Self {
            yt_dlp_path: yt_dlp_path.into(),
            cookies_from_browser: cookies_from_browser.into(),
            url_cache: HashMap::new(),
        }
    }

    /// Build a `Command` for yt-dlp, transparently handling Windows `.cmd`/`.bat` wrappers.
    fn base_command(&self) -> Command {
        let path = self.yt_dlp_path.trim();
        let lower = path.to_lowercase();
        let mut cmd = if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            let mut cmd = Command::new("cmd");
            cmd.args(["/c", path]);
            cmd
        } else {
            Command::new(path)
        };
        // Reuse the logged-in browser session when configured — required for
        // private/gated content.
        if !self.cookies_from_browser.is_empty() {
            cmd.args(["--cookies-from-browser", &self.cookies_from_browser]);
        }
        // So a timed-out (or otherwise dropped) call doesn't leave an
        // orphaned yt-dlp running in the background.
        cmd.kill_on_drop(true);
        cmd
    }

    /// Search YouTube and return up to `limit` results.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<UnifiedTrack>> {
        let search_query = format!("ytsearch{}:{}", limit, query);
        let mut cmd = self.base_command();
        cmd.args([
            "--no-update",
            "--default-search",
            "ytsearch",
            "--dump-single-json",
            "--flat-playlist",
            "--no-playlist",
            "--",
            &search_query,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let output = run_yt_dlp(&mut cmd, YT_DLP_TIMEOUT).await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ClimusicError::Source(format!("yt-dlp failed: {stderr}")));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let value: Value = serde_json::from_str(&json_str)
            .map_err(|e| ClimusicError::Source(format!("yt-dlp output parse error: {e}")))?;

        let entries = value
            .get("entries")
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();

        let mut tracks = Vec::new();
        for entry in entries.iter().take(limit) {
            if let Some(track) = parse_entry(entry) {
                tracks.push(track);
            }
        }
        Ok(tracks)
    }

    /// Resolve a single YouTube video/URL directly to a `UnifiedTrack`,
    /// e.g. when a user pastes a video link into search instead of a query.
    pub async fn resolve(&self, url: &str) -> Result<UnifiedTrack> {
        let mut cmd = self.base_command();
        cmd.args(["--no-update", "--dump-single-json", "--no-playlist", "--", url])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = run_yt_dlp(&mut cmd, YT_DLP_TIMEOUT).await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ClimusicError::Source(format!("yt-dlp failed: {stderr}")));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let value: Value = serde_json::from_str(&json_str)
            .map_err(|e| ClimusicError::Source(format!("yt-dlp output parse error: {e}")))?;

        parse_entry(&value)
            .ok_or_else(|| ClimusicError::Source("could not parse track info".into()))
    }

    /// Get a direct playable URL for a YouTube video ID or URL.
    /// Results are cached for `CACHE_TTL` to avoid re-running yt-dlp.
    pub async fn get_audio_url(&mut self, video_id_or_url: &str) -> Result<String> {
        let cache_key = video_id_or_url.to_string();

        if let Some((cached_url, fetched_at)) = self.url_cache.get(&cache_key) {
            if fetched_at.elapsed() < CACHE_TTL {
                return Ok(cached_url.clone());
            }
        }

        let url = if video_id_or_url.starts_with("http") {
            video_id_or_url.to_string()
        } else {
            format!("https://www.youtube.com/watch?v={}", video_id_or_url)
        };

        let mut cmd = self.base_command();
        cmd.args([
            "--no-update",
            // Bitrate ranks below language/codec preference in yt-dlp's
            // default format sort, so "bestaudio" alone can pick a lower-kbps
            // stream over a higher-kbps one of a less-preferred codec.
            // Putting abr first guarantees the highest-bitrate stream wins.
            "-S",
            "abr",
            "-f",
            "bestaudio",
            "--get-url",
            "--no-playlist",
            "--",
            &url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let output = run_yt_dlp(&mut cmd, YT_DLP_TIMEOUT).await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ClimusicError::Source(format!("yt-dlp failed: {stderr}")));
        }

        // `--get-url` can print more than one line for multi-format/playlist
        // entries; mpv's loadfile only ever wants a single URL.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let audio_url = stdout.lines().next().unwrap_or("").trim().to_string();
        if audio_url.is_empty() {
            return Err(ClimusicError::Source(
                "yt-dlp returned empty audio URL".into(),
            ));
        }

        // Prune expired entries on insert so the cache can't grow unbounded
        // over a long session (hits alone never evicted anything).
        self.url_cache
            .retain(|_, (_, fetched_at)| fetched_at.elapsed() < CACHE_TTL);
        self.url_cache
            .insert(cache_key, (audio_url.clone(), Instant::now()));
        Ok(audio_url)
    }
}

fn parse_entry(entry: &Value) -> Option<UnifiedTrack> {
    let id = entry.get("id")?.as_str()?;
    let title = entry
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(id)
        .to_string();
    let artist = entry
        .get("channel")
        .or_else(|| entry.get("uploader"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let duration_ms = entry
        .get("duration")
        .and_then(|v| v.as_u64())
        // Untrusted JSON — saturating, not `d * 1000` (debug-build overflow
        // panic on absurd values).
        .map(|d| d.saturating_mul(1000));
    let thumbnail = best_thumbnail(entry);

    Some(UnifiedTrack {
        id: id.to_string(),
        title,
        artist,
        album: String::new(),
        duration_ms,
        source: TrackSource::YouTube,
        playable_url: format!("https://www.youtube.com/watch?v={}", id),
        thumbnail_url: thumbnail,
    })
}
