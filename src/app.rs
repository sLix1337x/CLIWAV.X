use crate::config::Config;
use crate::db::{Database, Playlist};
use crate::error::{ClimusicError, Result};
use crate::player::eq;
use crate::player::mpv::MpvPlayer;
use crate::sources::local::{expand_path, scan_directory};
use crate::sources::soundcloud::SoundCloudSource;
use crate::sources::spotify::SpotifySource;
use crate::sources::youtube::YouTubeSource;
use crate::sources::{TrackSource, UnifiedTrack};
use futures::FutureExt;
use rand::Rng;
use ratatui_image::picker::Picker;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

/// How many entries to fetch per page when browsing a SoundCloud user's
/// Tracks/Likes/Reposts — collections like Likes can run into the
/// thousands, so they're paginated instead of fetched in one blocking call.
const SOUNDCLOUD_PAGE_SIZE: usize = 100;

/// How long a loaded SoundCloud list (user category or genre bucket) is
/// reused before re-opening it re-runs yt-dlp.
const SOUNDCLOUD_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// How far Shift+Left/Right rewinds/fast-forwards the current track.
const SEEK_STEP_SECS: f64 = 5.0;

/// Dashboard SoundCloud-pane selector slots: Search, then the three real
/// `SoundCloudCategory::ALL` categories (slot index - 1), then a Library
/// quick-access slot. Kept as plain indices (rather than folded into
/// `SoundCloudCategory`) because Search/Library aren't browsable categories
/// — they're pane actions — and `SoundCloudCategory::ALL` is shared with the
/// standalone SoundCloud tab, which has no Search/Library slots of its own.
pub const DASHBOARD_SC_SEARCH_SLOT: usize = 0;
pub const DASHBOARD_SC_LIBRARY_SLOT: usize = 4;
const DASHBOARD_SC_SLOT_COUNT: usize = 5;

/// Genre buckets shown when no SoundCloud username is configured — each is
/// just a live `scsearch:` query, so the SoundCloud pane and Dashboard are
/// useful before any setup. SoundCloud's real chart endpoints 404 through
/// yt-dlp, so these are searches, not charts.
pub const SOUNDCLOUD_GENRES: [&str; 7] = [
    "Trending",
    "Hip-Hop",
    "Electronic",
    "House",
    "Lo-Fi",
    "Indie",
    "Pop",
];

/// Synthetic playlist id injected at the front of the Library playlist list
/// so "Saved Tracks" persists across sessions without being a real playlist.
pub const SAVED_TRACKS_PLAYLIST_ID: i64 = -1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Search,
    Queue,
    Library,
    SoundCloud,
    NowPlaying,
    Eq,
}

/// Which pane of the Dashboard tab has keyboard focus (the Now Playing pane
/// is display-only and never focused).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardPane {
    SoundCloud,
    Queue,
}

/// Whether the Search tab is capturing keystrokes as query text, or letting
/// the usual single-key shortcuts navigate/act on the results list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFocus {
    Input,
    Results,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundCloudPane {
    Categories,
    Tracks,
}

/// Which browsed list a directly-played (non-queue) track came from, so that
/// when it finishes, playback can continue with the next entry in that same
/// list — e.g. auto-advancing through a SoundCloud Likes/Reposts page — the
/// same way it already does for the explicit queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackOrigin {
    Search,
    Playlist,
    SoundCloud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundCloudCategory {
    Tracks,
    Likes,
    Reposts,
}

impl SoundCloudCategory {
    pub const ALL: [SoundCloudCategory; 3] = [
        SoundCloudCategory::Tracks,
        SoundCloudCategory::Likes,
        SoundCloudCategory::Reposts,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            SoundCloudCategory::Tracks => "Tracks",
            SoundCloudCategory::Likes => "Likes",
            SoundCloudCategory::Reposts => "Reposts",
        }
    }

    fn url_suffix(&self) -> &'static str {
        match self {
            SoundCloudCategory::Tracks => "tracks",
            SoundCloudCategory::Likes => "likes",
            SoundCloudCategory::Reposts => "reposts",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    Off,
    Track,
    Queue,
}

impl LoopMode {
    pub fn label(&self) -> &'static str {
        match self {
            LoopMode::Off => "Off",
            LoopMode::Track => "Track",
            LoopMode::Queue => "All",
        }
    }
}

/// Where the UI's accent color comes from: `Auto` follows the current track's
/// artwork (with a neutral default while none is loaded); the rest are
/// curated fixed palettes — a predictable, always-good-looking override for
/// monochrome/washed-out covers whose extracted accent comes out muddy.
/// Cycled with `t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccentPalette {
    Auto,
    Teal,
    Magenta,
    Amber,
    Violet,
}

