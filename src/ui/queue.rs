use crate::app::{App, Tab};
use crate::ui::{accent_color, draw_section, highlight_style, is_now_playing, muted_style, source_color, source_glyph, track_name_spans, zebra_style};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let active = matches!(app.current_tab, Tab::Queue);
    let list_area = draw_section(frame, area, &format!("Queue ({})", app.queue.len()), active);

    let accent = accent_color(app);
    let items: Vec<ListItem> = app
        .queue
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let duration = track
                .duration_ms
                .map(format_duration)
                .unwrap_or_else(|| "--:--".to_string());

            let mut spans = vec![
                Span::styled(format!("{:>2}. ", i + 1), muted_style()),
                Span::styled(format!("{} {:<10} ", source_glyph(track.source), track.source.as_str()), Style::default().fg(source_color(track.source))),
            ];
            spans.extend(track_name_spans(
                &track.artist,
                &track.title,
                is_now_playing(app, track),
                accent,
            ));
            spans.push(Span::styled(format!("  ({})", duration), muted_style()));
            ListItem::new(Line::from(spans)).style(zebra_style(i))
        })
        .collect();

    let list = List::new(items).highlight_style(highlight_style());
    let mut state = ListState::default();
    if active && !app.queue.is_empty() {
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
