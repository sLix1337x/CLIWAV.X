use crate::app::App;
use crate::player::eq;
use crate::ui::{accent_color, draw_section, muted_style};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Bar, BarChart, BarGroup, Paragraph};
use ratatui::Frame;

/// Bar values are gain shifted into 0..=(2*MAX_GAIN_DB) and scaled by 10 for
/// finer bar-height resolution than whole-dB steps would give — `BarChart`
/// only takes non-negative integers.
const SCALE: u64 = 10;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(2)])
        .split(area);

    let content = draw_section(frame, chunks[0], &format!("Equalizer — {}", app.eq_preset), true);
    let accent = accent_color(app);
    let dim = muted_style().fg.unwrap_or(ratatui::style::Color::DarkGray);

    let bars: Vec<Bar> = eq::FREQUENCIES
        .iter()
        .zip(app.eq_gains.iter())
        .enumerate()
        .map(|(i, (freq, gain))| {
            let value = ((gain + eq::MAX_GAIN_DB) * SCALE as f64).round() as u64;
            let color = if i == app.eq_selected { accent } else { dim };
            let style = if i == app.eq_selected {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            Bar::default()
                .value(value)
                .label(Line::from(freq_label(*freq)))
                .text_value(format!("{gain:+.1}"))
                .style(style)
                .value_style(Style::default().fg(ratatui::style::Color::Black).bg(color))
        })
        .collect();

    let max = ((eq::MAX_GAIN_DB * 2.0) * SCALE as f64).round() as u64;
    let chart = BarChart::default()
        .data(BarGroup::default().bars(&bars))
        .bar_width(content.width / eq::BAND_COUNT as u16 - 1)
        .bar_gap(1)
        .max(max);
    frame.render_widget(chart, content);

    let help = Line::from(vec![
        Span::styled("←/→", Style::default().fg(accent)),
        Span::styled(" select band  ", muted_style()),
        Span::styled("↑/↓", Style::default().fg(accent)),
        Span::styled(" adjust gain  ", muted_style()),
        Span::styled("[ / ]", Style::default().fg(accent)),
        Span::styled(" preset", muted_style()),
    ]);
    frame.render_widget(Paragraph::new(help), chunks[1]);
}

fn freq_label(freq: u32) -> String {
    if freq >= 1000 {
        format!("{}k", freq / 1000)
    } else {
        freq.to_string()
    }
}
