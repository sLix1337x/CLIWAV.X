pub mod dashboard;
pub mod eq;
pub mod help;
pub mod library;
pub mod player;
pub mod queue;
pub mod search;
pub mod soundcloud;
pub mod theme;
pub mod visualizer;
pub mod waveform;

use crate::app::{App, InputPrompt, PlaybackState, Tab};
use crate::sources::{TrackSource, UnifiedTrack};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Tabs};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // tab bar
            Constraint::Length(1),  // divider
            Constraint::Min(10),   // tab content
            Constraint::Length(1),  // divider
            Constraint::Length(5),  // now-playing hero row
            Constraint::Length(4),  // controls
            Constraint::Length(1),  // status bar
        ])
        .split(frame.area());

    draw_tabs(frame, app, chunks[0]);
    draw_divider(frame, chunks[1]);

    match app.current_tab {
        Tab::Dashboard => dashboard::draw(frame, app, chunks[2]),
        Tab::Search => search::draw(frame, app, chunks[2]),
        Tab::Queue => queue::draw(frame, app, chunks[2]),
        Tab::Library => draw_library(frame, app, chunks[2]),
        Tab::SoundCloud => soundcloud::draw(frame, app, chunks[2]),
        Tab::NowPlaying => player::draw(frame, app, chunks[2]),
        Tab::Eq => eq::draw(frame, app, chunks[2]),
    }

    draw_divider(frame, chunks[3]);
    player::draw_now_playing_line(frame, app, chunks[4]);
    player::draw_controls(frame, app, chunks[5]);
    draw_status(frame, app, chunks[6]);

    if let Some(prompt) = &app.input_prompt {
        draw_prompt(frame, prompt);
    }

    if app.show_help {
        help::draw(frame);
    }
}

/// A full-width dim rule between major screen regions — separates them
/// without the visual weight of another box.
fn draw_divider(frame: &mut Frame, area: Rect) {
    let rule = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(
            rule,
            Style::default().fg(Color::Rgb(48, 54, 64)),
        )),
        area,
    );
}

fn draw_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = ["Dashboard", "Now Playing", "Library", "Queue", "SoundCloud", "EQ", "Search"]
        .iter()
        .map(|t| Line::from(Span::raw(format!(" {t} "))))
        .collect();

    let tabs = Tabs::new(titles)
        .block(
            theme::panel_border(Color::Rgb(60, 90, 110)).title(Line::from(gradient_spans(
                " ♫ CLIWAV.X ",
                (80, 220, 220),
                (190, 130, 255),
                true,
            ))),
        )
        .select(current_tab_index(&app.current_tab))
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled("│", Style::default().fg(Color::Rgb(60, 90, 110))));
    frame.render_widget(tabs, area);
}

/// Braille spinner frames, advanced one per UI tick — animates background
/// work (SoundCloud page fetches, library scans) so the app reads as busy
/// rather than possibly frozen.
const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub fn spinner_frame(tick: u64) -> char {
    SPINNER_FRAMES[(tick as usize) % SPINNER_FRAMES.len()]
}

/// Per-character linear RGB interpolation across `text` — a gradient headline
/// (the "♫ CLIWAV.X" title, the "P L A Y I N G" hero label) reads far more
/// polished than one flat color. No crates needed, just a lerp per character.
pub fn gradient_spans(text: &str, from: (u8, u8, u8), to: (u8, u8, u8), bold: bool) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let last = chars.len().saturating_sub(1).max(1) as f32;
    chars
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let mut style = Style::default().fg(lerp_rgb(from, to, i as f32 / last));
            if bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            Span::styled(c.to_string(), style)
        })
        .collect()
}

/// Volume/level meter as individually-colored spans: the filled portion
/// gradients from a dimmed shade of `accent` up toward a brightened one,
/// instead of the whole bar being one flat color. Unfilled cells stay dim.
pub fn gradient_meter_spans(percent: u8, width: usize, accent: Color) -> Vec<Span<'static>> {
    let base = to_rgb(accent);
    let from = dim_rgb(base, 0.45);
    let to = brighten_rgb(base, 0.3);
    let filled = ((percent.min(100) as usize) * width / 100).min(width);
    let last = filled.saturating_sub(1).max(1) as f32;
    let mut spans: Vec<Span<'static>> = (0..filled)
        .map(|i| {
            Span::styled(
                "█".to_string(),
                Style::default().fg(lerp_rgb(from, to, i as f32 / last)),
            )
        })
        .collect();
    if filled < width {
        spans.push(Span::styled(
            "░".repeat(width - filled),
            Style::default().fg(Color::Rgb(70, 75, 85)),
        ));
    }
    spans
}

