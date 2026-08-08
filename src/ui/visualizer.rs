//! Live spectrum-bar visualizer for the Now Playing tab — occupies the same
//! band as the static waveform timeline, toggled with `v` (see
//! `App::toggle_visualizer`). Reuses the waveform's eighths-block glyph
//! table for sub-row bar-height precision, just growing bars up from the
//! bottom instead of mirrored around a center line.

use crate::app::App;
use crate::audio::spectrum::BAND_COUNT;
use crate::ui::waveform::eighths_glyph;
use crate::ui::{brighten_rgb, dim_rgb, lerp_rgb, to_rgb};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let width = area.width as usize;
    let height = area.height as usize;
    if width == 0 || height == 0 {
        return;
    }

    let accent_rgb = to_rgb(accent);
    let quiet = dim_rgb(accent_rgb, 0.4);
    let loud = brighten_rgb(accent_rgb, 0.25);

    // Each band gets an equal-width slot; a slot wider than 1 column fills
    // solid (a "bar", not a single line) with one blank trailing column as
    // a gap, so bars read as distinct instead of one solid block.
    let slot_width = (width / BAND_COUNT).max(1);
    let bar_width = slot_width.saturating_sub(1).max(1);

    let render_row = |row_from_bottom: usize| -> Line<'static> {
        let mut spans = Vec::with_capacity(BAND_COUNT * 2);
        for band in 0..BAND_COUNT {
            let value = app.visualizer_bands.get(band).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let total_units = (value * height as f32 * 8.0).round() as i32;
            let units = (total_units - row_from_bottom as i32 * 8).clamp(0, 8);
            let glyph = eighths_glyph(units);
            let color = lerp_rgb(quiet, loud, value);
            spans.push(Span::styled(
                glyph.to_string().repeat(bar_width),
                Style::default().fg(color),
            ));
            if slot_width > bar_width {
                spans.push(Span::raw(" ".repeat(slot_width - bar_width)));
            }
        }
        Line::from(spans)
    };

    let lines: Vec<Line<'static>> = (0..height)
        .map(|screen_row| render_row(height - 1 - screen_row))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}
