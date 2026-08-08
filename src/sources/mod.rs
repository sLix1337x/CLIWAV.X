pub mod local;
pub mod soundcloud;
pub mod spotify;
pub mod youtube;

use crate::error::{ClimusicError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Run a yt-dlp child process with a hard deadline. Without this, a stalled
/// network fetch inside yt-dlp blocked the caller forever — and since search
/// and playback resolution are awaited directly on the UI event loop, one
/// hung yt-dlp froze the whole app.
pub async fn run_yt_dlp(
    cmd: &mut tokio::process::Command,
    timeout: Duration,
) -> Result<std::process::Output> {
    // kill_on_drop covers the direct child; set here too so callers can't
    // forget it.
    cmd.kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| ClimusicError::Source(format!("failed to run yt-dlp: {e}")))?;
    // Captured up front: wait_with_output consumes the child handle.
    let pid = child.id();

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(result) => {
            result.map_err(|e| ClimusicError::Source(format!("failed to run yt-dlp: {e}")))
        }
        Err(_) => {
            // The timed-out future was dropped, killing the direct child —
            // but under the Windows cmd/python wrapper the real yt-dlp is a
            // grandchild. Take down the whole tree, scoped to OUR pid only.
            #[cfg(windows)]
            if let Some(pid) = pid {
                let _ = tokio::process::Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &pid.to_string()])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await;
            }
            let _ = pid; // suppress unused warning on non-Windows
            Err(ClimusicError::Source(format!(
                "yt-dlp timed out after {}s",
                timeout.as_secs()
            )))
        }
    }
}

/// An HTTP client with a hard overall timeout — reqwest's default is no
/// timeout at all, so a stalled connection would hang its caller forever.
pub fn http_client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An "original"-tagged thumbnail with no usable url must not suppress
    /// the max-width fallback.
    #[test]
    fn original_without_url_falls_back_to_max_width() {
        let entry = serde_json::json!({"thumbnails": [
            {"id": "original"},
            {"url": "https://img/small.jpg", "width": 100},
            {"url": "https://img/large.jpg", "width": 500}
        ]});
        assert_eq!(best_thumbnail(&entry).as_deref(), Some("https://img/large.jpg"));
    }

    /// SoundCloud's "original" tag wins when it does carry a url.
    #[test]
    fn original_with_url_wins() {
        let entry = serde_json::json!({"thumbnails": [
            {"id": "original", "url": "https://img/orig.jpg"},
            {"url": "https://img/large.jpg", "width": 500}
        ]});
        assert_eq!(best_thumbnail(&entry).as_deref(), Some("https://img/orig.jpg"));
    }
}

/// Pick a thumbnail URL out of a yt-dlp flat-playlist entry. These never carry
/// a singular `thumbnail` field — only a `thumbnails` array of `{url, width,
/// height}` — so artwork silently never loaded until this looked there too.
/// Prefers the largest by width; SoundCloud's array additionally tags its
/// biggest entry `"id": "original"`, which has no `width` to compare by.
pub fn best_thumbnail(entry: &Value) -> Option<String> {
    if let Some(url) = entry.get("thumbnail").and_then(|v| v.as_str()) {
        return Some(url.to_string());
    }
    let thumbnails = entry.get("thumbnails")?.as_array()?;
    // Only entries with a usable url compete — an "original"-tagged entry
    // whose url is missing must not suppress the max-width fallback.
    fn valid_url(t: &Value) -> Option<&str> {
        t.get("url").and_then(|v| v.as_str())
    }
    thumbnails
        .iter()
        .find(|t| t.get("id").and_then(|v| v.as_str()) == Some("original") && valid_url(t).is_some())
        .or_else(|| {
            thumbnails
                .iter()
                .filter(|t| valid_url(t).is_some())
                .max_by_key(|t| t.get("width").and_then(|v| v.as_u64()).unwrap_or(0))
        })
        .and_then(valid_url)
        .map(|s| s.to_string())
}

/// A unified track representation across all sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>,
    pub source: TrackSource,
    /// For local files: absolute path. For web sources: playable URL or reference.
    pub playable_url: String,
    /// Optional thumbnail/cover URL.
    pub thumbnail_url: Option<String>,
    /// Precomputed waveform data URL from the source, if it publishes one
    /// (currently only SoundCloud) — lets waveform generation use the
    /// source's own amplitude data instead of decoding audio ourselves.
    /// `#[serde(default)]` so a queue saved before this field existed still
    /// deserializes.
    #[serde(default)]
    pub waveform_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackSource {
    Local,
    YouTube,
    SoundCloud,
    Spotify,
}

impl TrackSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrackSource::Local => "local",
            TrackSource::YouTube => "youtube",
            TrackSource::SoundCloud => "soundcloud",
            TrackSource::Spotify => "spotify",
        }
    }
}

impl std::fmt::Display for TrackSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
