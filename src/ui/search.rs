use crate::app::{App, SearchFocus, Tab};
use crate::ui::{accent_color, draw_section, highlight_style, is_now_playing, muted_style, source_color, source_glyph, track_name_spans, zebra_style};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = ratatui::layout::Layout::default()
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let input_focused = matches!(app.search_focus, SearchFocus::Input);
    let filter_text = app
        .search_source_filter
        .map(|s| format!(" [{}]", s.as_str()))
        .unwrap_or_default();
    let header = if input_focused {
        format!("Search{} (Enter to search)", filter_text)
    } else {
        format!("Search{} (Tab to type)", filter_text)
    };

    let query_text = if input_focused {
        format!("{}▏", app.search_query)
    } else {
        app.search_query.clone()
    };
    let input = Paragraph::new(query_text)
        .style(Style::default().fg(Color::Yellow))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(if input_focused {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::Gray)
                })
                .title(header),
        );
    frame.render_widget(input, chunks[0]);

    let results_area = draw_section(
        frame,
        chunks[1],
        &format!("Results ({})", app.search_results.len()),
        !input_focused,
    );

    let accent = accent_color(app);
    let items: Vec<ListItem> = app
        .search_results
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
            spans.push(Span::styled(
                format!("  ({})  {}", duration, track.album),
                muted_style(),
            ));
            ListItem::new(Line::from(spans)).style(zebra_style(i))
        })
        .collect();

    let list = List::new(items).highlight_style(highlight_style());
    let mut state = ListState::default();
    if matches!(app.current_tab, Tab::Search) && !app.search_results.is_empty() {
        state.select(Some(app.search_selected));
    }
    frame.render_stateful_widget(list, results_area, &mut state);
}

fn format_duration(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{}:{:02}", minutes, seconds)
}