impl AccentPalette {
    pub fn label(&self) -> &'static str {
        match self {
            AccentPalette::Auto => "auto",
            AccentPalette::Teal => "teal",
            AccentPalette::Magenta => "magenta",
            AccentPalette::Amber => "amber",
            AccentPalette::Violet => "violet",
        }
    }

    /// Fixed RGB for the curated palettes; `None` means "derive from artwork".
    pub fn rgb(&self) -> Option<(u8, u8, u8)> {
        match self {
            AccentPalette::Auto => None,
            AccentPalette::Teal => Some((45, 212, 191)),
            AccentPalette::Magenta => Some((232, 121, 249)),
            AccentPalette::Amber => Some((251, 191, 36)),
            AccentPalette::Violet => Some((167, 139, 250)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryPane {
    Playlists,
    Tracks,
}

pub struct App {
    pub config: Config,
    pub db: Database,
    pub player: MpvPlayer,
    pub youtube: YouTubeSource,
    pub soundcloud: SoundCloudSource,
    pub spotify: SpotifySource,

    pub current_tab: Tab,
    pub dashboard_pane: DashboardPane,
    pub search_query: String,
    pub search_results: Vec<UnifiedTrack>,
    pub search_selected: usize,
    pub search_source_filter: Option<TrackSource>,
    pub search_focus: SearchFocus,
    pub search_loading: bool,
    search_handle: Option<tokio::task::JoinHandle<(Vec<UnifiedTrack>, Option<String>, Option<String>)>>,

    pub dashboard_sc_category_selected: usize,
    pub dashboard_sc_search: bool,
    pub dashboard_sc_query: String,
    pub dashboard_sc_results: Vec<UnifiedTrack>,
    pub dashboard_sc_selected: usize,
    pub dashboard_sc_search_focus: SearchFocus,
    pub dashboard_sc_search_loading: bool,
    dashboard_sc_search_handle:
        Option<tokio::task::JoinHandle<(Vec<UnifiedTrack>, Option<String>, Option<String>)>>,

    pub soundcloud_username: String,
    pub soundcloud_pane: SoundCloudPane,
    pub soundcloud_category_selected: usize,
    pub soundcloud_user_tracks: Vec<UnifiedTrack>,
    pub soundcloud_track_selected: usize,
    pub soundcloud_loading: bool,
    pub soundcloud_has_more: bool,
    /// In-flight page load, keyed by the username and category it was
    /// started for, so a stale page (user/category changed mid-load) is
    /// discarded instead of merged into the new list. The joinhandle's
    /// usize is the RAW entry count yt-dlp returned (see user_category).
    soundcloud_load_handle: Option<(String, SoundCloudCategory, tokio::task::JoinHandle<Result<(Vec<UnifiedTrack>, usize)>>)>,
    /// Next 1-indexed page position to request — tracked explicitly rather
    /// than derived from the loaded-track count, which drifts when entries
    /// fail to parse.
    soundcloud_next_start: usize,
    /// Selected genre bucket (browse mode when no username is configured)
    /// and its in-flight load, keyed by genre label for staleness checks.
    pub soundcloud_genre_selected: usize,
    soundcloud_genre_handle: Option<(String, tokio::task::JoinHandle<Result<Vec<UnifiedTrack>>>)>,
    /// Loaded SoundCloud lists, keyed by (username, category) — or ("",
    /// genre) for genre buckets. Re-opening a cached list within the TTL
    /// skips the yt-dlp round-trip entirely. Value: (tracks, has_more,
    /// next_start, fetched_at).
    soundcloud_list_cache: HashMap<(String, String), (Vec<UnifiedTrack>, bool, usize, Instant)>,
    /// Set when auto-advance ran off the end of an already-loaded SoundCloud
    /// page while that category still has more to fetch: `poll_soundcloud_load`
    /// resumes playback with the freshly-loaded track once the page lands.
    soundcloud_autoplay_pending: bool,

    /// Set when an auto-advance attempt fails (dead link, transient yt-dlp
    /// error): stops `poll_playback` from retrying every tick. Cleared by
    /// any successful play, so a manual `n`/`Enter` re-arms autoplay.
    autoplay_failed: bool,

    /// Which list the currently-playing track was played *from* (if not the
    /// queue), and its index there — lets auto-advance continue through that
    /// list instead of just stopping. `None` while playing from the queue.
    playback_origin: Option<(PlaybackOrigin, usize)>,

    pub queue: VecDeque<UnifiedTrack>,
    /// Set whenever the queue changes; persisted (debounced via the tick
    /// loop) so the queue survives restarts.
    queue_dirty: bool,
    pub queue_selected: usize,
    pub current_track: Option<UnifiedTrack>,
    pub is_playing: bool,
    pub volume: u8,
    /// Last-polled playback position/duration in seconds (drives the Now
    /// Playing progress bar). 0 when nothing is loaded.
    pub playback_pos: f64,
    pub playback_dur: f64,
    pub game_mode: bool,
    pub loop_mode: LoopMode,
    pub shuffle: bool,
    pub palette: AccentPalette,
    /// Incremented once per UI tick; drives the animated loading spinner.
    pub tick: u64,

    pub eq_gains: eq::Gains,
    pub eq_selected: usize,
    /// Name of the matching built-in preset, or "Custom" after a manual
    /// band edit that no longer matches any preset exactly.
    pub eq_preset: String,
    /// Set on every band/preset edit; `poll_eq` debounces off this so a
    /// burst of Up/Down presses sends one `af set` to mpv, not one per key
    /// (each replace risks a small audible blip — see `MpvPlayer::set_eq`).
    eq_last_edit: Option<Instant>,
    eq_applied_gains: eq::Gains,
    eq_saved_gains: eq::Gains,

    /// Off by default — WASAPI loopback capture plus a continuous FFT is a
    /// real, constant background CPU cost that shouldn't run unless someone
    /// actually wants to see it. See `toggle_visualizer`.
    pub visualizer_on: bool,
    pub visualizer_bands: [f32; crate::audio::spectrum::BAND_COUNT],
    visualizer_spectrum: crate::audio::spectrum::Spectrum,
    #[cfg(windows)]
    audio_capture: Option<crate::audio::capture::AudioCapture>,

    pub playlists: Vec<Playlist>,
    pub selected_playlist: usize,
    pub playlist_tracks: Vec<UnifiedTrack>,
    pub selected_playlist_track: usize,
    pub library_pane: LibraryPane,
    /// Single-slot undo for the one explicitly destructive action — deleting
    /// a playlist. Captured at delete time, restored with Ctrl+Z.
    last_deleted_playlist: Option<(String, Vec<UnifiedTrack>)>,

    pub status_message: String,
    pub should_quit: bool,
    pub input_prompt: Option<InputPrompt>,
    pub show_help: bool,

    /// Terminal image picker, detected once at startup (falls back to
    /// Unicode halfblocks if the terminal doesn't answer the graphics query).
    /// `pub` so the UI layer can encode with it; rendering only needs `&App`
    /// because the encoded-protocol cache lives behind a `RefCell` below.
    pub picker: Picker,
    pub artwork: Option<image::DynamicImage>,
    /// (area it was last encoded for, encoded protocol). Interior-mutable so
    /// `ui::player` can re-encode on area-size change from an `&App` render
    /// pass, matching how ratatui-image apps typically cache this (e.g. Myx).
    pub artwork_cache: std::cell::RefCell<Option<(ratatui::layout::Rect, ratatui_image::protocol::Protocol)>>,
    /// Cover art color, extracted from the current track's artwork, used to
    /// accent the Now Playing panel. None while no track/artwork is loaded.
    pub artwork_accent: Option<(u8, u8, u8)>,
    artwork_key: Option<String>,
    artwork_handle: Option<tokio::task::JoinHandle<Option<image::DynamicImage>>>,

    /// Bucketed, normalized amplitude data for the current track's Now
    /// Playing waveform. `None` while unloaded/uncached/unsupported — the UI
    /// falls back to the plain progress bar in that case.
    pub waveform: Option<Vec<f32>>,
    waveform_key: Option<String>,
    waveform_handle: Option<tokio::task::JoinHandle<Option<Vec<f32>>>>,
    /// Session-lifetime cache keyed the same way as `waveform_key`, so
    /// replaying a track doesn't re-fetch/re-decode it.
    waveform_cache: HashMap<String, Vec<f32>>,

    scan_handle: Option<tokio::task::JoinHandle<Result<()>>>,
}

#[derive(Debug, Clone)]
pub struct InputPrompt {
    pub title: String,
    pub value: String,
    pub kind: PromptKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    NewPlaylist,
    SpotifyClientId,
    SpotifyClientSecret { client_id: String },
    SoundCloudUsername,
}

/// `Picker::from_query_stdio()` writes an escape sequence and blocks for up
/// to 1s waiting for the terminal to answer, before falling back to
/// `from_fontsize` regardless. On Windows consoles that reply almost never
/// arrives — cmd.exe, PowerShell, and plain Windows Terminal don't answer
/// these queries — so every launch paid that full 1s just to land on the
/// same fallback it would have used anyway. Skipping the query there and
/// going straight to the fallback removes that delay with no change in
/// outcome. Non-Windows terminals (Kitty, iTerm2, WezTerm, ...) reply
/// quickly and detect real capabilities, so they keep querying.
fn detect_picker() -> Picker {
    #[cfg(windows)]
    {
        Picker::from_fontsize((8, 16))
    }
    #[cfg(not(windows))]
    {
        Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((8, 16)))
    }
}

impl App {
    pub async fn new() -> Result<Self> {
        let config = Config::load()?;
        let db = Database::open(Config::db_path()?)?;
        let player = MpvPlayer::new(&config.player.mpv_path, config.player.audio_exclusive);
        let youtube = YouTubeSource::new(
            &config.player.yt_dlp_path,
            &config.player.cookies_from_browser,
        );
        let soundcloud = SoundCloudSource::new(
            &config.player.yt_dlp_path,
            &config.player.cookies_from_browser,
        );
        let spotify = SpotifySource::new(
            &config.spotify.client_id,
            &config.spotify.client_secret,
        );

        let scan_paths = config.local.paths.clone();
        let scan_db_path = Config::db_path()?;
        let scan_handle = tokio::task::spawn_blocking(move || {
            scan_library(&scan_paths, scan_db_path)
        });

        let soundcloud_username = config.soundcloud.username.clone();

        let mut app = Self {
            config,
            db,
            player,
            youtube,
            soundcloud,
            spotify,
            current_tab: Tab::Dashboard,
            dashboard_pane: DashboardPane::SoundCloud,
            search_query: String::new(),
            search_results: Vec::new(),
            search_selected: 0,
            search_source_filter: None,
            search_focus: SearchFocus::Input,
            search_loading: false,
            search_handle: None,
            // Starts on the Tracks slot (index 1: Search, Tracks, Likes,
            // Reposts, Library) so first launch behaves like before this
            // selector grew Search/Library entries.
            dashboard_sc_category_selected: 1,
            dashboard_sc_search: false,
            dashboard_sc_query: String::new(),
            dashboard_sc_results: Vec::new(),
            dashboard_sc_selected: 0,
            dashboard_sc_search_focus: SearchFocus::Input,
            dashboard_sc_search_loading: false,
            dashboard_sc_search_handle: None,
            soundcloud_username,
            soundcloud_pane: SoundCloudPane::Categories,
            soundcloud_category_selected: 0,
            soundcloud_user_tracks: Vec::new(),
            soundcloud_track_selected: 0,
            soundcloud_loading: false,
            soundcloud_has_more: true,
            soundcloud_load_handle: None,
            soundcloud_next_start: 1,
            soundcloud_genre_selected: 0,
            soundcloud_genre_handle: None,
            soundcloud_list_cache: HashMap::new(),
            soundcloud_autoplay_pending: false,
            autoplay_failed: false,
            playback_origin: None,
            queue: VecDeque::new(),
            queue_dirty: false,
            queue_selected: 0,
            current_track: None,
            is_playing: false,
            volume: 80,
            playback_pos: 0.0,
            playback_dur: 0.0,
            game_mode: false,
            loop_mode: LoopMode::Off,
            shuffle: false,
            palette: AccentPalette::Auto,
            tick: 0,
            eq_gains: eq::FLAT,
            eq_selected: 0,
            eq_preset: "Flat".to_string(),
            eq_last_edit: None,
            eq_applied_gains: eq::FLAT,
            eq_saved_gains: eq::FLAT,
            visualizer_on: false,
            visualizer_bands: [0.0; crate::audio::spectrum::BAND_COUNT],
            visualizer_spectrum: crate::audio::spectrum::Spectrum::new(),
            #[cfg(windows)]
            audio_capture: None,
            playlists: Vec::new(),
            selected_playlist: 0,
            playlist_tracks: Vec::new(),
            selected_playlist_track: 0,
            library_pane: LibraryPane::Playlists,
            last_deleted_playlist: None,
            status_message: "Scanning local library in background...".to_string(),
            should_quit: false,
            input_prompt: None,
            show_help: false,
            picker: detect_picker(),
            artwork: None,
            artwork_cache: RefCell::new(None),
            artwork_accent: None,
            artwork_key: None,
            artwork_handle: None,
            waveform: None,
            waveform_key: None,
            waveform_handle: None,
            waveform_cache: HashMap::new(),
            scan_handle: Some(scan_handle),
        };

        app.player.start().await?;
        app.player.set_volume(app.volume).await?;

        let eq_gains = eq::gains_from_slice(&app.config.eq.gains);
        // Flat is mpv's default (empty `af` chain) — skip sending a no-op
        // filter chain on every startup.
        if eq_gains != eq::FLAT {
            app.player.set_eq(eq_gains).await?;
        }
        app.eq_gains = eq_gains;
        app.eq_applied_gains = eq_gains;
        app.eq_saved_gains = eq_gains;
        app.eq_preset = eq::matching_preset(&eq_gains)
            .unwrap_or("Custom")
            .to_string();

        app.load_playlists()?;
        app.load_queue();

        Ok(app)
    }

    /// Restore the persisted queue (survives restarts). Missing/corrupt file
    /// just means an empty queue.
    fn load_queue(&mut self) {
        let Ok(path) = Config::queue_path() else {
            return;
        };
        let Ok(json) = std::fs::read_to_string(path) else {
            return;
        };
        if let Ok(queue) = serde_json::from_str(&json) {
            self.queue = queue;
        }
    }

    /// Persist the queue if it changed since the last save (called from the
    /// tick loop, so bursts of mutations coalesce into one write).
    pub fn maybe_save_queue(&mut self) {
        if !self.queue_dirty {
            return;
        }
        self.queue_dirty = false;
        let Ok(path) = Config::queue_path() else {
            return;
        };
        if let Ok(json) = serde_json::to_string(&self.queue) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn load_playlists(&mut self) -> Result<()> {
        let mut playlists = self.db.list_playlists()?;
        playlists.insert(
            0,
            Playlist {
                id: SAVED_TRACKS_PLAYLIST_ID,
                name: "Saved Tracks".to_string(),
            },
        );
        self.playlists = playlists;
        if self.selected_playlist >= self.playlists.len() && !self.playlists.is_empty() {
            self.selected_playlist = self.playlists.len() - 1;
        }
        self.load_selected_playlist_tracks()?;
        Ok(())
    }

    pub fn load_selected_playlist_tracks(&mut self) -> Result<()> {
        self.playlist_tracks.clear();
        self.selected_playlist_track = 0;
        if let Some(playlist) = self.playlists.get(self.selected_playlist) {
            if playlist.id == SAVED_TRACKS_PLAYLIST_ID {
                self.playlist_tracks = self.db.list_saved_tracks()?;
            } else {
                self.playlist_tracks = self.db.get_playlist_tracks(playlist.id)?;
            }
        }
        Ok(())
    }

    pub fn create_playlist(&mut self, name: &str) -> Result<()> {
        if name.trim().is_empty() {
            self.status_message = "Playlist name cannot be empty.".to_string();
            return Ok(());
        }
        if name.trim().eq_ignore_ascii_case("Saved Tracks") {
            self.status_message = "'Saved Tracks' is reserved.".to_string();
            return Ok(());
        }
        self.db.create_playlist(name.trim())?;
        self.load_playlists()?;
        self.status_message = format!("Created playlist '{}'.", name.trim());
        Ok(())
    }

    pub fn delete_selected_playlist(&mut self) -> Result<()> {
        if let Some(playlist) = self.playlists.get(self.selected_playlist).cloned() {
            if playlist.id == SAVED_TRACKS_PLAYLIST_ID {
                return Ok(());
            }
            // Capture for Ctrl+Z before destroying anything.
            let tracks = self.db.get_playlist_tracks(playlist.id).unwrap_or_default();
            self.last_deleted_playlist = Some((playlist.name.clone(), tracks));
            self.db.delete_playlist(playlist.id)?;
            self.load_playlists()?;
            self.status_message =
                format!("Deleted playlist '{}' (Ctrl+Z to undo).", playlist.name);
        }
        Ok(())
    }

    pub fn selected_playlist_is_saved_tracks(&self) -> bool {
        self.playlists
            .get(self.selected_playlist)
            .is_some_and(|p| p.id == SAVED_TRACKS_PLAYLIST_ID)
    }

    /// Return the currently-selected track from whichever tab/pane has focus.
    /// Used by global shortcuts like 's' so they act on the visible selection
    /// without every call site duplicating the focus logic.
    pub fn selected_track(&self) -> Option<&UnifiedTrack> {
        match self.current_tab {
            Tab::Dashboard => match self.dashboard_pane {
                DashboardPane::SoundCloud if self.dashboard_sc_search => {
                    self.dashboard_sc_results.get(self.dashboard_sc_selected)
                }
                DashboardPane::SoundCloud => {
                    self.soundcloud_user_tracks.get(self.soundcloud_track_selected)
                }
                DashboardPane::Queue => self.queue.get(self.queue_selected),
            },
            Tab::Search => self.search_results.get(self.search_selected),
            Tab::Queue => self.queue.get(self.queue_selected),
            Tab::Library if self.library_pane == LibraryPane::Tracks => {
                self.playlist_tracks.get(self.selected_playlist_track)
            }
            Tab::SoundCloud if self.soundcloud_pane == SoundCloudPane::Tracks => {
                self.soundcloud_user_tracks.get(self.soundcloud_track_selected)
            }
            _ => None,
        }
    }

    pub fn save_selected_track_to_library(&mut self) -> Result<()> {
        let Some(track) = self.selected_track().cloned() else {
            self.status_message = "Nothing selected to save.".to_string();
            return Ok(());
        };
        self.db.save_track(&track)?;
        if self.current_tab == Tab::Library
            && self.library_pane == LibraryPane::Tracks
            && self.selected_playlist_is_saved_tracks()
        {
            self.load_selected_playlist_tracks()?;
        }
        self.status_message = format!("Saved '{}' to library.", track.title);
        Ok(())
    }

    pub fn remove_selected_saved_track(&mut self) -> Result<()> {
        if self.current_tab != Tab::Library
            || self.library_pane != LibraryPane::Tracks
            || !self.selected_playlist_is_saved_tracks()
        {
            return Ok(());
        }
        let Some(track) = self.selected_track().cloned() else {
            return Ok(());
        };
        self.db.delete_saved_track(&track.playable_url)?;
        self.load_selected_playlist_tracks()?;
        self.status_message = format!("Removed '{}' from saved tracks.", track.title);
        Ok(())
    }

    /// Restore the most recently deleted playlist (single-slot undo).
    pub fn undo_delete_playlist(&mut self) -> Result<()> {
        let Some((name, tracks)) = self.last_deleted_playlist.take() else {
            self.status_message = "Nothing to undo.".to_string();
            return Ok(());
        };
        let id = self.db.create_playlist(&name)?;
        for track in &tracks {
            self.db.add_unified_track_to_playlist(id, track)?;
        }
        self.load_playlists()?;
        if let Some(idx) = self.playlists.iter().position(|p| p.id == id) {
            self.selected_playlist = idx;
            let _ = self.load_selected_playlist_tracks();
        }
        self.status_message = format!("Restored playlist '{}' ({} tracks).", name, tracks.len());
        Ok(())
    }

    pub fn add_selected_track_to_playlist(&mut self, playlist_index: usize) -> Result<()> {
        if let Some(playlist) = self.playlists.get(playlist_index).cloned() {
            if let Tab::Search = self.current_tab {
                if let Some(track) = self.search_results.get(self.search_selected).cloned() {
                    self.db.add_unified_track_to_playlist(playlist.id, &track)?;
                    self.status_message =
                        format!("Added '{}' to '{}'.", track.title, playlist.name);
                    if self.selected_playlist == playlist_index {
                        self.load_selected_playlist_tracks()?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Poll the background library scan and update the status message when done.
    pub fn poll_scan(&mut self) {
        if let Some(handle) = &self.scan_handle {
            if handle.is_finished() {
                let handle = self.scan_handle.take().unwrap();
                match handle.now_or_never().unwrap_or(Ok(Ok(()))) {
                    Ok(Ok(())) => self.status_message = "Local library scan complete.".to_string(),
                    Ok(Err(e)) => self.status_message = format!("Library scan failed: {e}"),
                    Err(_) => self.status_message = "Library scan panicked.".to_string(),
                }
            }
        }
    }

    pub async fn rescan_local_library(&mut self) -> Result<()> {
        if self.scan_in_progress() {
            // Replacing the handle would leave the old scan running detached:
            // two concurrent clear/insert cycles against the same SQLite file.
            self.status_message = "Library scan already in progress.".to_string();
            return Ok(());
        }
        let paths = self.config.local.paths.clone();
        let db_path = Config::db_path()?;
        self.status_message = "Scanning local library in background...".to_string();
        self.scan_handle = Some(tokio::task::spawn_blocking(move || {
            scan_library(&paths, db_path)
        }));
        Ok(())
    }

    /// Kick off a search in the background — previously this ran inline on
    /// the event loop, freezing the UI for the duration of several network
    /// round-trips. Results land via `poll_search`; `search_loading` drives
    /// the status-bar spinner meanwhile.
    pub fn start_search(&mut self) {
        let query = self.search_query.trim().to_string();
        if query.is_empty() {
            self.search_results.clear();
            self.search_selected = 0;
            return;
        }
        // A newer search supersedes one still in flight.
        if let Some(handle) = self.search_handle.take() {
            handle.abort();
        }
        self.search_results.clear();
        self.search_selected = 0;

        // A pasted share link resolves directly instead of being run through
        // scsearch/ytsearch as literal keywords (which finds nothing); a
        // Spotify playlist link imports the whole list into the library.
        self.status_message = if parse_spotify_playlist_id(&query).is_some() {
            "Importing Spotify playlist...".to_string()
        } else if let Some(source) = detect_url_source(&query) {
            format!("Resolving {} link...", source.as_str())
        } else {
            format!("Searching for '{}'...", query)
        };

        let db_path = match Config::db_path() {
            Ok(path) => path,
            Err(e) => {
                self.status_message = format!("Cannot search: {e}");
                return;
            }
        };
        self.search_loading = true;
        let filter = self.search_source_filter;
        let yt_dlp_path = self.config.player.yt_dlp_path.clone();
        let cookies = self.config.player.cookies_from_browser.clone();
        let spotify_id = self.config.spotify.client_id.clone();
        let spotify_secret = self.config.spotify.client_secret.clone();
        self.search_handle = Some(tokio::task::spawn(async move {
            run_search_task(query, filter, db_path, yt_dlp_path, cookies, spotify_id, spotify_secret).await
        }));
    }

    /// Merge in the results of a finished background search.
    pub fn poll_search(&mut self) {
        let Some(handle) = &self.search_handle else {
            return;
        };
        if !handle.is_finished() {
            return;
        }
        let handle = self.search_handle.take().unwrap();
        self.search_loading = false;
        let Some((tracks, error, imported)) = handle.now_or_never().and_then(|r| r.ok()) else {
            self.status_message = "Search task failed.".to_string();
            return;
        };
        let count = tracks.len();
        self.search_results = tracks;
        self.status_message = match (&imported, error) {
            (Some(name), _) => {
                // The import wrote a playlist — refresh the Library tab.
                let _ = self.load_playlists();
                format!("Imported playlist '{name}' ({count} tracks — see Library).")
            }
            (None, Some(e)) => format!("Found {count} results (partial: {e})."),
            (None, None) => format!("Found {count} results."),
        };
        // Mirror the old inline behavior: a fruitful Enter-search moves
        // focus from the input box to the results list.
        if count > 0
            && matches!(self.current_tab, Tab::Search)
            && matches!(self.search_focus, SearchFocus::Input)
        {
            self.search_focus = SearchFocus::Results;
        }
    }

    pub fn select_next(&mut self) {
        match self.current_tab {
            Tab::Dashboard => match self.dashboard_pane {
                DashboardPane::SoundCloud => {
                    if self.dashboard_sc_search {
                        if !self.dashboard_sc_results.is_empty() {
                            self.dashboard_sc_selected =
                                (self.dashboard_sc_selected + 1) % self.dashboard_sc_results.len();
                        }
                    } else if !self.soundcloud_user_tracks.is_empty() {
                        self.soundcloud_track_selected = (self.soundcloud_track_selected + 1)
                            % self.soundcloud_user_tracks.len();
                    }
                }
                DashboardPane::Queue => {
                    if !self.queue.is_empty() {
                        self.queue_selected = (self.queue_selected + 1) % self.queue.len();
                    }
                }
            },
            Tab::Search => {
                if !self.search_results.is_empty() {
                    self.search_selected = (self.search_selected + 1) % self.search_results.len();
                }
            }
            Tab::Queue => {
                if !self.queue.is_empty() {
                    self.queue_selected = (self.queue_selected + 1) % self.queue.len();
                }
            }
            Tab::Library => match self.library_pane {
                LibraryPane::Playlists => {
                    if !self.playlists.is_empty() {
                        self.selected_playlist =
                            (self.selected_playlist + 1) % self.playlists.len();
                        let _ = self.load_selected_playlist_tracks();
                    }
                }
                LibraryPane::Tracks => {
                    if !self.playlist_tracks.is_empty() {
                        self.selected_playlist_track =
                            (self.selected_playlist_track + 1) % self.playlist_tracks.len();
                    }
                }
            },
            Tab::SoundCloud => match self.soundcloud_pane {
                SoundCloudPane::Categories => {
                    self.soundcloud_category_selected =
                        (self.soundcloud_category_selected + 1) % SoundCloudCategory::ALL.len();
                }
                SoundCloudPane::Tracks => {
                    if !self.soundcloud_user_tracks.is_empty() {
                        self.soundcloud_track_selected = (self.soundcloud_track_selected + 1)
                            % self.soundcloud_user_tracks.len();
                    }
                }
            },
            _ => {}
        }
    }

    pub fn select_previous(&mut self) {
        match self.current_tab {
            Tab::Dashboard => match self.dashboard_pane {
                DashboardPane::SoundCloud => {
                    if self.dashboard_sc_search {
                        if !self.dashboard_sc_results.is_empty() {
                            let len = self.dashboard_sc_results.len();
                            self.dashboard_sc_selected =
                                (self.dashboard_sc_selected + len - 1) % len;
                        }
                    } else if !self.soundcloud_user_tracks.is_empty() {
                        let len = self.soundcloud_user_tracks.len();
                        self.soundcloud_track_selected =
                            (self.soundcloud_track_selected + len - 1) % len;
                    }
                }
                DashboardPane::Queue => {
                    if !self.queue.is_empty() {
                        self.queue_selected =
                            (self.queue_selected + self.queue.len() - 1) % self.queue.len();
                    }
                }
            },
            Tab::Search => {
                if !self.search_results.is_empty() {
                    self.search_selected =
                        (self.search_selected + self.search_results.len() - 1)
                            % self.search_results.len();
                }
            }
            Tab::Queue => {
                if !self.queue.is_empty() {
                    self.queue_selected =
                        (self.queue_selected + self.queue.len() - 1) % self.queue.len();
                }
            }
            Tab::Library => match self.library_pane {
                LibraryPane::Playlists => {
                    if !self.playlists.is_empty() {
                        self.selected_playlist =
                            (self.selected_playlist + self.playlists.len() - 1)
                                % self.playlists.len();
                        let _ = self.load_selected_playlist_tracks();
                    }
                }
                LibraryPane::Tracks => {
                    if !self.playlist_tracks.is_empty() {
                        self.selected_playlist_track =
                            (self.selected_playlist_track + self.playlist_tracks.len() - 1)
                                % self.playlist_tracks.len();
                    }
                }
            },
            Tab::SoundCloud => match self.soundcloud_pane {
                SoundCloudPane::Categories => {
                    let len = SoundCloudCategory::ALL.len();
                    self.soundcloud_category_selected =
                        (self.soundcloud_category_selected + len - 1) % len;
                }
                SoundCloudPane::Tracks => {
                    if !self.soundcloud_user_tracks.is_empty() {
                        let len = self.soundcloud_user_tracks.len();
                        self.soundcloud_track_selected =
                            (self.soundcloud_track_selected + len - 1) % len;
                    }
                }
            },
            _ => {}
        }
    }

    pub fn toggle_library_pane(&mut self) {
        if matches!(self.current_tab, Tab::Library) {
            self.library_pane = match self.library_pane {
                LibraryPane::Playlists => LibraryPane::Tracks,
                LibraryPane::Tracks => LibraryPane::Playlists,
            };
        }
    }

    pub async fn play_selected(&mut self) -> Result<()> {
        // A manual play supersedes any pending "fetch next page to continue"
        // autoplay — otherwise the page landing would restart the track the
        // user just picked.
        self.soundcloud_autoplay_pending = false;
        match self.current_tab {
            Tab::Dashboard => match self.dashboard_pane {
                DashboardPane::SoundCloud => {
                    if self.dashboard_sc_search {
                        if let Some(track) = self
                            .dashboard_sc_results
                            .get(self.dashboard_sc_selected)
                            .cloned()
                        {
                            self.playback_origin =
                                Some((PlaybackOrigin::SoundCloud, self.dashboard_sc_selected));
                            self.play_track(&track, true).await?;
                        }
                    } else if self.dashboard_sc_category_selected == DASHBOARD_SC_LIBRARY_SLOT {
                        self.current_tab = Tab::Library;
                    } else if self.soundcloud_user_tracks.is_empty() {
                        self.load_selected_soundcloud_category()?;
                    } else if let Some(track) = self
                        .soundcloud_user_tracks
                        .get(self.soundcloud_track_selected)
                        .cloned()
                    {
                        self.playback_origin =
                            Some((PlaybackOrigin::SoundCloud, self.soundcloud_track_selected));
                        self.play_track(&track, true).await?;
                    }
                }
                DashboardPane::Queue => {
                    if let Some(track) = self.queue.remove(self.queue_selected) {
                        self.queue_dirty = true;
                        self.playback_origin = None;
                        self.clamp_queue_selected();
                        self.play_track(&track, false).await?;
                    }
                }
            },
            Tab::Search => {
                if let Some(track) = self.search_results.get(self.search_selected).cloned() {
                    self.playback_origin = Some((PlaybackOrigin::Search, self.search_selected));
                    self.play_track(&track, true).await?;
                }
            }
            Tab::Queue => {
                // Playing a queue entry dequeues just that entry and leaves
                // the rest intact, so auto-advance continues through the
                // remaining queue — previously this passed clear_queue=true
                // and wiped the entire queue.
                if let Some(track) = self.queue.remove(self.queue_selected) {
                    self.queue_dirty = true;
                    self.playback_origin = None;
                    self.clamp_queue_selected();
                    self.play_track(&track, false).await?;
                }
            }
            Tab::Library => {
                if self.library_pane == LibraryPane::Tracks {
                    if let Some(track) = self.playlist_tracks.get(self.selected_playlist_track).cloned() {
                        self.playback_origin =
                            Some((PlaybackOrigin::Playlist, self.selected_playlist_track));
                        self.play_track(&track, true).await?;
                    }
                }
            }
            Tab::SoundCloud => {
                if self.soundcloud_pane == SoundCloudPane::Tracks {
                    if let Some(track) =
                        self.soundcloud_user_tracks.get(self.soundcloud_track_selected).cloned()
                    {
                        self.playback_origin =
                            Some((PlaybackOrigin::SoundCloud, self.soundcloud_track_selected));
                        self.play_track(&track, true).await?;
                    }
                } else {
                    self.load_selected_soundcloud_category()?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn add_selected_to_queue(&mut self) -> Result<()> {
        let track = match self.current_tab {
            Tab::Dashboard => match self.dashboard_pane {
                DashboardPane::SoundCloud if self.dashboard_sc_search => self
                    .dashboard_sc_results
                    .get(self.dashboard_sc_selected)
                    .cloned(),
                DashboardPane::SoundCloud => self
                    .soundcloud_user_tracks
                    .get(self.soundcloud_track_selected)
                    .cloned(),
                DashboardPane::Queue => None,
            },
            Tab::Search => self.search_results.get(self.search_selected).cloned(),
            Tab::Library => {
                if self.library_pane == LibraryPane::Tracks {
                    self.playlist_tracks.get(self.selected_playlist_track).cloned()
                } else {
                    None
                }
            }
            Tab::SoundCloud => {
                if self.soundcloud_pane == SoundCloudPane::Tracks {
                    self.soundcloud_user_tracks.get(self.soundcloud_track_selected).cloned()
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(track) = track {
            self.queue.push_back(track);
            self.queue_dirty = true;
            self.status_message = "Added to queue.".to_string();
        }
        Ok(())
    }

    /// Returns Ok(true) only if the track actually loaded and started —
    /// "soft" failures (unresolvable Spotify match, empty URL) are Ok(false).
    pub async fn play_track(&mut self, track: &UnifiedTrack, clear_queue: bool) -> Result<bool> {
        let playable_url = match track.source {
            TrackSource::Local => track.playable_url.clone(),
            TrackSource::YouTube => {
                let id = track.id.clone();
                self.youtube.get_audio_url(&id).await?
            }
            TrackSource::SoundCloud => self.soundcloud.get_audio_url(&track.playable_url).await?,
            TrackSource::Spotify => {
                // Resolve Spotify track via YouTube search.
                let query = format!("{} {}", track.artist, track.title);
                let results = self.youtube.search(&query, 1).await?;
                if let Some(first) = results.first() {
                    self.youtube.get_audio_url(&first.id).await?
                } else {
                    self.status_message =
                        format!("Could not resolve Spotify track on YouTube: {}", track.title);
                    return Ok(false);
                }
            }
        };

        // Guard against a stray multi-line/empty URL ever reaching mpv's IPC
        // command (that's what previously surfaced as an opaque
        // "error running command" from mpv).
        let playable_url = playable_url.lines().next().unwrap_or("").trim().to_string();
        if playable_url.is_empty() {
            self.status_message = format!("No playable URL for '{}'.", track.title);
            return Ok(false);
        }

        self.player.load(&playable_url, false).await?;
        self.current_track = Some(track.clone());
        self.is_playing = true;
        // A successful (manual or auto) play re-arms autoplay after a
        // previous auto-advance failure.
        self.autoplay_failed = false;
        self.status_message = format!("Playing: {} - {}", track.artist, track.title);
        self.maybe_fetch_artwork();
        self.maybe_fetch_waveform();

        if clear_queue {
            self.queue.clear();
            self.queue_dirty = true;
        }
        Ok(true)
    }

    pub async fn toggle_pause(&mut self) -> Result<()> {
        if self.current_track.is_none() {
            self.status_message = "Nothing playing.".to_string();
            return Ok(());
        }
        self.player.toggle_pause().await?;
        // Sync from mpv's actual state instead of blindly flipping a mirror
        // flag — mpv's pause can change without us (keep-open pauses at EOF).
        self.is_playing = !self.player.is_paused().await?;
        Ok(())
    }

    pub fn playback_state(&self) -> PlaybackState {
        if self.current_track.is_none() {
            PlaybackState::Stopped
        } else if self.is_playing {
            PlaybackState::Playing
        } else {
            PlaybackState::Paused
        }
    }

    /// Approximate perceived loudness in dB, treating 100% volume as 0 dB (unity gain).
    pub fn volume_db(&self) -> f64 {
        if self.volume == 0 {
            f64::NEG_INFINITY
        } else {
            20.0 * (self.volume as f64 / 100.0).log10()
        }
    }

    /// Keep the queue cursor inside the list after entries are removed
    /// (auto-advance pops, playing an entry directly, ...).
    fn clamp_queue_selected(&mut self) {
        if self.queue.is_empty() {
            self.queue_selected = 0;
        } else if self.queue_selected >= self.queue.len() {
            self.queue_selected = self.queue.len() - 1;
        }
    }

    /// Peek at the next queue entry WITHOUT removing it — the removal is
    /// committed (`commit_queue_advance`) only after the track actually
    /// starts, so a failed track stays in the queue instead of being
    /// silently consumed by a retry loop.
    fn peek_next_in_queue(&self) -> Option<(usize, UnifiedTrack)> {
        if self.queue.is_empty() {
            return None;
        }
        let idx = if self.shuffle {
            rand::thread_rng().gen_range(0..self.queue.len())
        } else {
            0
        };
        self.queue.get(idx).cloned().map(|t| (idx, t))
    }

    fn commit_queue_advance(&mut self, idx: usize) {
        self.queue.remove(idx);
        self.queue_dirty = true;
        self.clamp_queue_selected();
    }

    /// Peek at the next entry in the list the current track was played from
    /// (same peek-then-commit rationale as the queue).
    fn peek_next_in_origin(&self) -> Option<(usize, UnifiedTrack)> {
        let (origin, index) = self.playback_origin?;
        let next_index = index + 1;
        let next = match origin {
            PlaybackOrigin::Search => self.search_results.get(next_index),
            PlaybackOrigin::Playlist => self.playlist_tracks.get(next_index),
            PlaybackOrigin::SoundCloud => self.soundcloud_user_tracks.get(next_index),
        }
        .cloned()?;
        Some((next_index, next))
    }

    fn commit_origin_advance(&mut self, next_index: usize) {
        if let Some((origin, _)) = self.playback_origin {
            self.playback_origin = Some((origin, next_index));
        }
    }

    /// Ran off the end of an already-loaded SoundCloud page while browsing
    /// one — if that category has more, fetch the next page in the
    /// background and remember to resume playback with it once it lands
    /// (`poll_soundcloud_load`), instead of just stopping mid-Likes-list.
    fn try_continue_soundcloud_autoplay(&mut self) -> bool {
        let Some((PlaybackOrigin::SoundCloud, index)) = self.playback_origin else {
            return false;
        };
        if !self.soundcloud_has_more || self.soundcloud_loading {
            return false;
        }
        self.playback_origin = Some((PlaybackOrigin::SoundCloud, index + 1));
        self.soundcloud_autoplay_pending = true;
        self.load_more_soundcloud_tracks();
        true
    }

    /// Advance past the current track. `stop_if_empty` distinguishes a natural
    /// end-of-track (stop at the first broken entry, leave it in place, latch
    /// `autoplay_failed`) from a manual skip (keep going past broken entries —
    /// each failure commits the removal, so the candidates strictly shrink
    /// and the loop terminates).
    async fn advance_to_next(&mut self, stop_if_empty: bool) -> Result<()> {
        enum Target {
            Queue(usize),
            Origin(usize),
            /// Repeat-all replay of the just-finished track itself, used when
            /// the queue has run dry. Recomputed from `current_track` on every
            /// advance, so a successful replay needs no bookkeeping at all.
            Recycle,
        }

        // Repeat-all recycles the finished track — but ONLY when it came from
        // the queue (pushing an origin-list track into the queue would turn
        // origin playback into repeat-one and lose the origin), and it is
        // pushed back only once something else successfully starts (a failed
        // advance must not pile up duplicate copies).
        let mut recycle = if matches!(self.loop_mode, LoopMode::Queue)
            && self.playback_origin.is_none()
        {
            self.current_track.clone()
        } else {
            None
        };

        let mut last_failure: Option<String> = None;
        loop {
            let (target, next) = match self.peek_next_in_queue() {
                Some((idx, t)) => (Target::Queue(idx), t),
                None => match self.peek_next_in_origin() {
                    Some((i, t)) => (Target::Origin(i), t),
                    None => match recycle.take() {
                        Some(finished) => (Target::Recycle, finished),
                        None => break,
                    },
                },
            };

            match self.play_track(&next, false).await {
                Ok(true) => {
                    match target {
                        Target::Queue(idx) => {
                            self.commit_queue_advance(idx);
                            self.playback_origin = None;
                            if let Some(finished) = recycle {
                                self.queue.push_back(finished);
                                self.queue_dirty = true;
                            }
                        }
                        Target::Origin(i) => self.commit_origin_advance(i),
                        Target::Recycle => {}
                    }
                    return Ok(());
                }
                Ok(false) => {
                    last_failure = Some(format!("no playable URL for '{}'", next.title));
                }
                Err(e) => {
                    last_failure = Some(e.friendly());
                }
            }

            if stop_if_empty {
                self.autoplay_failed = true;
                self.status_message = match &last_failure {
                    Some(err) => format!("Autoplay stopped: {err} — press n to skip."),
                    None => "Autoplay stopped.".to_string(),
                };
                return Ok(());
            }
            // Manual skip: drop the broken entry, try the next candidate.
            match target {
                Target::Queue(idx) => self.commit_queue_advance(idx),
                Target::Origin(i) => self.commit_origin_advance(i),
                Target::Recycle => {}
            }
        }

        if self.try_continue_soundcloud_autoplay() {
            self.status_message = "End of loaded page — fetching more to continue...".to_string();
            return Ok(());
        }

        if stop_if_empty {
            self.current_track = None;
            self.is_playing = false;
            self.player.stop_playback().await?;
            self.status_message = "Queue finished.".to_string();
        } else {
            if let Some(err) = last_failure {
                self.status_message = format!("Skipped unplayable track: {err}");
            }
            self.player.next().await?;
        }
        Ok(())
    }

    pub async fn next_track(&mut self) -> Result<()> {
        // A manual skip supersedes a pending page-resume autoplay — the
        // landing page must not start a track the user just skipped past.
        self.soundcloud_autoplay_pending = false;
        self.advance_to_next(false).await
    }

    /// Called every tick to auto-advance when the current track has ended.
    /// Track-loop is handled entirely inside mpv (`loop-file`), so it's skipped here.
    pub async fn poll_playback(&mut self) {
        // Keep the Now Playing progress bar fresh whenever a track is loaded.
        if self.current_track.is_some() {
            if let Ok(pos) = self.player.get_position().await {
                self.playback_pos = pos;
            }
            if let Ok(dur) = self.player.get_duration().await {
                self.playback_dur = dur;
            }
        } else {
            self.playback_pos = 0.0;
            self.playback_dur = 0.0;
        }

        if self.current_track.is_none()
            || matches!(self.loop_mode, LoopMode::Track)
            || self.autoplay_failed
        {
            return;
        }
        if let Ok(true) = self.player.is_eof_reached().await {
            // Previously silently discarded: if advancing to the next track
            // failed (e.g. a dead link further down a Likes list), playback
            // just stopped with no explanation. advance_to_next now latches
            // autoplay_failed and explains itself in the status bar.
            if let Err(e) = self.advance_to_next(true).await {
                self.autoplay_failed = true;
                self.status_message = format!("Autoplay stopped: {e} — press n to skip.");
            }
        }
    }

    pub async fn cycle_loop_mode(&mut self) -> Result<()> {
        self.loop_mode = match self.loop_mode {
            LoopMode::Off => LoopMode::Track,
            LoopMode::Track => LoopMode::Queue,
            LoopMode::Queue => LoopMode::Off,
        };
        self.player
            .set_loop_file(matches!(self.loop_mode, LoopMode::Track))
            .await?;
        self.status_message = format!("Repeat: {}", self.loop_mode.label());
        Ok(())
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        self.status_message = format!("Shuffle: {}", if self.shuffle { "On" } else { "Off" });
    }

    pub fn cycle_palette(&mut self) {
        self.palette = match self.palette {
            AccentPalette::Auto => AccentPalette::Teal,
            AccentPalette::Teal => AccentPalette::Magenta,
            AccentPalette::Magenta => AccentPalette::Amber,
            AccentPalette::Amber => AccentPalette::Violet,
            AccentPalette::Violet => AccentPalette::Auto,
        };
        self.status_message = format!("Accent palette: {}", self.palette.label());
    }

    /// Whether the background local-library scan is still running — drives
    /// the status-bar spinner alongside `soundcloud_loading`.
    pub fn scan_in_progress(&self) -> bool {
        self.scan_handle.is_some()
    }

    pub fn eq_select_previous(&mut self) {
        self.eq_selected = self
            .eq_selected
            .checked_sub(1)
            .unwrap_or(eq::BAND_COUNT - 1);
    }

    pub fn eq_select_next(&mut self) {
        self.eq_selected = (self.eq_selected + 1) % eq::BAND_COUNT;
    }

    /// Adjust the selected band's gain by `delta` dB, clamped to
    /// `eq::MAX_GAIN_DB`. Marks the EQ dirty for `poll_eq` to pick up —
    /// the actual `af set` call is debounced, not sent here, so holding
    /// Up/Down doesn't spam mpv with one filter-chain replace per keypress.
    pub fn eq_adjust_selected(&mut self, delta: f64) {
        let gain = &mut self.eq_gains[self.eq_selected];
        *gain = (*gain + delta).clamp(-eq::MAX_GAIN_DB, eq::MAX_GAIN_DB);
        self.eq_preset = eq::matching_preset(&self.eq_gains)
            .unwrap_or("Custom")
            .to_string();
        self.eq_last_edit = Some(Instant::now());
    }

    /// Cycle to the next/previous built-in preset. Starting from "Custom"
    /// (no exact match) wraps in from the first preset ("Flat").
    pub fn eq_cycle_preset(&mut self, forward: bool) {
        let current = eq::PRESETS
            .iter()
            .position(|p| p.name == self.eq_preset)
            .unwrap_or(0);
        let len = eq::PRESETS.len();
        let next = if forward {
            (current + 1) % len
        } else {
            (current + len - 1) % len
        };
        let preset = &eq::PRESETS[next];
        self.eq_gains = preset.gains;
        self.eq_preset = preset.name.to_string();
        self.eq_last_edit = Some(Instant::now());
        self.status_message = format!("EQ preset: {}", preset.name);
    }

    /// Debounced EQ apply/persist, called every tick. Two separate debounce
    /// windows off the same edit timestamp: mpv gets the new filter chain
    /// quickly (still slow enough to coalesce a burst of keypresses into one
    /// `af set`), while config.toml is written less eagerly since disk I/O
    /// doesn't need to track the UI in real time.
    pub async fn poll_eq(&mut self) {
        const APPLY_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(200);
        const SAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(800);

        let Some(last_edit) = self.eq_last_edit else {
            return;
        };
        let elapsed = last_edit.elapsed();

        if self.eq_gains != self.eq_applied_gains && elapsed >= APPLY_DEBOUNCE {
            if let Err(e) = self.player.set_eq(self.eq_gains).await {
                self.status_message = format!("EQ error: {}", e.friendly());
            }
            self.eq_applied_gains = self.eq_gains;
        }

        if self.eq_gains != self.eq_saved_gains && elapsed >= SAVE_DEBOUNCE {
            self.config.eq.gains = self.eq_gains.to_vec();
            self.config.eq.preset = self.eq_preset.clone();
            let _ = self.config.save();
            self.eq_saved_gains = self.eq_gains;
        }
    }

    /// Toggle the live spectrum visualizer. Starts/stops the WASAPI
    /// loopback capture thread — mpv runs as a separate process we control
    /// over IPC and never see raw PCM from, so this captures system audio
    /// output independently instead (see `audio::capture`). Windows-only:
    /// the capture mechanism doesn't exist elsewhere.
    #[cfg(windows)]
    pub fn toggle_visualizer(&mut self) {
        self.visualizer_on = !self.visualizer_on;
        if self.visualizer_on {
            self.audio_capture = crate::audio::capture::AudioCapture::start();
            if self.audio_capture.is_none() {
                self.visualizer_on = false;
                self.status_message = "Visualizer: couldn't start audio capture.".to_string();
            } else {
                self.status_message = "Visualizer: on".to_string();
            }
        } else {
            self.audio_capture = None; // drops the capture thread
            self.status_message = "Visualizer: off".to_string();
        }
    }

    #[cfg(not(windows))]
    pub fn toggle_visualizer(&mut self) {
        self.status_message = "Visualizer requires Windows (WASAPI loopback capture).".to_string();
    }

    /// Pulls the latest captured samples and re-runs the FFT, called every
    /// tick while the visualizer is on. No-op (cheap: one bool check) the
    /// rest of the time, and a no-op entirely off Windows.
    #[cfg(windows)]
    pub fn poll_visualizer(&mut self) {
        if !self.visualizer_on {
            return;
        }
        let Some(capture) = &self.audio_capture else {
            return;
        };
        let samples = capture.latest(crate::audio::spectrum::FFT_SIZE);
        self.visualizer_bands = *self
            .visualizer_spectrum
            .analyze(&samples, crate::audio::capture::SAMPLE_RATE);
    }

    #[cfg(not(windows))]
    pub fn poll_visualizer(&mut self) {}

    pub async fn volume_up(&mut self) -> Result<()> {
        self.volume = (self.volume + 5).min(100);
        self.player.set_volume(self.volume).await?;
        Ok(())
    }

    pub async fn volume_down(&mut self) -> Result<()> {
        self.volume = self.volume.saturating_sub(5);
        self.player.set_volume(self.volume).await?;
        Ok(())
    }

    /// Skip ahead within the current track. No-op when nothing is loaded —
    /// mpv's `seek` errors on an idle player. `poll_playback` picks up the
    /// new position on the next tick, so the progress bar catches up on its
    /// own without an extra round-trip here.
    pub async fn seek_forward(&mut self) -> Result<()> {
        if self.current_track.is_none() {
            return Ok(());
        }
        self.player.seek(SEEK_STEP_SECS).await
    }

    /// Skip back within the current track. See `seek_forward`.
    pub async fn seek_backward(&mut self) -> Result<()> {
        if self.current_track.is_none() {
            return Ok(());
        }
        self.player.seek(-SEEK_STEP_SECS).await
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn toggle_game_mode(&mut self) {
        self.game_mode = !self.game_mode;
        self.status_message = if self.game_mode {
            "Game mode ON: reduced UI refresh.".to_string()
        } else {
            "Game mode OFF.".to_string()
        };
    }

    pub fn cycle_source_filter(&mut self) {
        self.search_source_filter = match self.search_source_filter {
            None => Some(TrackSource::Local),
            Some(TrackSource::Local) => Some(TrackSource::YouTube),
            Some(TrackSource::YouTube) => Some(TrackSource::SoundCloud),
            Some(TrackSource::SoundCloud) => Some(TrackSource::Spotify),
            Some(TrackSource::Spotify) => None,
        };
    }

    pub fn save_spotify_credentials(&mut self, client_id: &str, client_secret: &str) -> Result<()> {
        self.config.spotify.client_id = client_id.trim().to_string();
        self.config.spotify.client_secret = client_secret.trim().to_string();
        self.config.save()?;
        self.spotify = SpotifySource::new(
            &self.config.spotify.client_id,
            &self.config.spotify.client_secret,
        );
        self.status_message = "Spotify credentials saved.".to_string();
        Ok(())
    }

    pub fn toggle_search_focus(&mut self) {
        self.search_focus = match self.search_focus {
            SearchFocus::Input => SearchFocus::Results,
            SearchFocus::Results => SearchFocus::Input,
        };
    }

    pub fn save_soundcloud_username(&mut self, username: &str) -> Result<()> {
        let username = username.trim().trim_start_matches('@').to_string();
        self.config.soundcloud.username = username.clone();
        self.config.save()?;
        self.soundcloud_username = username;
        // Cancel any in-flight load for the OLD username and drop the
        // autoplay/origin state bound to it.
        self.abort_soundcloud_load();
        self.soundcloud_has_more = true;
        self.soundcloud_next_start = 1;
        self.soundcloud_pane = SoundCloudPane::Categories;
        self.soundcloud_user_tracks.clear();
        self.status_message = if self.soundcloud_username.is_empty() {
            "SoundCloud username cleared.".to_string()
        } else {
            format!("SoundCloud username set to '{}'.", self.soundcloud_username)
        };
        Ok(())
    }

    pub fn toggle_soundcloud_pane(&mut self) {
        if !matches!(self.current_tab, Tab::SoundCloud) {
            return;
        }
        self.soundcloud_pane = match self.soundcloud_pane {
            SoundCloudPane::Categories => SoundCloudPane::Tracks,
            SoundCloudPane::Tracks => SoundCloudPane::Categories,
        };
    }

    pub fn toggle_dashboard_pane(&mut self) {
        if !matches!(self.current_tab, Tab::Dashboard) {
            return;
        }
        self.dashboard_pane = match self.dashboard_pane {
            DashboardPane::SoundCloud => DashboardPane::Queue,
            DashboardPane::Queue => DashboardPane::SoundCloud,
        };
    }

    /// Dashboard Left/Right: switch between the Search / Tracks / Likes /
    /// Reposts / Library slots (or genre bucket when no username is
    /// configured), loading the selected category as it lands.
    pub fn cycle_dashboard_soundcloud_mode(&mut self, forward: bool) {
        if self.soundcloud_username.is_empty() {
            let len = SOUNDCLOUD_GENRES.len();
            self.soundcloud_genre_selected = if forward {
                (self.soundcloud_genre_selected + 1) % len
            } else {
                (self.soundcloud_genre_selected + len - 1) % len
            };
            self.dashboard_sc_search = false;
            self.load_selected_genre();
            return;
        }
        self.dashboard_sc_category_selected = if forward {
            (self.dashboard_sc_category_selected + 1) % DASHBOARD_SC_SLOT_COUNT
        } else {
            (self.dashboard_sc_category_selected + DASHBOARD_SC_SLOT_COUNT - 1)
                % DASHBOARD_SC_SLOT_COUNT
        };
        match self.dashboard_sc_category_selected {
            DASHBOARD_SC_SEARCH_SLOT => {
                self.dashboard_sc_search = true;
                self.dashboard_sc_search_focus = SearchFocus::Input;
                self.abort_dashboard_sc_search();
                self.status_message = "Search mode. Type a query and press Enter.".to_string();
            }
            DASHBOARD_SC_LIBRARY_SLOT => {
                self.dashboard_sc_search = false;
                self.status_message = "Press Enter to open Library.".to_string();
            }
            slot => {
                self.dashboard_sc_search = false;
                self.soundcloud_category_selected = slot - 1;
                let _ = self.load_selected_soundcloud_category();
            }
        }
    }

    fn abort_dashboard_sc_search(&mut self) {
        if let Some(handle) = self.dashboard_sc_search_handle.take() {
            handle.abort();
        }
        self.dashboard_sc_search_loading = false;
    }

    /// Searches all sources (local library, YouTube, SoundCloud, Spotify),
    /// same as the main Search tab — reuses `run_search_task` so the
    /// Dashboard's quick search isn't limited to SoundCloud.
    pub fn start_dashboard_sc_search(&mut self) {
        let query = self.dashboard_sc_query.trim().to_string();
        if query.is_empty() {
            self.dashboard_sc_results.clear();
            self.dashboard_sc_selected = 0;
            return;
        }
        if let Some(handle) = self.dashboard_sc_search_handle.take() {
            handle.abort();
        }
        self.dashboard_sc_results.clear();
        self.dashboard_sc_selected = 0;
        let db_path = match Config::db_path() {
            Ok(path) => path,
            Err(e) => {
                self.status_message = format!("Cannot search: {e}");
                return;
            }
        };
        self.dashboard_sc_search_loading = true;
        self.status_message = format!("Searching for '{}'...", query);
        let filter = self.search_source_filter;
        let yt_dlp_path = self.config.player.yt_dlp_path.clone();
        let cookies = self.config.player.cookies_from_browser.clone();
        let spotify_id = self.config.spotify.client_id.clone();
        let spotify_secret = self.config.spotify.client_secret.clone();
        self.dashboard_sc_search_handle = Some(tokio::task::spawn(async move {
            run_search_task(query, filter, db_path, yt_dlp_path, cookies, spotify_id, spotify_secret).await
        }));
    }

    pub fn poll_dashboard_sc_search(&mut self) {
        let Some(handle) = &self.dashboard_sc_search_handle else {
            return;
        };
        if !handle.is_finished() {
            return;
        }
        let handle = self.dashboard_sc_search_handle.take().unwrap();
        self.dashboard_sc_search_loading = false;
        let Some((tracks, error, _imported)) = handle.now_or_never().and_then(|r| r.ok()) else {
            self.status_message = "Search task failed.".to_string();
            return;
        };
        let count = tracks.len();
        self.dashboard_sc_results = tracks;
        self.status_message = match error {
            Some(e) => format!("Found {count} results (partial: {e})."),
            None => format!("Found {count} results."),
        };
        if count > 0 && self.dashboard_sc_search_focus == SearchFocus::Input {
            self.dashboard_sc_search_focus = SearchFocus::Results;
        }
    }

    /// Abort any in-flight SoundCloud page load and clear autoplay state
    /// that pointed into the old list. Dropping a JoinHandle only detaches
    /// the task (the yt-dlp child would keep running), so cancel properly.
    fn abort_soundcloud_load(&mut self) {
        if let Some((_, _, handle)) = self.soundcloud_load_handle.take() {
            handle.abort();
        }
        if let Some((_, handle)) = self.soundcloud_genre_handle.take() {
            handle.abort();
        }
        self.soundcloud_loading = false;
        self.soundcloud_autoplay_pending = false;
        if matches!(self.playback_origin, Some((PlaybackOrigin::SoundCloud, _))) {
            self.playback_origin = None;
        }
    }

    /// Load the selected genre bucket (browse mode when no SoundCloud
    /// username is configured) into the shared tracks pane.
    pub fn load_selected_genre(&mut self) {
        let genre = SOUNDCLOUD_GENRES[self.soundcloud_genre_selected];

        // Re-opening a recently-loaded bucket skips the yt-dlp round-trip.
        let cache_key = (String::new(), genre.to_string());
        let cached = self
            .soundcloud_list_cache
            .get(&cache_key)
            .filter(|(_, _, _, at)| at.elapsed() < SOUNDCLOUD_CACHE_TTL)
            .map(|(tracks, _, _, _)| tracks.clone());
        if let Some(tracks) = cached {
            if let Some((_, handle)) = self.soundcloud_genre_handle.take() {
                handle.abort();
            }
            self.soundcloud_loading = false;
            self.soundcloud_user_tracks = tracks;
            self.soundcloud_track_selected = 0;
            self.soundcloud_has_more = false;
            self.soundcloud_pane = SoundCloudPane::Tracks;
            self.status_message =
                format!("{genre}: {} tracks (cached).", self.soundcloud_user_tracks.len());
            return;
        }

        if let Some((_, _, handle)) = self.soundcloud_load_handle.take() {
            handle.abort();
        }
        if let Some((_, handle)) = self.soundcloud_genre_handle.take() {
            handle.abort();
        }
        self.soundcloud_user_tracks.clear();
        self.soundcloud_track_selected = 0;
        self.soundcloud_has_more = false;
        self.soundcloud_pane = SoundCloudPane::Tracks;
        self.soundcloud_autoplay_pending = false;
        if matches!(self.playback_origin, Some((PlaybackOrigin::SoundCloud, _))) {
            self.playback_origin = None;
        }
        self.soundcloud_loading = true;
        self.status_message = format!("Loading {genre}...");
        let yt_dlp_path = self.config.player.yt_dlp_path.clone();
        let cookies = self.config.player.cookies_from_browser.clone();
        let genre_label = genre.to_string();
        let task_query = genre.to_lowercase();
        let handle = tokio::task::spawn(async move {
            SoundCloudSource::new(yt_dlp_path, cookies).search(&task_query, 30).await
        });
        self.soundcloud_genre_handle = Some((genre_label, handle));
    }

    /// Merge in a finished genre load (see `load_selected_genre`).
    fn poll_genre_load(&mut self) {
        let Some((_, handle)) = &self.soundcloud_genre_handle else {
            return;
        };
        if !handle.is_finished() {
            return;
        }
        let (genre, handle) = self.soundcloud_genre_handle.take().unwrap();
        self.soundcloud_loading = false;
        // The genre selection moved on while this was loading — discard.
        if genre != SOUNDCLOUD_GENRES[self.soundcloud_genre_selected] {
            return;
        }
        match handle.now_or_never() {
            Some(Ok(Ok(tracks))) => {
                self.soundcloud_user_tracks = tracks;
                self.soundcloud_list_cache
                    .retain(|_, (_, _, _, at)| at.elapsed() < SOUNDCLOUD_CACHE_TTL);
                self.soundcloud_list_cache.insert(
                    (String::new(), genre.clone()),
                    (self.soundcloud_user_tracks.clone(), false, 1, Instant::now()),
                );
                self.status_message =
                    format!("{genre}: {} tracks.", self.soundcloud_user_tracks.len());
            }
            Some(Ok(Err(e))) => {
                self.status_message = format!("Error loading {genre}: {}", e.friendly())
            }
            _ => self.status_message = format!("{genre} load task failed."),
        }
    }

    /// Load the selected category (Tracks/Likes/Reposts) for the configured
    /// SoundCloud user and switch to the Tracks pane to show the results.
    /// Fetches happen in the background (see `poll_soundcloud_load`) so a
    /// large collection (Likes can run into the thousands) never blocks the UI.
    pub fn load_selected_soundcloud_category(&mut self) -> Result<()> {
        if self.soundcloud_username.is_empty() {
            // No username configured: the category list shows genre buckets
            // instead — Enter loads the selected one.
            self.load_selected_genre();
            return Ok(());
        }
        // A category switch while a page is in flight cancels the old fetch
        // and any autoplay state bound to the old list.
        self.abort_soundcloud_load();
        let category = SoundCloudCategory::ALL[self.soundcloud_category_selected];

        // Re-opening a recently-loaded list skips the yt-dlp round-trip.
        let cache_key = (self.soundcloud_username.clone(), category.label().to_string());
        let cached = self
            .soundcloud_list_cache
            .get(&cache_key)
            .filter(|(_, _, _, at)| at.elapsed() < SOUNDCLOUD_CACHE_TTL)
            .map(|(tracks, has_more, next_start, _)| (tracks.clone(), *has_more, *next_start));
        if let Some((tracks, has_more, next_start)) = cached {
            let count = tracks.len();
            self.soundcloud_user_tracks = tracks;
            self.soundcloud_has_more = has_more;
            self.soundcloud_next_start = next_start;
            self.soundcloud_track_selected = 0;
            self.soundcloud_pane = SoundCloudPane::Tracks;
            self.status_message = format!("{}: {} loaded (cached).", category.label(), count);
            return Ok(());
        }

        self.soundcloud_user_tracks.clear();
        self.soundcloud_track_selected = 0;
        self.soundcloud_has_more = true;
        self.soundcloud_next_start = 1;
        self.soundcloud_pane = SoundCloudPane::Tracks;
        self.spawn_soundcloud_page(category, 1);
        Ok(())
    }

    /// Fetch the next page of the currently loaded SoundCloud category.
    pub fn load_more_soundcloud_tracks(&mut self) {
        if self.soundcloud_loading || !self.soundcloud_has_more || self.soundcloud_username.is_empty()
        {
            return;
        }
        let category = SoundCloudCategory::ALL[self.soundcloud_category_selected];
        let start = self.soundcloud_next_start;
        self.spawn_soundcloud_page(category, start);
    }

    fn spawn_soundcloud_page(&mut self, category: SoundCloudCategory, start: usize) {
        // Replacing an in-flight load: abort it — dropping the handle would
        // only detach the task and leak the yt-dlp child.
        if let Some((_, _, old)) = self.soundcloud_load_handle.take() {
            old.abort();
        }
        self.soundcloud_loading = true;
        self.status_message = format!(
            "Loading {} {}-{}...",
            category.label(),
            start,
            start + SOUNDCLOUD_PAGE_SIZE - 1
        );
        let yt_dlp_path = self.config.player.yt_dlp_path.clone();
        let cookies = self.config.player.cookies_from_browser.clone();
        let username = self.soundcloud_username.clone();
        let suffix = category.url_suffix();
        let task_username = username.clone();
        let handle = tokio::task::spawn(async move {
            let source = SoundCloudSource::new(yt_dlp_path, cookies);
            source
                .user_category(&task_username, suffix, start, SOUNDCLOUD_PAGE_SIZE)
                .await
        });
        self.soundcloud_load_handle = Some((username, category, handle));
    }

    /// Poll the background SoundCloud page fetch and merge results in when done.
    pub async fn poll_soundcloud_load(&mut self) {
        self.poll_genre_load();
        let Some((_, _, handle)) = &self.soundcloud_load_handle else {
            return;
        };
        if !handle.is_finished() {
            return;
        }
        let (username, category, handle) = self.soundcloud_load_handle.take().unwrap();
        self.soundcloud_loading = false;

        // A username or category change while this page was in flight makes
        // the result stale — discard it instead of merging it into the new
        // list; whatever autoplay was waiting on it no longer applies.
        if username != self.soundcloud_username
            || category != SoundCloudCategory::ALL[self.soundcloud_category_selected]
        {
            self.soundcloud_autoplay_pending = false;
            return;
        }

        match handle.now_or_never().unwrap_or_else(|| {
            Ok(Err(ClimusicError::Source("load task panicked".into())))
        }) {
            Ok(Ok((page, raw_count))) => {
                self.soundcloud_user_tracks.extend(page);
                // Pagination advances by what yt-dlp actually returned, not
                // by how many entries survived parsing.
                self.soundcloud_next_start += raw_count;
                self.soundcloud_has_more = raw_count == SOUNDCLOUD_PAGE_SIZE;
                // Cache the list as loaded so far (prune expired first).
                self.soundcloud_list_cache
                    .retain(|_, (_, _, _, at)| at.elapsed() < SOUNDCLOUD_CACHE_TTL);
                self.soundcloud_list_cache.insert(
                    (username, category.label().to_string()),
                    (
                        self.soundcloud_user_tracks.clone(),
                        self.soundcloud_has_more,
                        self.soundcloud_next_start,
                        Instant::now(),
                    ),
                );
                self.status_message = if self.soundcloud_has_more {
                    format!(
                        "{}: {} loaded (press 'm' for more).",
                        category.label(),
                        self.soundcloud_user_tracks.len()
                    )
                } else {
                    format!(
                        "{}: {} loaded (all).",
                        category.label(),
                        self.soundcloud_user_tracks.len()
                    )
                };

                if self.soundcloud_autoplay_pending {
                    self.soundcloud_autoplay_pending = false;
                    if let Some((PlaybackOrigin::SoundCloud, index)) = self.playback_origin {
                        if let Some(track) = self.soundcloud_user_tracks.get(index).cloned() {
                            if let Err(e) = self.play_track(&track, false).await {
                                self.status_message = format!("Error resuming autoplay: {e}");
                            }
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                // A failed autoplay continuation fetch must latch — otherwise
                // poll_playback retries the same failing page every tick.
                if self.soundcloud_autoplay_pending {
                    self.autoplay_failed = true;
                }
                self.soundcloud_autoplay_pending = false;
                self.status_message = format!("Error loading SoundCloud page: {}", e.friendly());
            }
            Err(_) => {
                if self.soundcloud_autoplay_pending {
                    self.autoplay_failed = true;
                }
                self.soundcloud_autoplay_pending = false;
                self.status_message = "SoundCloud load task panicked.".to_string();
            }
        }
    }

    /// Kick off a background download+decode of the current track's artwork,
    /// if it has one and it isn't already loaded/loading.
    fn maybe_fetch_artwork(&mut self) {
        let Some(track) = self.current_track.clone() else {
            self.artwork_key = None;
            self.artwork = None;
            self.artwork_accent = None;
            *self.artwork_cache.borrow_mut() = None;
            self.artwork_handle = None;
            return;
        };

        // Key on the track itself, not the thumbnail URL: SoundCloud Likes/
        // Reposts/Tracks entries have no thumbnail at all up front (see
        // below), so the URL alone can't tell "same track" from "no track".
        let key = format!("{}:{}", track.source.as_str(), track.id);
        if Some(&key) == self.artwork_key.as_ref() {
            return;
        }
        self.artwork_key = Some(key);
        self.artwork = None;
        self.artwork_accent = None;
        *self.artwork_cache.borrow_mut() = None;
        self.artwork_handle = None;

        let direct_url = track.thumbnail_url.clone();
        // SoundCloud's flat-playlist listing (a user's Likes/Reposts/Tracks
        // page) carries no thumbnail field for any entry — only search
        // results do. Fall back to the public oEmbed endpoint, which
        // resolves it from the track's page URL with no auth required.
        // Only the currently-playing track ever takes this path, so it
        // stays cheap regardless of how large the list is.
        let oembed_fallback_url = if direct_url.is_none() && matches!(track.source, TrackSource::SoundCloud)
        {
            Some(track.playable_url.clone())
        } else {
            None
        };
        if direct_url.is_none() && oembed_fallback_url.is_none() {
            return;
        }

        self.artwork_handle = Some(tokio::task::spawn(async move {
            let thumb_url = match direct_url {
                Some(url) => url,
                None => {
                    let track_url = oembed_fallback_url?;
                    crate::sources::soundcloud::fetch_oembed_thumbnail(&track_url)
                        .await
                        .ok()
                        .flatten()?
                }
            };
            let bytes = crate::sources::http_client(std::time::Duration::from_secs(10))
                .get(&thumb_url)
                .send()
                .await
                .ok()?
                .bytes()
                .await
                .ok()?;
            image::load_from_memory(&bytes).ok()
        }));
    }

    /// Poll the background artwork fetch. The decoded image is stored as-is;
    /// `ui::player` encodes it to the terminal's graphics protocol lazily
    /// (and caches that encoding) the first time it's actually rendered.
    pub fn poll_artwork(&mut self) {
        let Some(handle) = &self.artwork_handle else {
            return;
        };
        if !handle.is_finished() {
            return;
        }
        let handle = self.artwork_handle.take().unwrap();
        if let Ok(Some(image)) = handle.now_or_never().unwrap_or(Ok(None)) {
            self.artwork_accent = extract_accent(&image);
            self.artwork = Some(image);
            *self.artwork_cache.borrow_mut() = None;
        }
    }

    /// Kick off a background waveform analysis for the current track, if it
    /// isn't already cached/loading. Mirrors `maybe_fetch_artwork`'s shape —
    /// see its comment for why the key is `"{source}:{id}"` rather than a
    /// URL. Unlike artwork, a successful result is also kept in
    /// `waveform_cache` for the rest of the session: waveform analysis is
    /// far more expensive to redo (a full decode, or a network fetch) than
    /// re-downloading a thumbnail.
    fn maybe_fetch_waveform(&mut self) {
        let Some(track) = self.current_track.clone() else {
            self.waveform_key = None;
            self.waveform = None;
            self.waveform_handle = None;
            return;
        };

        let key = format!("{}:{}", track.source.as_str(), track.id);
        if Some(&key) == self.waveform_key.as_ref() {
            return;
        }
        self.waveform_key = Some(key.clone());
        self.waveform_handle = None;

        if let Some(cached) = self.waveform_cache.get(&key) {
            self.waveform = Some(cached.clone());
            return;
        }
        self.waveform = None;

        // Spotify tracks are resolved through YouTube (or a local match)
        // only at play time, and that resolution isn't kept on the track —
        // re-deriving it here just for a waveform isn't worth another
        // search round-trip. Falls back to the plain progress bar.
        if matches!(track.source, TrackSource::Spotify) {
            return;
        }

        let yt_dlp_path = self.config.player.yt_dlp_path.clone();
        let cookies = self.config.player.cookies_from_browser.clone();
        let source = track.source;
        let playable_url = track.playable_url.clone();
        let waveform_url = track.waveform_url.clone();

        self.waveform_handle = Some(tokio::task::spawn(async move {
            match source {
                TrackSource::Local => {
                    tokio::task::spawn_blocking(move || crate::audio::waveform_from_file(&playable_url))
                        .await
                        .ok()
                        .flatten()
                }
                TrackSource::SoundCloud => {
                    // Prefer SoundCloud's own precomputed waveform (a few KB,
                    // no audio re-download) when yt-dlp exposed one; fall
                    // back to a low-bitrate decode otherwise.
                    if let Some(wurl) = &waveform_url
                        && let Some(w) = fetch_precomputed_waveform(wurl).await
                    {
                        return Some(w);
                    }
                    let sc = SoundCloudSource::new(&yt_dlp_path, &cookies);
                    let audio_url = sc.get_waveform_audio_url(&playable_url).await.ok()?;
                    let bytes = crate::sources::http_client(std::time::Duration::from_secs(30))
                        .get(&audio_url)
                        .send()
                        .await
                        .ok()?
                        .bytes()
                        .await
                        .ok()?
                        .to_vec();
                    tokio::task::spawn_blocking(move || crate::audio::waveform_from_bytes(bytes, None))
                        .await
                        .ok()
                        .flatten()
                }
                TrackSource::YouTube => {
                    let yt = YouTubeSource::new(&yt_dlp_path, &cookies);
                    let audio_url = yt.get_waveform_audio_url(&playable_url).await.ok()?;
                    let bytes = crate::sources::http_client(std::time::Duration::from_secs(30))
                        .get(&audio_url)
                        .send()
                        .await
                        .ok()?
                        .bytes()
                        .await
                        .ok()?
                        .to_vec();
                    tokio::task::spawn_blocking(move || crate::audio::waveform_from_bytes(bytes, None))
                        .await
                        .ok()
                        .flatten()
                }
                TrackSource::Spotify => None,
            }
        }));
    }

    /// Poll the background waveform fetch; a successful result is cached for
    /// the rest of the session under the same key `maybe_fetch_waveform` used.
    pub fn poll_waveform(&mut self) {
        let Some(handle) = &self.waveform_handle else {
            return;
        };
        if !handle.is_finished() {
            return;
        }
        let handle = self.waveform_handle.take().unwrap();
        if let Ok(Some(waveform)) = handle.now_or_never().unwrap_or(Ok(None)) {
            if let Some(key) = &self.waveform_key {
                self.waveform_cache.insert(key.clone(), waveform.clone());
            }
            self.waveform = Some(waveform);
        }
    }
}

/// A vibrant, mid-brightness swatch from the artwork's color palette, used to
/// accent the Now Playing panel so each track's screen feels like it belongs
/// to that cover — the idea (not the implementation) borrowed from Myx
/// (github.com/HaseebKhalid1507/Myx), which builds a full reactive theme off
/// the same trick; this is a deliberately smaller, single-accent version of it.
fn extract_accent(image: &image::DynamicImage) -> Option<(u8, u8, u8)> {
    let rgb = image.to_rgb8();
    let palette = color_thief::get_palette(rgb.as_raw(), color_thief::ColorFormat::Rgb, 10, 8).ok()?;
    palette
        .into_iter()
        .max_by_key(|c| {
            let (r, g, b) = (c.r as i32, c.g as i32, c.b as i32);
            let spread = r.max(g).max(b) - r.min(g).min(b); // rough saturation proxy
            let brightness = (r + g + b) / 3;
            let brightness_score = 255 - (brightness - 140).abs(); // favor mid-brightness
            spread * 2 + brightness_score
        })
        .map(|c| (c.r, c.g, c.b))
}

/// Fetches and parses a source's own precomputed waveform (currently: a
/// SoundCloud `waveform_url`). `None` on any failure — the caller falls
/// back to decoding audio itself.
async fn fetch_precomputed_waveform(url: &str) -> Option<Vec<f32>> {
    let bytes = crate::sources::http_client(std::time::Duration::from_secs(15))
        .get(url)
        .send()
        .await
        .ok()?
        .bytes()
        .await
        .ok()?;
    crate::audio::waveform_from_amplitude_json(&bytes)
}

/// The background half of search — runs entirely off the event loop.
/// Returns the merged results, an optional summary of which sources failed
/// (partial results are still shown, same as the old inline search), and —
/// for a pasted Spotify playlist link — the name of the playlist the tracks
/// were imported into.
async fn run_search_task(
    query: String,
    filter: Option<TrackSource>,
    db_path: PathBuf,
    yt_dlp_path: String,
    cookies: String,
    spotify_client_id: String,
    spotify_client_secret: String,
) -> (Vec<UnifiedTrack>, Option<String>, Option<String>) {
    let mut tracks = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    // A pasted Spotify playlist link imports the playlist (first page, up to
    // 100 tracks) into the library, and shows the tracks as results.
    if let Some(playlist_id) = parse_spotify_playlist_id(&query) {
        let mut spotify = SpotifySource::new(&spotify_client_id, &spotify_client_secret);
        return match spotify.playlist(&playlist_id).await {
            Ok((name, found, truncated)) => {
                let mut error = truncated.then(|| "first 100 tracks only".to_string());
                match Database::open(&db_path).and_then(|db| {
                    let id = db.create_playlist(&name)?;
                    for track in &found {
                        db.add_unified_track_to_playlist(id, track)?;
                    }
                    Ok(())
                }) {
                    Ok(()) => {}
                    Err(e) => error = Some(format!("import failed: {e}")),
                }
                (found, error, Some(name))
            }
            Err(e) => (tracks, Some(e.to_string()), None),
        };
    }

    if let Some(source) = detect_url_source(&query) {
        let resolved = match source {
            TrackSource::SoundCloud => SoundCloudSource::new(&yt_dlp_path, &cookies).resolve(&query).await,
            TrackSource::YouTube => YouTubeSource::new(&yt_dlp_path, &cookies).resolve(&query).await,
            _ => unreachable!(),
        };
        match resolved {
            Ok(track) => tracks.push(track),
            Err(e) => failures.push(format!("could not resolve link: {}", e.friendly())),
        }
        return (tracks, failures.first().cloned(), None);
    }

    // Local first, then web sources unless filtered out — same merge order
    // the inline search used.
    match Database::open(&db_path).and_then(|db| db.search_local(&query, 20)) {
        Ok(local) => tracks.extend(local.into_iter().map(|t| t.to_unified())),
        Err(e) => failures.push(format!("local: {e}")),
    }

    if filter.map(|s| s == TrackSource::YouTube).unwrap_or(true) {
        match YouTubeSource::new(&yt_dlp_path, &cookies).search(&query, 10).await {
            Ok(found) => tracks.extend(found),
            Err(e) => failures.push(format!("YouTube: {}", e.friendly())),
        }
    }

    if filter.map(|s| s == TrackSource::SoundCloud).unwrap_or(true) {
        match SoundCloudSource::new(&yt_dlp_path, &cookies).search(&query, 10).await {
            Ok(found) => tracks.extend(found),
            Err(e) => failures.push(format!("SoundCloud: {}", e.friendly())),
        }
    }

    let mut spotify = SpotifySource::new(&spotify_client_id, &spotify_client_secret);
    if filter.map(|s| s == TrackSource::Spotify).unwrap_or(true) && spotify.is_configured() {
        match spotify.search(&query, 10).await {
            Ok(found) => tracks.extend(found),
            Err(e) => failures.push(format!("Spotify: {}", e.friendly())),
        }
    }

    let error = if failures.is_empty() {
        None
    } else {
        Some(failures.join("; "))
    };
    (tracks, error, None)
}

/// Extract the playlist id from a pasted open.spotify.com/playlist/<id> link
/// (Spotify ids are base62, so this also strips any `?si=...` suffix).
fn parse_spotify_playlist_id(query: &str) -> Option<String> {
    let rest = query
        .trim()
        .strip_prefix("https://open.spotify.com/playlist/")
        .or_else(|| query.trim().strip_prefix("http://open.spotify.com/playlist/"))?;
    let id: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
    if id.is_empty() { None } else { Some(id) }
}

/// Detects a pasted SoundCloud/YouTube share link so it can be resolved
/// directly instead of being mangled into a keyword search.
fn detect_url_source(query: &str) -> Option<TrackSource> {
    let q = query.to_lowercase();
    if !(q.starts_with("http://") || q.starts_with("https://")) {
        return None;
    }
    if q.contains("soundcloud.com") {
        Some(TrackSource::SoundCloud)
    } else if q.contains("youtube.com") || q.contains("youtu.be") {
        Some(TrackSource::YouTube)
    } else {
        None
    }
}

fn scan_library(paths: &[String], db_path: PathBuf) -> Result<()> {
    let mut db = Database::open(db_path)?;

    let mut tracks = Vec::new();
    for path in paths {
        let expanded = expand_path(path);
        if expanded.exists() {
            tracks.extend(scan_directory(&expanded)?);
        } else {
            // Was silently skipped — a typo'd or unmounted scan path just
            // produced an empty library with no explanation.
            tracing::warn!("library scan: configured path '{}' does not exist", expanded.display());
        }
    }

    // One atomic sync: upserts keep row ids (and the playlist references
    // pointing at them) stable across rescans; only files that actually
    // vanished from disk are removed from the index.
    db.sync_local_tracks(&tracks)?;
    Ok(())
}
