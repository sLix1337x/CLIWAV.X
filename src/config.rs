use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

fn default_local_paths() -> Vec<String> {
    vec!["~/Music".to_string()]
}

fn default_mpv_path() -> String {
    "mpv".to_string()
}

fn default_yt_dlp_path() -> String {
    "yt-dlp".to_string()
}

fn default_volume() -> u8 {
    80
}

// Every section and field carries a serde default so a partial or
// hand-edited config.toml (e.g. from an older version, or with a section
// deleted) loads with sensible defaults instead of refusing to start.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub local: LocalConfig,
    #[serde(default)]
    pub spotify: SpotifyConfig,
    #[serde(default)]
    pub soundcloud: SoundCloudConfig,
    #[serde(default)]
    pub player: PlayerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    #[serde(default = "default_local_paths")]
    pub paths: Vec<String>,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            paths: default_local_paths(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpotifyConfig {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SoundCloudConfig {
    #[serde(default)]
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerConfig {
    #[serde(default = "default_mpv_path")]
    pub mpv_path: String,
    #[serde(default = "default_yt_dlp_path")]
    pub yt_dlp_path: String,
    #[serde(default = "default_volume")]
    pub volume: u8,
    /// Optional browser name passed to yt-dlp as `--cookies-from-browser`
    /// (e.g. "firefox", "chrome") — reuses the logged-in browser session so
    /// private Likes and subscriber/region-gated tracks resolve. Empty = off.
    #[serde(default)]
    pub cookies_from_browser: String,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            mpv_path: default_mpv_path(),
            yt_dlp_path: default_yt_dlp_path(),
            volume: default_volume(),
            cookies_from_browser: String::new(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            let default = Self::default();
            default.save()?;
            return Ok(default);
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config at {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("failed to parse config at {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // toml::to_string_pretty emits no comments, so prepend the guidance a
        // fresh file needs (note: saving rewrites the file, so hand-added
        // comments elsewhere in it don't survive).
        let content = format!(
            "# CLIWAV.X configuration\n\
             # mpv_path / yt_dlp_path may be full paths (e.g. a wrapper script).\n\
             # local.paths supports '~'. Get Spotify credentials at\n\
             # https://developer.spotify.com/dashboard\n\n{}",
            toml::to_string_pretty(self)?
        );
        fs::write(&path, content)
            .with_context(|| format!("failed to write config at {}", path.display()))?;
        Ok(())
    }

    pub fn config_path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("com", "climusic", "climusic")
            .context("could not determine project directories")?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    pub fn db_path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("com", "climusic", "climusic")
            .context("could not determine project directories")?;
        let data_dir = dirs.data_dir();
        fs::create_dir_all(data_dir)?;
        Ok(data_dir.join("library.db"))
    }

    pub fn cache_dir() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("com", "climusic", "climusic")
            .context("could not determine project directories")?;
        let cache_dir = dirs.cache_dir();
        fs::create_dir_all(cache_dir)?;
        Ok(cache_dir.to_path_buf())
    }

    pub fn queue_path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("com", "climusic", "climusic")
            .context("could not determine project directories")?;
        let data_dir = dirs.data_dir();
        fs::create_dir_all(data_dir)?;
        Ok(data_dir.join("queue.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config.toml missing whole sections (older versions, hand edits)
    /// must load with defaults instead of failing to start the app.
    #[test]
    fn partial_config_loads_with_defaults() {
        let cfg: Config = toml::from_str("[soundcloud]\nusername = \"someone\"\n").unwrap();
        assert_eq!(cfg.soundcloud.username, "someone");
        assert_eq!(cfg.local.paths, vec!["~/Music".to_string()]);
        assert!(cfg.spotify.client_id.is_empty());
        assert!(cfg.spotify.client_secret.is_empty());
        assert_eq!(cfg.player.mpv_path, "mpv");
        assert_eq!(cfg.player.yt_dlp_path, "yt-dlp");
        assert_eq!(cfg.player.volume, 80);
    }

    /// A section present but with keys missing fills just those keys.
    #[test]
    fn partial_section_fills_missing_keys() {
        let cfg: Config = toml::from_str("[player]\nvolume = 42\n").unwrap();
        assert_eq!(cfg.player.volume, 42);
        assert_eq!(cfg.player.mpv_path, "mpv");
    }
}
