//! Live audio-reactive background effect for the Now Playing tab — rendered
//! first, behind the artwork/title/artist card and everything else in the
//! panel (toggled/cycled with `v`, see `App::cycle_visualizer`). Two
//! distinct modes rather than one combined effect — showing both together
//! made neither clearly legible:
//!
//! - **Rain**: one falling trail per terminal column, speed and brightness
//!   driven by whichever frequency band that column maps to.
//! - **Particles**: rising sparks spawned from the bottom in proportion to
//!   bass energy, drifting upward and fading out.
//!
//! Both are colored by a warm(bass)-to-cool(treble) sweep through the
//! current accent color, so the whole effect reads as one continuous
//! frequency-mapped gradient rather than a flat single-color display.
//!
//! Rendering needs to *advance* a small simulation each frame (rain head
//! positions, particle physics) but every `ui::*::draw` function only gets
//! `&App` — the same situation `artwork_cache` solves, and the same fix:
//! the simulation state lives behind a `RefCell` (`App::visualizer_fx`).
//! Both sub-simulations keep advancing regardless of which mode is
//! currently *rendered*, so switching modes never shows a stale, frozen
//! snapshot — only which one gets drawn to the grid changes.

use crate::app::{App, VisualizerFx, VisualizerMode, VizParticle};
use crate::audio::spectrum::BAND_COUNT;
use crate::ui::{brighten_rgb, dim_rgb, lerp_rgb, to_rgb};
use rand::Rng;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Rows of fading trail behind each rain drop's leading (brightest) cell.
const RAIN_TRAIL_LEN: u16 = 8;
/// Baseline fall speed (rows/frame) for a column whose band is silent.
const RAIN_SPEED_MIN: f32 = 0.10;
/// Extra fall speed at full band loudness, added to the baseline.
const RAIN_SPEED_SCALE: f32 = 0.55;

const MAX_PARTICLES: usize = 120;
/// Particles spawned per frame at full bass energy (scaled down at lower
/// energy, and never spawning at all below `SPAWN_THRESHOLD`).
const PARTICLE_SPAWN_SCALE: f32 = 4.0;
const PARTICLE_SPAWN_THRESHOLD: f32 = 0.08;
const PARTICLE_SPEED_MIN: f32 = 0.28;
const PARTICLE_SPEED_RANGE: f32 = 0.55;
/// Life lost per frame (1.0 -> 0.0); ~50 frames at the visualizer's ~30fps
/// tick rate, so a particle lives a little over a second.
const PARTICLE_LIFE_DECAY: f32 = 0.020;

const RAIN_CHARS: &[char] = &['0', '1', '7', '$', '%', '&', '*', '+', '/', '\\', '|', '~', '^', ':', ';'];

