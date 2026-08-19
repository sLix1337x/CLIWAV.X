use crate::app::{App, SoundCloudCategory, SoundCloudPane};
use crate::ui::{
    accent_color, draw_section, highlight_style, is_now_playing, muted_style, normal_style,
    player::draw_mini_now_playing, spinner_frame, track_name_spans, zebra_style,
};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    // Below ~60 columns, show only the focused pane instead of crushing both
    // (Tab still switches between them).
    if area.width < 60 {
        match app.soundcloud_pane {
            SoundCloudPane::Categories => draw_categories(frame, app, area),
            SoundCloudPane::Tracks => draw_tracks(frame, app, area),
        }
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(30)])
        .split(area);

    draw_sidebar(frame, app, chunks[0]);
    draw_tracks(frame, app, chunks[1]);
}

/// Narrow left column: the profile/category picker on top (it only ever
/// holds 3 short items, so it doesn't need much room), and what's currently
/// playing — artwork included — filling the rest. Previously this whole
/// column was just the category list with a lot of unused space below it.
fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(10)])
        .split(area);

    draw_categories(frame, app, rows[0]);
    draw_mini_now_playing(frame, app, rows[1]);
}

fn draw_categories(frame: &mut Frame, app: &App, area: Rect) {
    let active = matches!(app.soundcloud_pane, SoundCloudPane::Categories);
    let accent = accent_color(app);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let title = if app.soundcloud_username.is_empty() {
        "SoundCloud".to_string()
    } else {
        app.soundcloud_username.clone()
    };
    let title_color = if active { accent } else { Color::DarkGray };
    frame.render_widget(
        Paragraph::new(Span::styled(
            title,
            Style::default().fg(title_color).add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        chunks[0],
    );
    let list_area = chunks[1];

    // No username configured: show the genre buckets instead of an empty
    // state — each is a live scsearch query, so the tab is useful before
    // any setup (Enter loads one, playback auto-advances through it).
    if app.soundcloud_username.is_empty() {
        let items: Vec<ListItem> = crate::app::SOUNDCLOUD_GENRES
            .iter()
            .map(|genre| {
                ListItem::new(Line::from(vec![
                    Span::styled("  › ", normal_style()),
                    Span::styled(*genre, normal_style()),
                ]))
            })
            .collect();
        let list = List::new(items).highlight_style(highlight_style());
        let mut state = ListState::default();
        if active {
            state.select(Some(app.soundcloud_genre_selected));
        }
        frame.render_stateful_widget(list, list_area, &mut state);
        return;
    }

    // Which category (if any) the currently-playing track belongs to, so it
    // can be marked the same way the playing track itself is marked in the
    // list on the right — only ever one of the three, since only one
    // category's tracks are loaded into `soundcloud_user_tracks` at a time.
    let playing_category = app.current_track.as_ref().and_then(|ct| {
        app.soundcloud_user_tracks
            .iter()
            .any(|t| t.source == ct.source && t.id == ct.id)
            .then_some(SoundCloudCategory::ALL[app.soundcloud_category_selected])
    });

    let items: Vec<ListItem> = SoundCloudCategory::ALL
        .iter()
        .map(|category| {
            let is_playing = Some(*category) == playing_category;
            let (marker, style) = if is_playing {
                ("▶ ", crate::ui::theme::accent_bold(accent))
            } else {
                ("› ", normal_style())
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {marker}"), style),
                Span::styled(category.label(), style),
            ]))
        })
        .collect();

    let list = List::new(items).highlight_style(highlight_style());
    let mut state = ListState::default();
    if active {
        state.select(Some(app.soundcloud_category_selected));
    }
    frame.render_stateful_widget(list, list_area, &mut state);
}

fn draw_tracks(frame: &mut Frame, app: &App, area: Rect) {
    let active = matches!(app.soundcloud_pane, SoundCloudPane::Tracks);
    let category = SoundCloudCategory::ALL[app.soundcloud_category_selected];

    let count_label = if app.soundcloud_loading {
        format!(
            "{} ({}, {} loading)",
            category.label(),
            app.soundcloud_user_tracks.len(),
            spinner_frame(app.tick)
        )
    } else if app.soundcloud_has_more && !app.soundcloud_user_tracks.is_empty() {
        format!("{} ({}, 'm' for more)", category.label(), app.soundcloud_user_tracks.len())
    } else {
        format!("{} ({})", category.label(), app.soundcloud_user_tracks.len())
    };
    let list_area = draw_section(frame, area, &count_label, active);

    if app.soundcloud_user_tracks.is_empty() {
        let hint = if app.soundcloud_username.is_empty() {
            "Enter on a genre to load it, or 'C' to set a username."
        } else if app.soundcloud_loading {
            "Loading"
        } else {
            "Press Enter on a category to load it."
        };
        let para = Paragraph::new(if app.soundcloud_loading {
            format!("{} {}", spinner_frame(app.tick), hint)
        } else {
            hint.to_string()
        })
        .style(muted_style());
        frame.render_widget(para, list_area);
        return;
    }

    let accent = accent_color(app);
    let items: Vec<ListItem> = app
        .soundcloud_user_tracks
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
            ListItem::new(Line::from(spans)).style(zebra_style(i, app.is_dark_bg))
        })
        .collect();

    let list = List::new(items).highlight_style(highlight_style());
    let mut state = ListState::default();
    if active && !app.soundcloud_user_tracks.is_empty() {
        state.select(Some(app.soundcloud_track_selected));
    }
    frame.render_stateful_widget(list, list_area, &mut state);
}

fn format_duration(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{}:{:02}", minutes, seconds)
}
