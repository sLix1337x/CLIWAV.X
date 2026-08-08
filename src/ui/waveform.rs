//! Mirrored bar-chart waveform for the Now Playing timeline — bars grow
//! from a center line both up and down using eighth-block Unicode
//! characters for sub-row precision, colored played (before the current
//! position) vs. unplayed (after it), the way SoundCloud's and foobar2000's
//! seekbars read.

use crate::ui::{dim_rgb, to_rgb};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

const EIGHTHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn eighths_glyph(units: i32) -> char {
    EIGHTHS[units.clamp(0, 8) as usize]
}

/// Renders `samples` (any length — typically `crate::audio::WAVEFORM_BUCKETS`)
/// resampled to exactly `width` columns and `height` rows. `progress_ratio`
/// (0.0–1.0) splits columns into played (`accent`) and unplayed (dimmed).
///
/// Known simplification: partial-fill glyphs always fill from the bottom of
/// their cell (the only direction Unicode's block-element set supports at
/// eighth-row precision). For the half below center that means a partially
/// filled row's visible fill can sit a notch away from the center line
/// instead of flush against it — a minor cosmetic gap, not a data error,
/// and the standard tradeoff every terminal waveform/VU renderer makes.
pub fn render(
    samples: &[f32],
    width: u16,
    height: u16,
    progress_ratio: f32,
    accent: Color,
) -> Vec<Line<'static>> {
    let width = width as usize;
    let height = height as usize;
    if width == 0 || height == 0 || samples.is_empty() {
        return Vec::new();
    }

    let top_half = height / 2;
    let bottom_half = height - top_half;

    let accent_rgb = to_rgb(accent);
    let played_color = Color::Rgb(accent_rgb.0, accent_rgb.1, accent_rgb.2);
    let (ur, ug, ub) = dim_rgb(accent_rgb, 0.35);
    let unplayed_color = Color::Rgb(ur, ug, ub);

    let playhead_col = ((progress_ratio.clamp(0.0, 1.0) * width as f32) as usize).min(width);

    // Nearest-neighbor resample — simple and correct whether up- or
    // down-sampling from the fixed bucket count to the actual column count.
    let column_amp = |col: usize| -> f32 {
        let idx = (col * samples.len()) / width.max(1);
        samples[idx.min(samples.len() - 1)]
    };

    // Eighth-units for one row of one half, where `row_from_center` 0 is the
    // row closest to center — filled first, so a bar always grows outward
    // from the middle rather than from the widget's outer edge.
    let units_for = |amp: f32, half_rows: usize, row_from_center: usize| -> i32 {
        let total = (amp.clamp(0.0, 1.0) * half_rows as f32 * 8.0).round() as i32;
        (total - row_from_center as i32 * 8).clamp(0, 8)
    };

    let render_row = |row_from_center: usize, half_rows: usize| -> Line<'static> {
        let mut played = String::with_capacity(playhead_col);
        let mut unplayed = String::with_capacity(width.saturating_sub(playhead_col));
        for col in 0..width {
            let glyph = eighths_glyph(units_for(column_amp(col), half_rows, row_from_center));
            if col < playhead_col {
                played.push(glyph);
            } else {
                unplayed.push(glyph);
            }
        }
        Line::from(vec![
            Span::styled(played, Style::default().fg(played_color)),
            Span::styled(unplayed, Style::default().fg(unplayed_color)),
        ])
    };

    let mut lines = Vec::with_capacity(height);
    // Top half: screen-topmost row is furthest from center, so walk
    // `row_from_center` in reverse as we go down toward the middle.
    for screen_row in 0..top_half {
        lines.push(render_row(top_half - 1 - screen_row, top_half));
    }
    // Bottom half: closest-to-center row first, growing outward.
    for row_from_center in 0..bottom_half {
        lines.push(render_row(row_from_center, bottom_half));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_text(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect()
    }

    #[test]
    fn renders_requested_row_and_column_count() {
        let samples = vec![0.5f32; 200];
        let lines = render(&samples, 10, 4, 0.5, Color::Cyan);
        assert_eq!(lines.len(), 4);
        for row in plain_text(&lines) {
            assert_eq!(row.chars().count(), 10);
        }
    }

    #[test]
    fn silence_renders_blank() {
        let samples = vec![0.0f32; 200];
        let lines = render(&samples, 8, 4, 0.5, Color::Cyan);
        for row in plain_text(&lines) {
            assert!(row.chars().all(|c| c == ' '), "expected blank row, got {row:?}");
        }
    }

    #[test]
    fn full_amplitude_fills_every_row() {
        let samples = vec![1.0f32; 200];
        let lines = render(&samples, 8, 4, 0.5, Color::Cyan);
        for row in plain_text(&lines) {
            assert!(row.chars().all(|c| c == '█'), "expected full block row, got {row:?}");
        }
    }

    #[test]
    fn progress_ratio_splits_played_and_unplayed_spans() {
        let samples = vec![1.0f32; 200];
        let lines = render(&samples, 10, 2, 0.3, Color::Cyan);
        let row = &lines[0];
        // Two spans: the played (accent) run, then the unplayed (dimmed) run.
        assert_eq!(row.spans.len(), 2);
        assert_eq!(row.spans[0].content.chars().count(), 3);
        assert_eq!(row.spans[1].content.chars().count(), 7);
    }

    #[test]
    fn zero_size_is_empty_not_panicking() {
        assert!(render(&[0.5], 0, 4, 0.5, Color::Cyan).is_empty());
        assert!(render(&[0.5], 10, 0, 0.5, Color::Cyan).is_empty());
        assert!(render(&[], 10, 4, 0.5, Color::Cyan).is_empty());
    }
}