pub fn draw(frame: &mut Frame, app: &App, area: Rect, accent: Color, mode: VisualizerMode) {
    let width = area.width;
    let height = area.height;
    if width == 0 || height == 0 {
        return;
    }

    {
        let mut fx = app.visualizer_fx.borrow_mut();
        advance(&mut fx, &app.visualizer_bands, width, height);
    }
    let fx = app.visualizer_fx.borrow();
    let accent_rgb = to_rgb(accent);

    let mut grid: Vec<Vec<Option<(char, Color)>>> =
        vec![vec![None; width as usize]; height as usize];

    match mode {
        VisualizerMode::Rain => {
            for (col, head) in fx.rain.iter().enumerate() {
                let band_idx = (col * BAND_COUNT / width as usize).min(BAND_COUNT - 1);
                let color_t = band_idx as f32 / (BAND_COUNT - 1) as f32;
                let base = band_color(color_t, accent_rgb);
                for offset in 0..RAIN_TRAIL_LEN {
                    let row_f = head - offset as f32;
                    if row_f < 0.0 || row_f as usize >= height as usize {
                        continue;
                    }
                    let row = row_f as usize;
                    // Brightest at the head (offset 0), fading toward the
                    // tail — never fully to black, so the trail stays
                    // faintly visible.
                    let brightness = (1.0 - offset as f32 / RAIN_TRAIL_LEN as f32).max(0.12);
                    let (r, g, b) = dim_rgb(base, brightness);
                    grid[row][col] = Some((char_for_cell(col, row), Color::Rgb(r, g, b)));
                }
            }
        }
        VisualizerMode::Particles => {
            for p in fx.particles.iter() {
                if p.x < 0.0 || p.y < 0.0 {
                    continue;
                }
                let (col, row) = (p.x as usize, p.y as usize);
                if col >= width as usize || row >= height as usize {
                    continue;
                }
                let base = band_color(p.hue_t, accent_rgb);
                let (r, g, b) = brighten_rgb(base, (p.life * 0.4).clamp(0.0, 0.4));
                let glyph = if p.life > 0.66 {
                    '*'
                } else if p.life > 0.33 {
                    '+'
                } else {
                    '.'
                };
                grid[row][col] = Some((glyph, Color::Rgb(r, g, b)));
            }
        }
    }

    let lines: Vec<Line<'static>> = grid.into_iter().map(row_to_line).collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn row_to_line(row: Vec<Option<(char, Color)>>) -> Line<'static> {
    let mut spans = Vec::new();
    let mut blank_run = 0usize;
    for cell in row {
        match cell {
            None => blank_run += 1,
            Some((ch, color)) => {
                if blank_run > 0 {
                    spans.push(Span::raw(" ".repeat(blank_run)));
                    blank_run = 0;
                }
                spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
            }
        }
    }
    if blank_run > 0 {
        spans.push(Span::raw(" ".repeat(blank_run)));
    }
    Line::from(spans)
}

/// Advances the rain heads and particle physics by one frame, reseeding
/// `rain` on first use or after a terminal resize (the column count changed,
/// so the old positions no longer line up with anything).
fn advance(fx: &mut VisualizerFx, bands: &[f32; BAND_COUNT], width: u16, height: u16) {
    let mut rng = rand::thread_rng();

    if fx.size != (width, height) || fx.rain.len() != width as usize {
        // Staggered negative start heights so every column doesn't begin
        // its first drop in lockstep at row 0.
        fx.rain = (0..width)
            .map(|_| -(rng.gen_range(0..height.max(1) as i32)) as f32)
            .collect();
        fx.size = (width, height);
    }

    for (col, head) in fx.rain.iter_mut().enumerate() {
        let band_idx = (col * BAND_COUNT / width as usize).min(BAND_COUNT - 1);
        let band = bands[band_idx].clamp(0.0, 1.0);
        *head += RAIN_SPEED_MIN + band * RAIN_SPEED_SCALE;
        if *head - RAIN_TRAIL_LEN as f32 > height as f32 {
            *head = -(rng.gen_range(0..height.max(1) as i32)) as f32;
        }
    }

    // Bass energy (the lowest quarter of bands) drives particle spawn rate
    // — sparks rising on the beat, not on treble hi-hats.
    let bass_bins = (BAND_COUNT / 4).max(1);
    let bass_energy: f32 = bands[..bass_bins].iter().sum::<f32>() / bass_bins as f32;
    if bass_energy > PARTICLE_SPAWN_THRESHOLD {
        let spawn_count = (bass_energy.clamp(0.0, 1.0) * PARTICLE_SPAWN_SCALE) as usize;
        for _ in 0..spawn_count {
            if fx.particles.len() >= MAX_PARTICLES {
                break;
            }
            fx.particles.push(VizParticle {
                x: rng.gen_range(0.0..width.max(1) as f32),
                y: (height as f32 - 1.0).max(0.0),
                vy: -(PARTICLE_SPEED_MIN + rng.gen_range(0.0..1.0f32) * PARTICLE_SPEED_RANGE),
                life: 1.0,
                hue_t: rng.gen_range(0.0..1.0f32),
            });
        }
    }

    for p in fx.particles.iter_mut() {
        p.y += p.vy;
        p.life -= PARTICLE_LIFE_DECAY;
    }
    fx.particles.retain(|p| p.life > 0.0 && p.y >= 0.0);
}

/// Warm-to-cool color sweep through the current accent color: bass (t=0)
/// leans toward an ember orange, treble (t=1) toward an electric blue, with
/// the accent color itself sitting at the midpoint — ties the effect's
/// palette back to the app's theme instead of a fixed, theme-independent
/// rainbow.
fn band_color(t: f32, accent_rgb: (u8, u8, u8)) -> (u8, u8, u8) {
    const WARM: (u8, u8, u8) = (255, 110, 60);
    const COOL: (u8, u8, u8) = (80, 160, 255);
    let t = t.clamp(0.0, 1.0);
    let color = if t < 0.5 {
        lerp_rgb(WARM, accent_rgb, t * 2.0)
    } else {
        lerp_rgb(accent_rgb, COOL, (t - 0.5) * 2.0)
    };
    to_rgb(color)
}

/// Deterministic per-cell character pick (not per-frame-random) so a given
/// screen position keeps the same glyph as the rain head passes over and
/// past it, instead of flickering to a new random character every redraw.
fn char_for_cell(col: usize, row: usize) -> char {
    let idx = (col.wrapping_mul(31).wrapping_add(row.wrapping_mul(17))) % RAIN_CHARS.len();
    RAIN_CHARS[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_seeds_one_rain_head_per_column() {
        let mut fx = VisualizerFx::default();
        let bands = [0.0f32; BAND_COUNT];
        advance(&mut fx, &bands, 40, 10);
        assert_eq!(fx.rain.len(), 40);
        assert_eq!(fx.size, (40, 10));
    }

    #[test]
    fn advance_reseeds_rain_on_resize() {
        let mut fx = VisualizerFx::default();
        let bands = [0.0f32; BAND_COUNT];
        advance(&mut fx, &bands, 40, 10);
        advance(&mut fx, &bands, 25, 8);
        assert_eq!(fx.rain.len(), 25);
        assert_eq!(fx.size, (25, 8));
    }

    #[test]
    fn loud_band_makes_its_column_fall_faster_than_a_silent_one() {
        let width = BAND_COUNT as u16;
        let mut fx = VisualizerFx::default();
        // Pre-seed at a matching size/length so `advance` skips the random
        // reseed path entirely — otherwise each column's independently
        // randomized starting offset (up to `height` rows) would swamp the
        // one-frame speed difference this test is actually checking.
        fx.rain = vec![0.0; width as usize];
        fx.size = (width, 20);
        let mut bands = [0.0f32; BAND_COUNT];
        bands[0] = 1.0; // loudest possible for column 0's band
        advance(&mut fx, &bands, width, 20);
        let loud_head = fx.rain[0];
        let quiet_head = fx.rain[BAND_COUNT - 1];
        assert!(
            loud_head > quiet_head,
            "loud column ({loud_head}) should have advanced further than quiet column ({quiet_head})"
        );
    }

    #[test]
    fn silence_spawns_no_particles() {
        let mut fx = VisualizerFx::default();
        let bands = [0.0f32; BAND_COUNT];
        for _ in 0..10 {
            advance(&mut fx, &bands, 40, 10);
        }
        assert!(fx.particles.is_empty());
    }

    #[test]
    fn loud_bass_spawns_particles() {
        let mut fx = VisualizerFx::default();
        let bands = [1.0f32; BAND_COUNT];
        advance(&mut fx, &bands, 40, 10);
        assert!(!fx.particles.is_empty());
    }

    #[test]
    fn particles_rise_and_eventually_die() {
        let mut fx = VisualizerFx::default();
        fx.particles.push(VizParticle { x: 5.0, y: 9.0, vy: -1.0, life: 1.0, hue_t: 0.5 });
        let silent = [0.0f32; BAND_COUNT];
        for _ in 0..50 {
            advance(&mut fx, &silent, 40, 10);
        }
        assert!(fx.particles.is_empty(), "a particle should fade out and be removed over time");
    }

    #[test]
    fn band_color_is_warm_at_bass_and_cool_at_treble() {
        let accent = (150, 150, 150);
        let bass = band_color(0.0, accent);
        let treble = band_color(1.0, accent);
        // Warm: red channel clearly higher than blue. Cool: the reverse.
        assert!(bass.0 as i32 - bass.2 as i32 > 50, "bass color {bass:?} should be warm");
        assert!(treble.2 as i32 - treble.0 as i32 > 50, "treble color {treble:?} should be cool");
    }

    #[test]
    fn char_for_cell_is_deterministic_and_varies() {
        assert_eq!(char_for_cell(3, 7), char_for_cell(3, 7));
        let distinct: std::collections::HashSet<char> =
            (0..20).map(|i| char_for_cell(i, i * 2)).collect();
        assert!(distinct.len() > 1, "expected varied glyphs across cells, got {distinct:?}");
    }
}
