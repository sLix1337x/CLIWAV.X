use anyhow::Result;
use climusic::app::{App, DashboardPane, InputPrompt, PromptKind, SearchFocus, Tab};
use climusic::ui;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::time::{Duration, Instant};
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(name = "CLIWAV.X")]
#[command(about = "Low-overhead CLI music player")]
struct Args {
    /// Optional search query to run on startup
    #[arg(short, long)]
    query: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Logs go to a file, not stdout: the TUI owns stdout once the alternate
    // screen is active, and any log line written there corrupts the display.
    let log_file =
        std::fs::File::create(climusic::config::Config::cache_dir()?.join("climusic.log"))?;
    tracing_subscriber::fmt()
        .with_env_filter("climusic=info")
        .with_writer(std::sync::Mutex::new(log_file))
        .with_ansi(false)
        .init();

    let args = Args::parse();

    let mut terminal = setup_terminal()?;
    let mut app = match App::new().await {
        Ok(app) => app,
        Err(e) => {
            restore_terminal(&mut terminal)?;
            anyhow::bail!("failed to initialize app: {e}");
        }
    };

    if let Some(query) = args.query {
        app.search_query = query;
        app.start_search();
    }

    let result = run_event_loop(&mut terminal, &mut app).await;

    if let Err(e) = app.player.stop().await {
        error!("failed to stop player: {e}");
    }

    restore_terminal(&mut terminal)?;

    if let Err(e) = result {
        error!("event loop error: {e}");
    }

    info!("climusic exited");
    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let mut last_tick = Instant::now();

