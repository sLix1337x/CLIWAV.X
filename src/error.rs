use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClimusicError {
    #[error("player error: {0}")]
    Player(String),

    #[error("source error: {0}")]
    Source(String),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("spotify API error: {0}")]
    SpotifyApi(String),

    // Neutral label: this catches anyhow errors from anywhere in the app —
    // calling them all "config error" misdirected diagnosis.
    #[error("error: {0}")]
    Anyhow(#[from] anyhow::Error),

    #[error("metadata error: {0}")]
    Metadata(#[from] lofty::error::LoftyError),
}

pub type Result<T> = std::result::Result<T, ClimusicError>;

impl ClimusicError {
    /// A plain-language version for the status bar. yt-dlp's raw stderr is
    /// noisy and often meaningless ("ERROR: [soundcloud] 12345: ... HTTP
    /// Error 404 ..."), so the common failure shapes are translated;
    /// anything unrecognized falls through to the raw message.
    pub fn friendly(&self) -> String {
        let ClimusicError::Source(msg) = self else {
            return self.to_string();
        };
        let m = msg.to_lowercase();
        if m.contains("timed out") {
            "the source took too long to answer — try again".to_string()
        } else if m.contains("404") || m.contains("not found") {
            "track not found — deleted, private, or a dead link".to_string()
        } else if m.contains("403") || m.contains("forbidden") {
            "access denied — likely region-locked or account-gated".to_string()
        } else if m.contains("geo") {
            "track is geo-restricted in your region".to_string()
        } else if m.contains("not available") {
            "track is not available (gated, restricted, or deleted)".to_string()
        } else if m.contains("empty audio url") {
            "no playable stream found for this track".to_string()
        } else {
            self.to_string()
        }
    }
}
