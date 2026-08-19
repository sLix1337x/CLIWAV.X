use crate::app::{App, LibraryPane, SAVED_TRACKS_PLAYLIST_ID};
use crate::ui::{accent_color, draw_section, highlight_style, is_now_playing, muted_style, normal_style, source_color, source_glyph, track_name_spans, zebra_style};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    // Below ~60 columns the two panes crush each other into unreadability —
    // show only the focused one (Tab still switches between them).
    if area.width < 60 {
        match app.library_pane {
            LibraryPane::Playlists => draw_playlists(frame, app, area),
            LibraryPane::Tracks => draw_tracks(frame, app, area),
        }
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    draw_playlists(frame, app, chunks[0]);
    draw_tracks(frame, app, chunks[1]);
}

fn draw_playlists(frame: &mut Frame, app: &App, area: Rect) {
    let active = matches!(app.library_pane, LibraryPane::Playlists);
    let list_area = draw_section(frame, area, "Playlists", active);

    let items: Vec<ListItem> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(i, playlist)| {
            let (glyph, glyph_style) = if playlist.id == SAVED_TRACKS_PLAYLIST_ID {
                ("♥ ", Style::default().fg(Color::Red))
            } else {
                ("▸ ", muted_style())
            };
            ListItem::new(Line::from(vec![
                Span::styled(glyph, glyph_style),
                Span::styled(&playlist.name, normal_style()),
            ]))
            .style(zebra_style(i, app.is_dark_bg))
        })
        .collect();

    let list = List::new(items).highlight_style(highlight_style());
    let mut state = ListState::default();
    if active && !app.playlists.is_empty() {
        state.select(Some(app.selected_playlist));
    }
    frame.render_stateful_widget(list, list_area, &mut state);
}

fn draw_tracks(frame: &mut Frame, app: &App, area: Rect) {
    let active = matches!(app.library_pane, LibraryPane::Tracks);

    let saved_tracks_selected = app
        .playlists
        .get(app.selected_playlist)
        .is_some_and(|p| p.id == SAVED_TRACKS_PLAYLIST_ID);

    if app.playlists.is_empty() {
        let content_area = draw_section(frame, area, "Tracks", active);
        let para = Paragraph::new("No playlists. Press 'n' to create one.").style(muted_style());
        frame.render_widget(para, content_area);
        return;
    }

    if saved_tracks_selected && app.playlist_tracks.is_empty() {
        let content_area = draw_section(frame, area, "Saved Tracks", active);
        let para = Paragraph::new("No saved tracks. Press 's' on a track to save it.")
            .style(muted_style());
        frame.render_widget(para, content_area);
        return;
    }

    let playlist_name = app
        .playlists
        .get(app.selected_playlist)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Tracks".to_string());
    let list_area = draw_section(frame, area, &playlist_name, active);

    let accent = accent_color(app);
    let items: Vec<ListItem> = app
        .playlist_tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let duration = track
                .duration_ms
                .map(format_duration)
                .unwrap_or_else(|| "--:--".to_string());

            let mut spans = vec![Span::styled(
                format!("{} {:<10} ", source_glyph(track.source), track.source.as_str()),
                Style::default().fg(source_color(track.source)),
            )];
            spans.extend(track_name_spans(
                &track.artist,
                &track.title,
                is_now_playing(app, track),
                accent,
            ));
            spans.push(Span::styled(format!("  ({})", duration), muted_style()));
            ListItem::new(Line::from(spans)).style(zebra_style(i, app.is_dark_bg))
        })
        .collect();

    let list = List::new(items).highlight_style(highlight_style());
    let mut state = ListState::default();
    if active && !app.playlist_tracks.is_empty() {
        state.select(Some(app.selected_playlist_track));
    }
    frame.render_stateful_widget(list, list_area, &mut state);
}

fn format_duration(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{}:{:02}", minutes, seconds)
}
