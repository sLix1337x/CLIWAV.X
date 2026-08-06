use crate::app::{App, DashboardPane, SearchFocus};
use crate::ui::{
    accent_color, draw_section, highlight_style, is_now_playing, muted_style, normal_style,
    player::draw_mini_now_playing, source_color, source_glyph, spinner_frame, track_name_spans,
    zebra_style,
};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;

/// The landing tab: what's playing (artwork + info), the SoundCloud
/// Tracks/Likes/Reposts lists, and the queue — one glanceable screen.
pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    // Three columns need room; at narrow widths show only the focused pane
    // (Tab still switches, and the Now Playing pane is display-only anyway).
    if area.width < 75 {
        match app.dashboard_pane {
            DashboardPane::SoundCloud => draw_soundcloud(frame, app, area),
            DashboardPane::Queue => draw_queue(frame, app, area),
        }
        return;
    }

    let cols = Layout::horizontal([
        Constraint::Percentage(30),
        Constraint::Percentage(48),
        Constraint::Percentage(22),
    ])
    .split(area);

    draw_now_playing(frame, app, cols[0]);
    draw_soundcloud(frame, app, cols[1]);
    draw_queue(frame, app, cols[2]);
}

fn draw_now_playing(frame: &mut Frame, app: &App, area: Rect) {
    // Display-only pane: never marked active, it takes no keyboard focus.
    let content = draw_section(frame, area, "Now Playing", false);
    draw_mini_now_playing(frame, app, content);
}

fn draw_soundcloud(frame: &mut Frame, app: &App, area: Rect) {
    let active = matches!(app.dashboard_pane, DashboardPane::SoundCloud);
    let accent = accent_color(app);

    // When a SoundCloud username is configured, the Dashboard selector adds a
    // fourth "Search" mode that shows a query box + results list.
    let search_mode = !app.soundcloud_username.is_empty() && app.dashboard_sc_search;
    let rows = if search_mode {
        Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area)
    } else {
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area)
    };

    // Category selector line: "◂ Tracks · Likes · Reposts · Search ▸" (Left/Right
    // to switch, which also reloads the list). Without a username the selector
    // browses genre buckets instead — too many to show, so just the current.
    let selector: Vec<Span> = if app.soundcloud_username.is_empty() {
        let genre = crate::app::SOUNDCLOUD_GENRES[app.soundcloud_genre_selected];
        vec![
            Span::styled("◂ ", muted_style()),
            Span::styled(
                genre,
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ▸", muted_style()),
        ]
    } else {
        let labels = ["Tracks", "Likes", "Reposts", "Search"];
        let mut spans = vec![Span::styled("◂ ", muted_style())];
        for (i, label) in labels.iter().enumerate() {
            let style = if i == app.dashboard_sc_category_selected {
                Style::default().fg(accent).add_modifier(Modifier::BOLD)
            } else {
                muted_style()
            };
            spans.push(Span::styled(*label, style));
            spans.push(Span::styled(
                if i + 1 < labels.len() { " · " } else { " ▸" },
                muted_style(),
            ));
        }
        spans
    };
    frame.render_widget(
        Paragraph::new(Line::from(selector)).alignment(Alignment::Center),
        rows[0],
    );

    if search_mode {
        let input_area = rows[1];
        let list_area = draw_section(frame, rows[2], "SoundCloud Search", active);

        let input_spans = vec![
            Span::styled("Search: ", muted_style()),
            Span::styled(&app.dashboard_sc_query, normal_style()),
            if matches!(app.dashboard_sc_search_focus, SearchFocus::Input) && active {
                Span::styled("▏", Style::default().fg(accent))
            } else {
                Span::styled("", Style::default())
            },
        ];
        frame.render_widget(
            Paragraph::new(Line::from(input_spans)),
            input_area,
        );

        if app.dashboard_sc_results.is_empty() {
            let hint = if app.dashboard_sc_search_loading {
                format!("{} Loading...", spinner_frame(app.tick))
            } else {
                "Type a query and press Enter.".to_string()
            };
            frame.render_widget(Paragraph::new(hint).style(muted_style()), list_area);
            return;
        }

        let items: Vec<ListItem> = app
            .dashboard_sc_results
            .iter()
            .enumerate()
            .map(|(i, track)| {
                let duration = track
                    .duration_ms
                    .map(format_duration)
                    .unwrap_or_else(|| "--:--".to_string());
                let mut spans = track_name_spans(
                    &track.artist,
                    &track.title,
                    is_now_playing(app, track),
                    accent,
                );
                spans.push(Span::styled(format!("  ({})", duration), muted_style()));
                ListItem::new(Line::from(spans)).style(zebra_style(i))
            })
            .collect();

        let list = List::new(items).highlight_style(highlight_style());
        let mut state = ListState::default();
        if active {
            state.select(Some(app.dashboard_sc_selected));
        }
        frame.render_stateful_widget(list, list_area, &mut state);
        return;
    }

    let list_area = draw_section(frame, rows[1], "SoundCloud", active);

    if app.soundcloud_user_tracks.is_empty() {
        let hint = if app.soundcloud_loading {
            format!("{} Loading...", spinner_frame(app.tick))
        } else if app.soundcloud_username.is_empty() {
            "◂ ▸ pick a genre, Enter to load.".to_string()
        } else {
            "Enter to load, ← → to switch category.".to_string()
        };
        frame.render_widget(Paragraph::new(hint).style(muted_style()), list_area);
        return;
    }

    let items: Vec<ListItem> = app
        .soundcloud_user_tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let duration = track
                .duration_ms
                .map(format_duration)
                .unwrap_or_else(|| "--:--".to_string());
            let mut spans =
                track_name_spans(&track.artist, &track.title, is_now_playing(app, track), accent);
            spans.push(Span::styled(format!("  ({})", duration), muted_style()));
            ListItem::new(Line::from(spans)).style(zebra_style(i))
        })
        .collect();

    let list = List::new(items).highlight_style(highlight_style());
    let mut state = ListState::default();
    if active {
        state.select(Some(app.soundcloud_track_selected));
    }
    frame.render_stateful_widget(list, list_area, &mut state);
}

fn draw_queue(frame: &mut Frame, app: &App, area: Rect) {
    let active = matches!(app.dashboard_pane, DashboardPane::Queue);
    let list_area = draw_section(frame, area, &format!("Queue ({})", app.queue.len()), active);

    if app.queue.is_empty() {
        frame.render_widget(
            Paragraph::new("Queue is empty ('a' adds tracks).").style(muted_style()),
            list_area,
        );
        return;
    }

    let accent = accent_color(app);
    let items: Vec<ListItem> = app
        .queue
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let mut spans = vec![
                Span::styled(format!("{:>2}. ", i + 1), muted_style()),
                Span::styled(
                    format!("{} ", source_glyph(track.source)),
                    Style::default().fg(source_color(track.source)),
                ),
            ];
            spans.extend(track_name_spans(
                &track.artist,
                &track.title,
                is_now_playing(app, track),
                accent,
            ));
            ListItem::new(Line::from(spans)).style(zebra_style(i))
        })
        .collect();

    let list = List::new(items).highlight_style(highlight_style());
    let mut state = ListState::default();
    if active {
        state.select(Some(app.queue_selected));
    }
    frame.render_stateful_widget(list, list_area, &mut state);
}

fn format_duration(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{}:{:02}", minutes, seconds)
}