    loop {
        let tick_rate = if app.game_mode {
            Duration::from_millis(1000)
        } else {
            Duration::from_millis(250)
        };

        terminal.draw(|f| ui::draw(f, app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if handle_key(key, app).await? {
                        break;
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.tick = app.tick.wrapping_add(1);
            app.poll_scan();
            app.poll_playback().await;
            app.poll_soundcloud_load().await;
            app.poll_dashboard_sc_search();
            app.poll_search();
            app.poll_artwork();
            app.maybe_save_queue();
            last_tick = Instant::now();
        }
    }

    Ok(())
}

/// Run a fallible app action without letting the error kill the whole TUI —
/// it's surfaced in the status bar instead. A bad mpv/yt-dlp/db call used to
/// propagate all the way out of the event loop and silently exit the app.
macro_rules! run {
    ($app:expr, $action:expr) => {
        if let Err(e) = $action {
            $app.status_message = format!("Error: {}", e.friendly());
        }
    };
}

/// Returns true if the app should quit.
async fn handle_key(key: event::KeyEvent, app: &mut App) -> Result<bool> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Handle input prompts first.
    if app.input_prompt.is_some() {
        return handle_prompt_key(key, app).await;
    }

    // Help overlay swallows all keys except its own close keys and the
    // quit keys it advertises.
    if app.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') => app.show_help = false,
            KeyCode::Char('q') if !ctrl => app.show_help = false,
            KeyCode::Char('Q') => return Ok(true),
            KeyCode::Char('c') | KeyCode::Char('q') if ctrl => return Ok(true),
            _ => {}
        }
        return Ok(false);
    }

    // While the Search box has focus, printable characters are always query
    // text — never shortcuts. This is what makes pasting a URL (which is
    // very likely to contain letters like 'n', 's', 'f', ...) work instead
    // of triggering unrelated actions (or, previously, crashing the app —
    // see the `run!` macro above for why a stray 'n' used to be fatal).
    if matches!(app.current_tab, Tab::Search) && matches!(app.search_focus, SearchFocus::Input) {
        match key.code {
            // AltGr produces Char with CONTROL|ALT on Windows (e.g. '@' is
            // AltGr+Q on German layouts) — that's text, not a shortcut.
            KeyCode::Char(c) if !ctrl || key.modifiers.contains(KeyModifiers::ALT) => {
                app.search_query.push(c);
                return Ok(false);
            }
            KeyCode::Backspace => {
                app.search_query.pop();
                return Ok(false);
            }
            _ => {} // let Enter/Tab/BackTab/Esc/arrows fall through below
        }
    }

    // Same typing-first behavior for the Dashboard SoundCloud search box.
    if matches!(app.current_tab, Tab::Dashboard)
        && matches!(app.dashboard_pane, DashboardPane::SoundCloud)
        && app.dashboard_sc_search
        && matches!(app.dashboard_sc_search_focus, SearchFocus::Input)
    {
        match key.code {
            KeyCode::Char(c) if !ctrl || key.modifiers.contains(KeyModifiers::ALT) => {
                app.dashboard_sc_query.push(c);
                return Ok(false);
            }
            KeyCode::Backspace => {
                app.dashboard_sc_query.pop();
                return Ok(false);
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('c') if ctrl => return Ok(true),
        KeyCode::Char('Q') => return Ok(true),

        KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Char('z') if ctrl => run!(app, app.undo_delete_playlist()),

        KeyCode::Tab => match app.current_tab {
            Tab::Dashboard => app.toggle_dashboard_pane(),
            Tab::Library => app.toggle_library_pane(),
            Tab::Search => app.toggle_search_focus(),
            Tab::SoundCloud => app.toggle_soundcloud_pane(),
            _ => {
                app.current_tab = match app.current_tab {
                    Tab::Dashboard => Tab::NowPlaying,
                    Tab::NowPlaying => Tab::Library,
                    Tab::Library => Tab::Queue,
                    Tab::Queue => Tab::SoundCloud,
                    Tab::SoundCloud => Tab::Search,
                    Tab::Search => Tab::Dashboard,
                };
            }
        },
        KeyCode::BackTab => {
            app.current_tab = match app.current_tab {
                Tab::Dashboard => Tab::Search,
                Tab::NowPlaying => Tab::Dashboard,
                Tab::Library => Tab::NowPlaying,
                Tab::Queue => Tab::Library,
                Tab::SoundCloud => Tab::Queue,
                Tab::Search => Tab::SoundCloud,
            };
            if matches!(app.current_tab, Tab::Search) {
                app.search_focus = SearchFocus::Input;
            }
        }

        // Jump order matches the tab bar: Dashboard, Now Playing, Library,
        // Queue, SoundCloud, Search.
        KeyCode::Char('1') => app.current_tab = Tab::Dashboard,
        KeyCode::Char('2') => app.current_tab = Tab::NowPlaying,
        KeyCode::Char('3') => app.current_tab = Tab::Library,
        KeyCode::Char('4') => app.current_tab = Tab::Queue,
        KeyCode::Char('5') => app.current_tab = Tab::SoundCloud,
        KeyCode::Char('6') => {
            app.current_tab = Tab::Search;
            app.search_focus = SearchFocus::Input;
        }

        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Up | KeyCode::Char('k') => app.select_previous(),

        // Rewind/fast-forward the current track — checked before the
        // Dashboard-specific arrows below so Shift+Left/Right always seeks,
        // regardless of which tab/pane has focus.
        KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => {
            run!(app, app.seek_backward().await);
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => {
            run!(app, app.seek_forward().await);
        }

        // Dashboard SoundCloud pane: switch Search/Tracks/Likes/Reposts/Library.
        KeyCode::Left
            if matches!(app.current_tab, Tab::Dashboard)
                && matches!(app.dashboard_pane, DashboardPane::SoundCloud) =>
        {
            app.cycle_dashboard_soundcloud_mode(false);
        }
        KeyCode::Right
            if matches!(app.current_tab, Tab::Dashboard)
                && matches!(app.dashboard_pane, DashboardPane::SoundCloud) =>
        {
            app.cycle_dashboard_soundcloud_mode(true);
        }

        KeyCode::Enter
            if matches!(app.current_tab, Tab::Search) && matches!(app.search_focus, SearchFocus::Input) =>
        {
            // Background search: focus moves to the results when they land
            // (see poll_search) — the UI stays responsive meanwhile.
            app.start_search();
        }
        KeyCode::Enter
            if matches!(app.current_tab, Tab::Dashboard)
                && matches!(app.dashboard_pane, DashboardPane::SoundCloud)
                && app.dashboard_sc_search
                && matches!(app.dashboard_sc_search_focus, SearchFocus::Input) =>
        {
            app.start_dashboard_sc_search();
        }
        KeyCode::Enter => run!(app, app.play_selected().await),
        KeyCode::Char('a') => run!(app, app.add_selected_to_queue().await),
        KeyCode::Char('s') => run!(app, app.save_selected_track_to_library()),
        KeyCode::Char(' ') => run!(app, app.toggle_pause().await),

        KeyCode::Char('n') if matches!(app.current_tab, Tab::Library) => {
            app.input_prompt = Some(InputPrompt {
                title: "New playlist name".to_string(),
                value: String::new(),
                kind: PromptKind::NewPlaylist,
            });
        }
        KeyCode::Char('n') => run!(app, app.next_track().await),

        KeyCode::Char('+') | KeyCode::Char('=') => run!(app, app.volume_up().await),
        KeyCode::Char('-') => run!(app, app.volume_down().await),

        KeyCode::Char('f') => app.cycle_source_filter(),
        KeyCode::Char('g') => app.toggle_game_mode(),
        KeyCode::Char('r') => run!(app, app.rescan_local_library().await),
        KeyCode::Char('l') => run!(app, app.cycle_loop_mode().await),
        KeyCode::Char('x') => app.toggle_shuffle(),
        KeyCode::Char('t') => app.cycle_palette(),
        KeyCode::Char('m') if matches!(app.current_tab, Tab::SoundCloud | Tab::Dashboard) => {
            app.load_more_soundcloud_tracks();
        }

        KeyCode::Char('p') if matches!(app.current_tab, Tab::Search) => {
            run!(app, add_selected_to_playlist(app));
        }

        KeyCode::Char('d') if matches!(app.current_tab, Tab::Library) => {
            if app.selected_playlist_is_saved_tracks() {
                run!(app, app.remove_selected_saved_track());
            } else {
                run!(app, app.delete_selected_playlist());
            }
        }

        KeyCode::Char('S') => {
            app.input_prompt = Some(InputPrompt {
                title: "Spotify client ID".to_string(),
                value: app.config.spotify.client_id.clone(),
                kind: PromptKind::SpotifyClientId,
            });
        }
        KeyCode::Char('C') => {
            app.input_prompt = Some(InputPrompt {
                title: "SoundCloud username".to_string(),
                value: app.soundcloud_username.clone(),
                kind: PromptKind::SoundCloudUsername,
            });
        }

        _ => {}
    }

    Ok(false)
}

async fn handle_prompt_key(key: event::KeyEvent, app: &mut App) -> Result<bool> {
    // Ctrl-modified keys are not text: Ctrl+C/Ctrl+Q quit even with a prompt
    // open, and any other Ctrl combo is swallowed rather than typed into the
    // field. AltGr (Ctrl+Alt on Windows) IS text though — e.g. '@' is
    // AltGr+Q on German layouts — so only swallow Ctrl-without-Alt.
    let ctrl_only = key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT);
    if ctrl_only {
        if let KeyCode::Char('c') | KeyCode::Char('q') = key.code {
            return Ok(true);
        }
        return Ok(false);
    }
    match key.code {
        KeyCode::Enter => {
            if let Some(prompt) = app.input_prompt.take() {
                match prompt.kind {
                    PromptKind::NewPlaylist => {
                        run!(app, app.create_playlist(&prompt.value));
                    }
                    PromptKind::SpotifyClientId => {
                        app.input_prompt = Some(InputPrompt {
                            title: "Spotify client secret".to_string(),
                            value: String::new(),
                            kind: PromptKind::SpotifyClientSecret {
                                client_id: prompt.value,
                            },
                        });
                    }
                    PromptKind::SpotifyClientSecret { client_id } => {
                        run!(app, app.save_spotify_credentials(&client_id, &prompt.value));
                    }
                    PromptKind::SoundCloudUsername => {
                        run!(app, app.save_soundcloud_username(&prompt.value));
                    }
                }
            }
        }
        KeyCode::Esc => {
            app.input_prompt = None;
            app.status_message = "Cancelled.".to_string();
        }
        KeyCode::Backspace => {
            if let Some(prompt) = app.input_prompt.as_mut() {
                prompt.value.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Some(prompt) = app.input_prompt.as_mut() {
                prompt.value.push(c);
            }
        }
        _ => {}
    }
    Ok(false)
}

fn add_selected_to_playlist(app: &mut App) -> climusic::error::Result<()> {
    if app.playlists.is_empty() {
        app.status_message = "No playlists. Create one in the Library tab.".to_string();
        return Ok(());
    }
    // For simplicity, add to the currently selected playlist in the Library tab.
    let index = app.selected_playlist;
    app.add_selected_track_to_playlist(index)?;
    Ok(())
}
