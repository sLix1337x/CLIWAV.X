use crate::error::{ClimusicError, Result};
use crate::sources::{TrackSource, UnifiedTrack};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct SpotifySource {
    client_id: String,
    client_secret: String,
    token: Option<SpotifyToken>,
    /// One shared client (15s timeout) instead of a fresh one per call —
    /// new TLS state and no connection reuse otherwise.
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
struct SpotifyToken {
    access_token: String,
    expires_at: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    tracks: TracksPage,
}

#[derive(Debug, Deserialize)]
struct TracksPage {
    items: Vec<SpotifyTrack>,
}

#[derive(Debug, Deserialize)]
struct SpotifyTrack {
    id: String,
    name: String,
    artists: Vec<SpotifyArtist>,
    album: SpotifyAlbum,
    duration_ms: u64,
}

#[derive(Debug, Deserialize)]
struct SpotifyArtist {
    name: String,
}

#[derive(Debug, Deserialize)]
struct SpotifyAlbum {
    name: String,
    images: Vec<SpotifyImage>,
}

#[derive(Debug, Deserialize)]
struct SpotifyImage {
    url: String,
}

impl SpotifySource {
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            token: None,
            client: crate::sources::http_client(std::time::Duration::from_secs(15)),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.client_id.is_empty() && !self.client_secret.is_empty()
    }

    pub async fn search(&mut self, query: &str, limit: usize) -> Result<Vec<UnifiedTrack>> {
        if !self.is_configured() {
            return Ok(Vec::new());
        }
        self.ensure_token().await?;

        let token = self
            .token
            .as_ref()
            .map(|t| t.access_token.clone())
            .ok_or_else(|| ClimusicError::SpotifyApi("missing token".into()))?;

        let resp = self
            .client
            .get("https://api.spotify.com/v1/search")
            .query(&[
                ("q", query),
                ("type", "track"),
                ("limit", &limit.to_string()),
            ])
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClimusicError::SpotifyApi(format!("search failed: {text}")));
        }

        let data: SearchResponse = resp.json().await?;
        Ok(data.tracks.items.into_iter().map(into_unified).collect())
    }

    /// Fetch a playlist's name + first page of tracks (up to 100) by its id.
    /// `truncated` in the result flags that more pages exist.
    pub async fn playlist(&mut self, playlist_id: &str) -> Result<(String, Vec<UnifiedTrack>, bool)> {
        if !self.is_configured() {
            return Err(ClimusicError::SpotifyApi(
                "Spotify credentials not set (press S)".into(),
            ));
        }
        self.ensure_token().await?;

        let token = self
            .token
            .as_ref()
            .map(|t| t.access_token.clone())
            .ok_or_else(|| ClimusicError::SpotifyApi("missing token".into()))?;

        let resp = self
            .client
            .get(format!("https://api.spotify.com/v1/playlists/{playlist_id}"))
            .query(&[(
                "fields",
                "name,tracks.next,tracks.items(track(name,artists(name),album(name,images),duration_ms,id))",
            )])
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClimusicError::SpotifyApi(format!("playlist fetch failed: {text}")));
        }

        let data: PlaylistResponse = resp.json().await?;
        let truncated = data.tracks.next.is_some();
        let tracks = data
            .tracks
            .items
            .into_iter()
            .filter_map(|item| item.track)
            .map(into_unified)
            .collect();
        Ok((data.name, tracks, truncated))
    }

    async fn ensure_token(&mut self) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if let Some(token) = &self.token {
            if token.expires_at > now + 60 {
                return Ok(());
            }
        }

        let credentials = format!("{}:{}", self.client_id, self.client_secret);
        let encoded = STANDARD.encode(credentials);

        let resp = self
            .client
            .post("https://accounts.spotify.com/api/token")
            .header(AUTHORIZATION, format!("Basic {encoded}"))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body("grant_type=client_credentials")
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClimusicError::SpotifyApi(format!("token failed: {text}")));
        }

        let token_resp: TokenResponse = resp.json().await?;
        self.token = Some(SpotifyToken {
            access_token: token_resp.access_token,
            expires_at: now + token_resp.expires_in,
        });
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct PlaylistResponse {
    name: String,
    tracks: PlaylistTracksPage,
}

#[derive(Debug, Deserialize)]
struct PlaylistTracksPage {
    items: Vec<PlaylistItem>,
    /// Non-null when the playlist has more than one page of tracks.
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PlaylistItem {
    /// Null for local files / episodes in the playlist — skipped.
    track: Option<SpotifyTrack>,
}

fn into_unified(track: SpotifyTrack) -> UnifiedTrack {
    let artist = track
        .artists
        .into_iter()
        .map(|a| a.name)
        .collect::<Vec<_>>()
        .join(", ");
    let thumbnail_url = track.album.images.first().map(|img| img.url.clone());

    UnifiedTrack {
        id: track.id.clone(),
        title: track.name,
        artist,
        album: track.album.name,
        duration_ms: Some(track.duration_ms),
        source: TrackSource::Spotify,
        playable_url: format!("spotify:track:{}", track.id),
        thumbnail_url,
    }
}
