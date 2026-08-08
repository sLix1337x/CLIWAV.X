use crate::error::{ClimusicError, Result};
use crate::sources::{best_thumbnail, http_client, run_yt_dlp, TrackSource, UnifiedTrack};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;

/// Hard deadline for any single yt-dlp invocation (see `run_yt_dlp`).
const YT_DLP_TIMEOUT: Duration = Duration::from_secs(30);
/// User category pages fetch up to 100 entries per call — allow longer.
const YT_DLP_PAGE_TIMEOUT: Duration = Duration::from_secs(120);
/// Resolved stream URLs expire; cache briefly (same TTL as YouTube's).
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

pub struct SoundCloudSource {
    yt_dlp_path: String,
    cookies_from_browser: String,
    /// Resolved track-page URL -> (stream URL, fetched at). Saves a multi-
    /// second yt-dlp round-trip when replaying a track within the TTL.
    url_cache: HashMap<String, (String, Instant)>,
}

/// Resolve a track's artwork via SoundCloud's public oEmbed endpoint
/// (developers.soundcloud.com/docs/oembed) — no auth, no yt-dlp needed.
///
/// This is the fallback for tracks listed on a user's Likes/Reposts/Tracks
/// page: that flat-playlist listing carries no thumbnail data at all (unlike
/// search results, which do), so `best_thumbnail` finds nothing for them.
/// oEmbed still resolves it from the track's page URL alone. Only called for
/// the single track that's actually playing, not per list entry, so it stays
/// cheap even for a 9,000-entry Likes list.
pub async fn fetch_oembed_thumbnail(track_url: &str) -> Result<Option<String>> {
    let resp = http_client(Duration::from_secs(15))
        .get("https://soundcloud.com/oembed")
        .query(&[("url", track_url), ("format", "json")])
        .send()
        .await
        .map_err(|e| ClimusicError::Source(format!("oEmbed request failed: {e}")))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let value: Value = resp
        .json()
        .await
        .map_err(|e| ClimusicError::Source(format!("oEmbed response parse failed: {e}")))?;

    Ok(value
        .get("thumbnail_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
}

impl SoundCloudSource {
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
        // private/gated content (private Likes, SoundCloud Go+ tracks).
        if !self.cookies_from_browser.is_empty() {
            cmd.args(["--cookies-from-browser", &self.cookies_from_browser]);
        }
        // So a timed-out (or otherwise dropped) call doesn't leave an
        // orphaned yt-dlp running in the background.
        cmd.kill_on_drop(true);
        cmd
    }

    /// Search SoundCloud via yt-dlp's generic search.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<UnifiedTrack>> {
        // yt-dlp supports soundcloud search via "scsearchN:query"
        let search_query = format!("scsearch{}:{}", limit, query);
        let mut cmd = self.base_command();
        cmd.args([
            "--no-update",
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

    /// Fetch one page of a user's Tracks / Likes / Reposts page.
    /// `category` is the SoundCloud URL suffix: "tracks", "likes", or "reposts".
    /// `start` is 1-indexed (matching yt-dlp's `--playlist-start`); `count` is
    /// the page size. Collections with thousands of entries (e.g. Likes) are
    /// paginated rather than fetched in one call so the UI never blocks for
    /// an unbounded amount of time.
    ///
    /// Returns the parsed tracks plus the RAW entry count yt-dlp produced
    /// (before parse filtering): pagination state must derive from what the
    /// source actually returned — entries that fail to parse would otherwise
    /// drift every later page (duplicates) or end paging early.
    pub async fn user_category(
        &self,
        username: &str,
        category: &str,
        start: usize,
        count: usize,
    ) -> Result<(Vec<UnifiedTrack>, usize)> {
        let username = username.trim().trim_start_matches('@');
        if username.is_empty() {
            return Err(ClimusicError::Source("no SoundCloud username set".into()));
        }
        let url = format!("https://soundcloud.com/{username}/{category}");
        let end = start + count.saturating_sub(1);

        let mut cmd = self.base_command();
        cmd.args([
            "--no-update",
            "--dump-single-json",
            "--flat-playlist",
            "--playlist-start",
            &start.to_string(),
            "--playlist-end",
            &end.to_string(),
            "--",
            &url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let output = run_yt_dlp(&mut cmd, YT_DLP_PAGE_TIMEOUT).await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ClimusicError::Source(format!(
                "could not load '{category}' for user '{username}': {stderr}"
            )));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let value: Value = serde_json::from_str(&json_str)
            .map_err(|e| ClimusicError::Source(format!("yt-dlp output parse error: {e}")))?;

        let entries = value
            .get("entries")
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();

        let raw_count = entries.len();
        let mut tracks = Vec::new();
        for entry in entries.iter().take(count) {
            if let Some(track) = parse_entry(entry) {
                tracks.push(track);
            }
        }
        Ok((tracks, raw_count))
    }

    /// Resolve a single SoundCloud track/URL directly to a `UnifiedTrack`,
    /// e.g. when a user pastes a share link into search instead of a query.
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

    /// Get a direct playable URL for a SoundCloud track URL.
    /// Results are cached for `CACHE_TTL` to avoid re-running yt-dlp.
    pub async fn get_audio_url(&mut self, url: &str) -> Result<String> {
        if let Some((cached_url, fetched_at)) = self.url_cache.get(url) {
            if fetched_at.elapsed() < CACHE_TTL {
                return Ok(cached_url.clone());
            }
        }

        let mut cmd = self.base_command();
        cmd.args([
            "--no-update",
            // See the matching comment in sources/youtube.rs: this forces
            // yt-dlp to prefer the highest-bitrate audio stream over its
            // default codec/language-first tie-breaking.
            "-S",
            "abr",
            "-f",
            "bestaudio",
            "--get-url",
            "--no-playlist",
            "--",
            url,
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

        // Prune expired entries on insert so the cache can't grow unbounded.
        self.url_cache
            .retain(|_, (_, fetched_at)| fetched_at.elapsed() < CACHE_TTL);
        self.url_cache
            .insert(url.to_string(), (audio_url.clone(), Instant::now()));
        Ok(audio_url)
    }

    /// Fallback for waveform analysis when a track has no `waveform_url` of
    /// its own to reuse: a low-bitrate stream, deliberately not the
    /// playback-quality one from `get_audio_url`, so the fallback doesn't
    /// roughly double the bandwidth of an already-streamed track. Mirrors
    /// `YouTubeSource::get_waveform_audio_url`.
    pub async fn get_waveform_audio_url(&self, url: &str) -> Result<String> {
        let mut cmd = self.base_command();
        cmd.args([
            "--no-update",
            "-f",
            "worstaudio",
            "--get-url",
            "--no-playlist",
            "--",
            url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let output = run_yt_dlp(&mut cmd, YT_DLP_TIMEOUT).await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ClimusicError::Source(format!("yt-dlp failed: {stderr}")));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let audio_url = stdout.lines().next().unwrap_or("").trim().to_string();
        if audio_url.is_empty() {
            return Err(ClimusicError::Source(
                "yt-dlp returned empty audio URL".into(),
            ));
        }
        Ok(audio_url)
    }
}

fn id_to_string(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        Some(s.to_string())
    } else {
        value.as_i64().map(|n| n.to_string())
    }
}

fn parse_entry(entry: &Value) -> Option<UnifiedTrack> {
    let id = entry.get("id").and_then(id_to_string)?;
    let title = entry
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();

    // `webpage_url` is only present on fully-resolved entries (e.g. search
    // results). Flat-playlist listings for a user's likes/reposts/tracks
    // page only carry `url` per entry — falling back to a numeric-id-based
    // guess produces an unresolvable URL, which is what made "yt-dlp
    // returned empty audio URL" show up when playing a Like.
    let url = entry
        .get("webpage_url")
        .and_then(|v| v.as_str())
        .or_else(|| entry.get("url").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("https://soundcloud.com/{}", id));

    let artist = entry
        .get("uploader")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| uploader_from_url(&url))
        .unwrap_or_default();

    let duration_ms = entry
        .get("duration")
        .and_then(|v| v.as_f64())
        .map(|d| (d * 1000.0) as u64);
    let thumbnail = best_thumbnail(entry);
    // SoundCloud publishes its own precomputed waveform per track — if
    // yt-dlp passes this field through, waveform generation can use it
    // directly instead of decoding audio itself. Not every entry shape is
    // guaranteed to carry it (e.g. flat-playlist listings may not), so this
    // is treated as a bonus, not a requirement — `None` here just means the
    // waveform fetch falls back to decoding.
    let waveform_url = entry
        .get("waveform_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Some(UnifiedTrack {
        id: id.to_string(),
        title,
        artist,
        album: String::new(),
        duration_ms,
        source: TrackSource::SoundCloud,
        playable_url: url,
        thumbnail_url: thumbnail,
        waveform_url,
    })
}

/// Flat-playlist entries for a user's likes/reposts/tracks page don't carry
/// an `uploader` field, so recover a display name from the URL's user slug
/// (e.g. "https://soundcloud.com/stmpdrcrds/track-name" -> "stmpdrcrds").
fn uploader_from_url(url: &str) -> Option<String> {
    url.strip_prefix("https://soundcloud.com/")
        .or_else(|| url.strip_prefix("http://soundcloud.com/"))
        .and_then(|rest| rest.split('/').next())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}