/// Interpolates between two colors through HSL space rather than raw RGB.
/// A straight per-channel RGB lerp between two saturated colors (e.g. cyan
/// to magenta) dips through a muddy, desaturated grey partway through and
/// steps unevenly in perceived brightness; going via hue/saturation/
/// lightness keeps the gradient looking like one continuous, evenly-lit
/// color ramp instead. Hue interpolates the short way around the color
/// wheel (never more than 180°), and an achromatic endpoint (grey — where
/// hue is undefined) borrows the other endpoint's hue instead of swinging
/// through an arbitrary one.
pub fn lerp_rgb(from: (u8, u8, u8), to: (u8, u8, u8), t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let (mut h1, s1, l1) = rgb_to_hsl(from);
    let (mut h2, s2, l2) = rgb_to_hsl(to);
    if s1 < 0.001 {
        h1 = h2;
    }
    if s2 < 0.001 {
        h2 = h1;
    }
    let mut dh = h2 - h1;
    if dh > 180.0 {
        dh -= 360.0;
    } else if dh < -180.0 {
        dh += 360.0;
    }
    let h = h1 + dh * t;
    let s = s1 + (s2 - s1) * t;
    let l = l1 + (l2 - l1) * t;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    Color::Rgb(r, g, b)
}

/// RGB (0-255 per channel) to HSL (hue in degrees 0-360, saturation and
/// lightness both 0.0-1.0). Standard conversion, no crate needed for
/// something this small.
fn rgb_to_hsl((r, g, b): (u8, u8, u8)) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let delta = max - min;
    if delta < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };
    let h = if max == rf {
        ((gf - bf) / delta).rem_euclid(6.0)
    } else if max == gf {
        (bf - rf) / delta + 2.0
    } else {
        (rf - gf) / delta + 4.0
    };
    (h * 60.0, s, l)
}

/// HSL back to RGB (0-255 per channel).
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    if s <= 0.0 {
        let v = (l.clamp(0.0, 1.0) * 255.0).round() as u8;
        return (v, v, v);
    }
    let h = h.rem_euclid(360.0) / 360.0;
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue_to_rgb = |p: f32, q: f32, t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    (
        to_byte(hue_to_rgb(p, q, h + 1.0 / 3.0)),
        to_byte(hue_to_rgb(p, q, h)),
        to_byte(hue_to_rgb(p, q, h - 1.0 / 3.0)),
    )
}

/// Mix `rgb` toward white by `amount` (0.0–1.0).
pub fn brighten_rgb(rgb: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let mix = |c: u8| (c as f32 + (255.0 - c as f32) * amount).round() as u8;
    (mix(rgb.0), mix(rgb.1), mix(rgb.2))
}

/// Scale `rgb` toward black by `factor` (0.0–1.0 keeps this much brightness).
pub fn dim_rgb(rgb: (u8, u8, u8), factor: f32) -> (u8, u8, u8) {
    let scale = |c: u8| (c as f32 * factor).round() as u8;
    (scale(rgb.0), scale(rgb.1), scale(rgb.2))
}

/// Approximate RGB for the handful of named ANSI colors we gradient from —
/// exact shades don't matter here, only the hue.
pub fn to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Green => (60, 200, 90),
        Color::LightGreen => (140, 220, 120),
        Color::Yellow => (220, 200, 60),
        Color::Cyan => (80, 200, 220),
        Color::Magenta => (220, 100, 200),
        Color::Red => (220, 70, 60),
        Color::DarkGray => (110, 110, 110),
        _ => (200, 200, 200),
    }
}

/// Per-source accent colors, kept in one place — previously the same match
/// was copy-pasted across four UI files.
pub fn source_color(source: TrackSource) -> Color {
    match source {
        TrackSource::Local => Color::Green,
        TrackSource::YouTube => Color::Red,
        TrackSource::SoundCloud => Color::Magenta,
        TrackSource::Spotify => Color::LightGreen,
    }
}

/// A distinct glyph per source — more at-a-glance recognition than the same
/// colored dot on every row. Kept to single-width glyphs (no emoji) so list
/// columns stay aligned.
pub fn source_glyph(source: TrackSource) -> &'static str {
    match source {
        TrackSource::Local => "♪",
        TrackSource::YouTube => "▶",
        TrackSource::SoundCloud => "☁",
        TrackSource::Spotify => "✳",
    }
}

