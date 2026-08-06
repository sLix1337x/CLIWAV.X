use crate::error::{ClimusicError, Result};
use crate::sources::local::LocalTrack;
use crate::sources::{TrackSource, UnifiedTrack};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        // foreign_keys is OFF by default in SQLite, which silently disabled
        // the schema's ON DELETE CASCADE / SET NULL — orphaned playlist rows
        // and dangling track references. busy_timeout keeps a background
        // rescan from failing with "database is locked" against app traffic.
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS tracks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                album TEXT NOT NULL DEFAULT '',
                duration_ms INTEGER,
                path TEXT NOT NULL UNIQUE,
                source TEXT NOT NULL DEFAULT 'local'
            );

            CREATE INDEX IF NOT EXISTS idx_tracks_title ON tracks(title);
            CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
            CREATE INDEX IF NOT EXISTS idx_tracks_path ON tracks(path);

            CREATE TABLE IF NOT EXISTS playlists (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                source TEXT NOT NULL DEFAULT 'local'
            );

            CREATE TABLE IF NOT EXISTS playlist_tracks (
                playlist_id INTEGER NOT NULL,
                track_id INTEGER,
                external_url TEXT,
                source TEXT NOT NULL,
                position INTEGER NOT NULL,
                FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
                FOREIGN KEY (track_id) REFERENCES tracks(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS saved_tracks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                album TEXT NOT NULL DEFAULT '',
                duration_ms INTEGER,
                playable_url TEXT NOT NULL UNIQUE,
                source TEXT NOT NULL,
                thumbnail_url TEXT,
                external_id TEXT
            );
            "#,
        )?;
        Ok(())
    }

    pub fn search_local(&self, query: &str, limit: usize) -> Result<Vec<LocalTrack>> {
        // Escape LIKE metacharacters so the query matches literally —
        // otherwise typing "%" or "_" acts as a wildcard (a bare "%" used to
        // match every track in the library).
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let mut stmt = self.conn.prepare(
            r#"
            SELECT title, artist, album, duration_ms, path
            FROM tracks
            WHERE source = 'local'
              AND (title LIKE ?1 ESCAPE '\' OR artist LIKE ?1 ESCAPE '\' OR album LIKE ?1 ESCAPE '\')
            ORDER BY artist, title
            LIMIT ?2
            "#,
        )?;
        let rows = stmt.query_map((&pattern, limit), |row| {
            Ok(LocalTrack {
                title: row.get(0)?,
                artist: row.get(1)?,
                album: row.get(2)?,
                duration_ms: row.get(3)?,
                path: row.get(4)?,
            })
        })?;

        let mut tracks = Vec::new();
        for row in rows {
            tracks.push(row?);
        }
        Ok(tracks)
    }

    pub fn get_local_track_by_path(&self, path: &str) -> Result<Option<LocalTrack>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT title, artist, album, duration_ms, path
            FROM tracks
            WHERE path = ?1
            "#,
        )?;
        let result = stmt
            .query_row([path], |row| {
                Ok(LocalTrack {
                    title: row.get(0)?,
                    artist: row.get(1)?,
                    album: row.get(2)?,
                    duration_ms: row.get(3)?,
                    path: row.get(4)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    /// Atomically rebuild the local index from a fresh scan. Upserts are
    /// path-keyed, so row ids — and therefore playlist references — survive
    /// a rescan; only files that genuinely disappeared are deleted (which,
    /// with foreign keys on, NULLs the playlist reference instead of breaking
    /// the join). One transaction, so a mid-scan failure can't leave a
    /// half-empty index. Replaces the old DELETE-all + re-INSERT sync, which
    /// reassigned every id and orphaned every playlist reference.
    pub fn sync_local_tracks(&mut self, tracks: &[LocalTrack]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS scanned_paths(path TEXT PRIMARY KEY);
             DELETE FROM scanned_paths;",
        )?;
        {
            let mut seen = tx.prepare("INSERT OR IGNORE INTO scanned_paths(path) VALUES (?1)")?;
            let mut upsert = tx.prepare(
                r#"
                INSERT INTO tracks (title, artist, album, duration_ms, path, source)
                VALUES (?1, ?2, ?3, ?4, ?5, 'local')
                ON CONFLICT(path) DO UPDATE SET
                    title = excluded.title,
                    artist = excluded.artist,
                    album = excluded.album,
                    duration_ms = excluded.duration_ms
                "#,
            )?;
            for track in tracks {
                seen.execute([&track.path])?;
                upsert.execute((
                    &track.title,
                    &track.artist,
                    &track.album,
                    track.duration_ms,
                    &track.path,
                ))?;
            }
        }
        tx.execute(
            "DELETE FROM tracks WHERE source = 'local' AND path NOT IN (SELECT path FROM scanned_paths)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn create_playlist(&self, name: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO playlists (name, source) VALUES (?1, 'local')",
            [name],
        )?;
        let id: i64 = self
            .conn
            .query_row(
                "SELECT id FROM playlists WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .map_err(|e| ClimusicError::Database(e))?;
        Ok(id)
    }

    pub fn add_track_to_playlist(
        &self,
        playlist_id: i64,
        track_id: Option<i64>,
        external_url: Option<&str>,
        source: &str,
    ) -> Result<()> {
        let position: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
            [playlist_id],
            |row| row.get(0),
        )?;
        self.conn.execute(
            r#"
            INSERT INTO playlist_tracks (playlist_id, track_id, external_url, source, position)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            (playlist_id, track_id, external_url, source, position),
        )?;
        Ok(())
    }

    pub fn list_playlists(&self) -> Result<Vec<Playlist>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name FROM playlists ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok(Playlist {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })?;
        let mut playlists = Vec::new();
        for row in rows {
            playlists.push(row?);
        }
        Ok(playlists)
    }

    pub fn delete_playlist(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn get_playlist_tracks(&self, playlist_id: i64) -> Result<Vec<UnifiedTrack>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                COALESCE(t.title, 'Unknown'),
                COALESCE(t.artist, 'Unknown'),
                COALESCE(t.album, ''),
                t.duration_ms,
                t.path,
                pt.external_url,
                pt.source,
                t.id
            FROM playlist_tracks pt
            LEFT JOIN tracks t ON pt.track_id = t.id
            WHERE pt.playlist_id = ?1
            ORDER BY pt.position
            "#,
        )?;
        let rows = stmt.query_map([playlist_id], |row| {
            let source_str: String = row.get(6)?;
            let source = match source_str.as_str() {
                "local" => TrackSource::Local,
                "youtube" => TrackSource::YouTube,
                "soundcloud" => TrackSource::SoundCloud,
                "spotify" => TrackSource::Spotify,
                _ => TrackSource::Local,
            };
            // Prefer the index row's path, but fall back to the URL stored on
            // the playlist row itself: a dangling index reference (e.g. the
            // file was deleted and a rescan NULLed track_id) must not make
            // the whole playlist fail to load.
            let indexed_path: Option<String> = row.get(4)?;
            let stored_url = row.get::<_, Option<String>>(5)?.unwrap_or_default();
            let playable_url = match source {
                TrackSource::Local => indexed_path.or({
                    if stored_url.is_empty() {
                        None
                    } else {
                        Some(stored_url)
                    }
                }),
                _ => Some(stored_url),
            };
            // Neither the index nor the playlist row can play this entry —
            // skip it rather than failing the entire playlist.
            let Some(playable_url) = playable_url else {
                return Ok(None);
            };
            let id = if source == TrackSource::Local {
                playable_url.clone()
            } else {
                row.get::<_, Option<i64>>(7)?
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| playable_url.clone())
            };
            Ok(Some(UnifiedTrack {
                id,
                title: row.get(0)?,
                artist: row.get(1)?,
                album: row.get(2)?,
                duration_ms: row.get::<_, Option<u64>>(3)?,
                source,
                playable_url,
                thumbnail_url: None,
            }))
        })?;
        let mut tracks = Vec::new();
        for row in rows {
            if let Some(track) = row? {
                tracks.push(track);
            }
        }
        Ok(tracks)
    }

    pub fn add_unified_track_to_playlist(
        &self,
        playlist_id: i64,
        track: &UnifiedTrack,
    ) -> Result<()> {
        let track_id: Option<i64> = if track.source == TrackSource::Local {
            self.conn
                .query_row(
                    "SELECT id FROM tracks WHERE path = ?1",
                    [&track.playable_url],
                    |row| row.get(0),
                )
                .optional()?
        } else {
            None
        };
        // external_url doubles as a self-sufficient fallback for ALL sources,
        // local included: if the index row ever goes away (file deleted,
        // rescan), the playlist entry can still play from its own stored path.
        self.add_track_to_playlist(
            playlist_id,
            track_id,
            Some(&track.playable_url),
            track.source.as_str(),
        )
    }

    pub fn save_track(&self, track: &UnifiedTrack) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT OR IGNORE INTO saved_tracks
                (title, artist, album, duration_ms, playable_url, source, thumbnail_url, external_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            (
                &track.title,
                &track.artist,
                &track.album,
                track.duration_ms,
                &track.playable_url,
                track.source.as_str(),
                track.thumbnail_url.as_ref(),
                Some(&track.id),
            ),
        )?;
        Ok(())
    }

    pub fn list_saved_tracks(&self) -> Result<Vec<UnifiedTrack>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT title, artist, album, duration_ms, playable_url, source, thumbnail_url, external_id
            FROM saved_tracks
            ORDER BY id
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            let source_str: String = row.get(5)?;
            let source = match source_str.as_str() {
                "local" => TrackSource::Local,
                "youtube" => TrackSource::YouTube,
                "soundcloud" => TrackSource::SoundCloud,
                "spotify" => TrackSource::Spotify,
                _ => TrackSource::Local,
            };
            let playable_url: String = row.get(4)?;
            let id = row
                .get::<_, Option<String>>(7)?
                .unwrap_or_else(|| playable_url.clone());
            Ok(UnifiedTrack {
                id,
                title: row.get(0)?,
                artist: row.get(1)?,
                album: row.get(2)?,
                duration_ms: row.get::<_, Option<u64>>(3)?,
                source,
                playable_url,
                thumbnail_url: row.get(6)?,
            })
        })?;
        let mut tracks = Vec::new();
        for row in rows {
            tracks.push(row?);
        }
        Ok(tracks)
    }

    pub fn delete_saved_track(&self, playable_url: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM saved_tracks WHERE playable_url = ?1", [playable_url])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::local::LocalTrack;

    /// A fresh database in a unique temp file per test.
    fn temp_db(name: &str) -> (Database, std::path::PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "climusic-dbtest-{}-{}-{nanos}.db",
            name,
            std::process::id()
        ));
        let db = Database::open(&path).expect("open temp db");
        (db, path)
    }

    fn local_track(path: &str) -> LocalTrack {
        LocalTrack {
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration_ms: Some(1000),
            path: path.to_string(),
        }
    }

    /// The critical audit bug: a rescan used to DELETE all local rows and
    /// re-INSERT them under new AUTOINCREMENT ids, orphaning every playlist
    /// reference and making the whole playlist fail to load.
    #[test]
    fn rescan_keeps_playlist_references() {
        let (mut db, path) = temp_db("rescan");
        let playlist = db.create_playlist("mix").unwrap();
        db.sync_local_tracks(&[local_track("C:/music/a.mp3")]).unwrap();
        db.add_unified_track_to_playlist(playlist, &local_track("C:/music/a.mp3").to_unified())
            .unwrap();

        // Rescan: same file still present, plus a new one.
        db.sync_local_tracks(&[local_track("C:/music/a.mp3"), local_track("C:/music/b.mp3")])
            .unwrap();

        let tracks = db.get_playlist_tracks(playlist).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].playable_url, "C:/music/a.mp3");
        assert_eq!(tracks[0].title, "Song");
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    /// A file deleted from disk drops out of the index on the next sync; the
    /// playlist row must survive (FK SET NULL) and still play from its own
    /// stored URL.
    #[test]
    fn playlist_row_survives_track_leaving_index() {
        let (mut db, path) = temp_db("dangling");
        let playlist = db.create_playlist("mix").unwrap();
        db.sync_local_tracks(&[local_track("C:/music/a.mp3")]).unwrap();
        db.add_unified_track_to_playlist(playlist, &local_track("C:/music/a.mp3").to_unified())
            .unwrap();

        // File vanished from disk: empty sync removes it from the index.
        db.sync_local_tracks(&[]).unwrap();

        let tracks = db.get_playlist_tracks(playlist).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].playable_url, "C:/music/a.mp3");
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    /// LIKE metacharacters in the query must match literally — a bare "%"
    /// used to act as a wildcard and return the entire library.
    #[test]
    fn search_escapes_like_wildcards() {
        let (mut db, path) = temp_db("like");
        let mut percent_track = local_track("C:/music/percent.mp3");
        percent_track.title = "100%".to_string();
        db.sync_local_tracks(&[local_track("C:/music/a.mp3"), percent_track])
            .unwrap();

        let hits = db.search_local("%", 20).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "100%");

        // "_" is a single-char wildcard in LIKE; here it must be literal.
        assert_eq!(db.search_local("_", 20).unwrap().len(), 0);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    /// With foreign keys enabled, deleting a playlist cascades to its rows —
    /// previously they were orphaned in playlist_tracks forever.
    #[test]
    fn delete_playlist_removes_its_rows() {
        let (mut db, path) = temp_db("cascade");
        let playlist = db.create_playlist("gone").unwrap();
        db.sync_local_tracks(&[local_track("C:/music/x.mp3")]).unwrap();
        db.add_unified_track_to_playlist(playlist, &local_track("C:/music/x.mp3").to_unified())
            .unwrap();

        db.delete_playlist(playlist).unwrap();

        let remaining: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM playlist_tracks WHERE playlist_id = ?1",
                [playlist],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn saved_tracks_round_trip() {
        let (db, path) = temp_db("saved");
        let track = UnifiedTrack {
            id: "sc:123".to_string(),
            title: "Liked Song".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration_ms: Some(123_000),
            source: TrackSource::SoundCloud,
            playable_url: "https://soundcloud.com/artist/track".to_string(),
            thumbnail_url: Some("https://img/small.jpg".to_string()),
        };

        db.save_track(&track).unwrap();
        let saved = db.list_saved_tracks().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, "sc:123");
        assert_eq!(saved[0].title, "Liked Song");
        assert_eq!(saved[0].source, TrackSource::SoundCloud);

        // Saving the same track again must be a quiet no-op.
        db.save_track(&track).unwrap();
        assert_eq!(db.list_saved_tracks().unwrap().len(), 1);

        db.delete_saved_track(&track.playable_url).unwrap();
        assert!(db.list_saved_tracks().unwrap().is_empty());

        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
