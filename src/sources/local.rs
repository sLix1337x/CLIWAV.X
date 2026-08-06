use crate::error::Result;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::Accessor;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>,
    pub path: String,
}

impl LocalTrack {
    pub fn to_unified(&self) -> super::UnifiedTrack {
        super::UnifiedTrack {
            id: self.path.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration_ms: self.duration_ms,
            source: super::TrackSource::Local,
            playable_url: self.path.clone(),
            thumbnail_url: None,
        }
    }
}

/// Scan a directory recursively for audio files.
pub fn scan_directory<P: AsRef<Path>>(path: P) -> Result<Vec<LocalTrack>> {
    let mut tracks = Vec::new();
    let extensions: &[&str] = &["mp3", "flac", "ogg", "opus", "m4a", "aac", "wav", "wma"];

    for entry in WalkDir::new(path).follow_links(true) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                // Was swallowed silently — an unreadable subdir meant its
                // tracks vanished from the index with no trace.
                tracing::warn!("library scan: skipping unreadable entry: {e}");
                continue;
            }
        };
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                let ext = ext.to_string_lossy().to_lowercase();
                if extensions.contains(&ext.as_str()) {
                    match read_metadata(path) {
                        Ok(track) => tracks.push(track),
                        Err(e) => {
                            tracing::warn!("library scan: skipping {}: {e}", path.display())
                        }
                    }
                }
            }
        }
    }

    Ok(tracks)
}

pub fn read_metadata<P: AsRef<Path>>(path: P) -> Result<LocalTrack> {
    let path = path.as_ref();
    let tagged_file = Probe::open(path)?.read()?;
    let tag = tagged_file.primary_tag();

    let title = tag
        .and_then(|t| t.title().map(|s| s.to_string()))
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });
    let artist = tag
        .and_then(|t| t.artist().map(|s| s.to_string()))
        .unwrap_or_default();
    let album = tag
        .and_then(|t| t.album().map(|s| s.to_string()))
        .unwrap_or_default();
    let duration_ms = tagged_file.properties().duration().as_millis() as u64;

    Ok(LocalTrack {
        title,
        artist,
        album,
        duration_ms: Some(duration_ms),
        path: path.to_string_lossy().to_string(),
    })
}

/// Expand a path that starts with `~` (in any of its common forms — bare
/// `~`, `~/...`, or Windows-style `~\...`) to an absolute home-based path.
pub fn expand_path(input: &str) -> PathBuf {
    let rest = input
        .strip_prefix("~/")
        .or_else(|| input.strip_prefix("~\\"))
        .or(if input == "~" { Some("") } else { None });
    if let Some(rest) = rest {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_all_tilde_forms() {
        let Some(home) = dirs::home_dir() else {
            return; // no home dir in this environment — nothing to assert
        };
        assert_eq!(expand_path("~"), home);
        assert_eq!(expand_path("~/Music"), home.join("Music"));
        assert_eq!(expand_path("~\\Music"), home.join("Music"));
    }

    #[test]
    fn leaves_other_paths_alone() {
        assert_eq!(expand_path("C:/Music"), PathBuf::from("C:/Music"));
        assert_eq!(expand_path("/home/user/Music"), PathBuf::from("/home/user/Music"));
        // A tilde that isn't at the start is not a home shortcut.
        assert_eq!(expand_path("C:/~weird"), PathBuf::from("C:/~weird"));
    }
}