/// Icon, label, and color for the current playback state — shared by the
/// persistent player bar and the Now Playing tab.
pub fn playback_status(app: &App) -> (&'static str, &'static str, Color) {
    match app.playback_state() {
        PlaybackState::Stopped => ("■", "Stopped", Color::DarkGray),
        PlaybackState::Playing => ("▶", "Playing", Color::Green),
        PlaybackState::Paused => ("⏸", "Paused", Color::Yellow),
    }
}

fn draw_library(frame: &mut Frame, app: &App, area: Rect) {
    library::draw(frame, app, area);
}

/// Renders a one-line accent-bar header ("▎ Title") at the top of `area`
/// (cyan when `active`) and returns the remaining, borderless space below
/// it for content. Used everywhere instead of boxing every single pane.
pub fn draw_section(frame: &mut Frame, area: Rect, title: &str, active: bool) -> Rect {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let (bar_color, text_color) = if active {
        (Color::Cyan, Color::Cyan)
    } else {
        (Color::DarkGray, Color::Gray)
    };
    let header = Line::from(vec![
        Span::styled("▎", Style::default().fg(bar_color).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(" {title}"),
            Style::default().fg(text_color).add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(header), chunks[0]);
    chunks[1]
}

/// A block-character level meter, e.g. `meter_bar(64, 20)` -> "████████████▒░░░░░░░".
pub fn meter_bar(percent: u8, width: usize) -> String {
    let percent = (percent as usize).min(100);
    let filled = (percent * width) / 100;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// The active accent color as RGB: the curated palette when one is selected
/// (`t` key), otherwise the current track's artwork accent, otherwise a
/// neutral steel-blue default while no track/artwork is loaded.
pub fn accent_rgb(app: &App) -> (u8, u8, u8) {
    if let Some(rgb) = app.palette.rgb() {
        return rgb;
    }
    app.artwork_accent.unwrap_or((60, 90, 110))
}

/// The active accent color, shared by every panel and list that tints itself
/// to the current track (Now Playing, SoundCloud sidebar, and the "currently
/// playing" row marker in every track list).
pub fn accent_color(app: &App) -> Color {
    let (r, g, b) = accent_rgb(app);
    Color::Rgb(r, g, b)
}

/// Whether `track` is the one actually loaded in the player right now —
/// independent of list cursor/selection, which is a separate concept.
pub fn is_now_playing(app: &App, track: &UnifiedTrack) -> bool {
    app.current_track
        .as_ref()
        .is_some_and(|t| t.source == track.source && t.id == track.id)
}

/// "Artist — Title" spans styled so they're never the same color: the artist
/// (or uploader/profile name) is muted/gray, the track title is bright and
/// bold. Used by every track list (Search, Queue, Library, SoundCloud).
///
/// When `playing` is true (this is the track currently loaded in the
/// player — see [`is_now_playing`]), both artist and title switch to the
/// accent color instead, with a leading "▶" marker, so the active track is
/// visually distinct from the rest of the list regardless of where the
/// cursor/selection happens to be.
pub fn track_name_spans<'a>(artist: &'a str, title: &'a str, playing: bool, accent: Color) -> Vec<Span<'a>> {
    if playing {
        let style = theme::accent_bold(accent);
        return if artist.is_empty() {
            vec![Span::styled("▶ ", style), Span::styled(title, style)]
        } else {
            vec![
                Span::styled("▶ ", style),
                Span::styled(artist, style),
                Span::styled(" - ", style),
                Span::styled(title, style),
            ]
        };
    }
    if artist.is_empty() {
        return vec![
            Span::raw("  "),
            Span::styled(title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ];
    }
    vec![
        Span::raw("  "),
        Span::styled(artist, muted_style()),
        Span::styled(" - ", muted_style()),
        Span::styled(
            title,
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
    ]
}

/// Subtle alternating-row background so long lists are easier to scan.
pub fn zebra_style(index: usize) -> Style {
    if index % 2 == 1 {
        Style::default().bg(Color::Rgb(22, 24, 30))
    } else {
        Style::default()
    }
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let accent = accent_color(app);
    let busy = app.soundcloud_loading || app.scan_in_progress() || app.search_loading;

    let mut spans = vec![Span::styled(" ", Style::default())];
    if busy {
        spans.push(Span::styled(
            format!("{} ", spinner_frame(app.tick)),
            Style::default().fg(accent),
        ));
    }
    spans.extend([
        Span::styled(&app.status_message, Style::default().fg(Color::White)),
        Span::styled(" │ ", muted_style()),
        Span::styled("Filter: ", muted_style()),
        Span::styled(
            app.search_source_filter.map(|s| s.as_str()).unwrap_or("all"),
            Style::default().fg(Color::Magenta),
        ),
        Span::styled(" │ ", muted_style()),
        Span::styled("Game mode: ", muted_style()),
        Span::styled(
            if app.game_mode { "ON" } else { "OFF" },
            if app.game_mode {
                Style::default().fg(Color::Magenta)
            } else {
                muted_style()
            },
        ),
        Span::styled(" │ ", muted_style()),
        Span::styled("Theme: ", muted_style()),
        Span::styled(app.palette.label(), Style::default().fg(accent)),
        Span::styled(" │ ", muted_style()),
        Span::styled("? for help", muted_style()),
    ]);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn current_tab_index(tab: &Tab) -> usize {
    match tab {
        Tab::Dashboard => 0,
        Tab::NowPlaying => 1,
        Tab::Library => 2,
        Tab::Queue => 3,
        Tab::SoundCloud => 4,
        Tab::Eq => 5,
        Tab::Search => 6,
    }
}

pub fn highlight_style() -> Style {
    Style::default()
        .bg(Color::DarkGray)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

pub fn normal_style() -> Style {
    Style::default().fg(Color::White)
}

pub fn muted_style() -> Style {
    // DarkGray (ANSI "bright black") reads as clearly dimmer than White in
    // essentially every terminal theme. Plain Gray (ANSI 7) is sometimes
    // themed close to White, which was exactly the "artist and title look
    // the same color" complaint.
    Style::default().fg(Color::DarkGray)
}

fn draw_prompt(frame: &mut Frame, prompt: &InputPrompt) {
    let area = frame.area();
    // Clamped to the frame so the popup can never extend past the buffer —
    // previously `area.height / 2 - 2` underflowed on tiny terminals, and an
    // out-of-bounds popup makes ratatui's `Clear` panic outright.
    let width = (area.width / 2).min(area.width);
    let height = 5.min(area.height);
    let popup_area = ratatui::layout::Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };

    frame.render_widget(
        ratatui::widgets::Clear,
        popup_area,
    );

    let block = theme::panel_border(Color::Cyan)
        .title(Span::styled(prompt.title.clone(), theme::accent_bold(Color::Cyan)))
        .style(Style::default().bg(Color::Black).fg(Color::White));
    let para = Paragraph::new(format!("{}▏", prompt.value)).block(block);
    frame.render_widget(para, popup_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_rgb(c: Color) -> (u8, u8, u8) {
        match c {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => panic!("expected Color::Rgb, got {c:?}"),
        }
    }

    #[test]
    fn lerp_rgb_endpoints_are_exact() {
        let from = (10, 20, 30);
        let to = (200, 100, 50);
        assert_eq!(as_rgb(lerp_rgb(from, to, 0.0)), from);
        assert_eq!(as_rgb(lerp_rgb(from, to, 1.0)), to);
    }

    #[test]
    fn lerp_rgb_midpoint_does_not_dip_through_grey() {
        // Cyan -> magenta: a straight RGB lerp passes through a desaturated
        // grey-ish color at t=0.5 (roughly equal R/G/B). Going via HSL
        // should stay clearly saturated instead.
        let cyan = (0u8, 255u8, 255u8);
        let magenta = (255u8, 0u8, 255u8);
        let (r, g, b) = as_rgb(lerp_rgb(cyan, magenta, 0.5));
        let max = r.max(g).max(b) as i32;
        let min = r.min(g).min(b) as i32;
        assert!(
            max - min > 100,
            "expected a clearly saturated midpoint color, got ({r}, {g}, {b})"
        );
    }

    #[test]
    fn lerp_rgb_handles_grey_endpoint_without_panicking() {
        let grey = (128, 128, 128);
        let teal = (32, 178, 170);
        // Just needs to not panic and stay in range; grey has no hue of its
        // own, so this exercises the "borrow the other endpoint's hue" path.
        let _ = lerp_rgb(grey, teal, 0.3);
        let _ = lerp_rgb(teal, grey, 0.7);
    }

    #[test]
    fn hsl_roundtrip_is_close_to_identity() {
        for rgb in [(0, 0, 0), (255, 255, 255), (200, 50, 10), (12, 240, 90), (128, 128, 128)] {
            let (h, s, l) = rgb_to_hsl(rgb);
            let (r, g, b) = hsl_to_rgb(h, s, l);
            let close = |a: u8, b: u8| (a as i32 - b as i32).abs() <= 1;
            assert!(
                close(r, rgb.0) && close(g, rgb.1) && close(b, rgb.2),
                "roundtrip mismatch: {rgb:?} -> hsl({h}, {s}, {l}) -> ({r}, {g}, {b})"
            );
        }
    }
}
