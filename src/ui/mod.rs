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
            // The tab row's own bottom edge is the rule under it, so unlike
            // the lower region there's no separate divider here.
            Constraint::Length(3),  // tab bar (incl. its connecting rule)
            Constraint::Min(10),   // tab content
            Constraint::Length(1),  // divider
            Constraint::Length(5),  // now-playing hero row
            Constraint::Length(4),  // controls
            Constraint::Length(1),  // status bar
        ])
        .split(frame.area());

    draw_tabs(frame, current_tab_index(&app.current_tab), chunks[0]);

    match app.current_tab {
        Tab::Dashboard => dashboard::draw(frame, app, chunks[1]),
        Tab::Search => search::draw(frame, app, chunks[1]),
        Tab::Queue => queue::draw(frame, app, chunks[1]),
        Tab::Library => draw_library(frame, app, chunks[1]),
        Tab::SoundCloud => soundcloud::draw(frame, app, chunks[1]),
        Tab::NowPlaying => player::draw(frame, app, chunks[1]),
        Tab::Eq => eq::draw(frame, app, chunks[1]),
    }

    draw_divider(frame, chunks[2]);
    player::draw_now_playing_line(frame, app, chunks[3]);
    player::draw_controls(frame, app, chunks[4]);
    draw_status(frame, app, chunks[5]);

    if let Some(prompt) = &app.input_prompt {
        draw_prompt(frame, prompt, app.is_dark_bg);
    }

    if app.show_help {
        help::draw(frame, app.is_dark_bg);
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

/// Tab labels, in the same order `current_tab_index` numbers them.
const TAB_LABELS: [&str; 7] = [
    "Dashboard",
    "Now Playing",
    "Library",
    "Queue",
    "SoundCloud",
    "EQ",
    "Search",
];

const TAB_BORDER_DIM: Color = Color::Rgb(60, 90, 110);
const TAB_TITLE: &str = " ♫ CLIWAV.X ";

/// Draws the tab row as individual boxes whose active member breaks open at
/// the bottom, so the selected tab and the content beneath it read as one
/// connected surface instead of two stacked boxes.
///
/// Hand-drawn rather than built on ratatui's `Tabs`: that widget renders one
/// continuous strip inside a single shared block, which can't express a
/// per-tab box or a bottom edge that opens under only the active tab.
///
/// The bottom line doubles as the rule separating the tab row from the
/// content, which is why `draw` no longer emits a separate divider here —
/// two full-width rules stacked would just read as a thick smear.
fn draw_tabs(frame: &mut Frame, selected: usize, area: Rect) {
    // Each box is "│ Label │": the label with one space of padding per side,
    // between two border columns.
    let inner: Vec<usize> = TAB_LABELS.iter().map(|l| l.chars().count() + 2).collect();
    let boxes_width: usize = inner.iter().map(|w| w + 2).sum();

    // Below three rows there's nowhere to put a box, and past the terminal's
    // width the boxes would wrap into garbage — fall back to the plain strip
    // rather than rendering something broken.
    if area.height < 3 || boxes_width > area.width as usize {
        draw_tabs_compact(frame, selected, area);
        return;
    }

    let dim = Style::default().fg(TAB_BORDER_DIM);
    let lit = Style::default().fg(SELECTION);

    let mut tops: Vec<Span> = Vec::new();
    let mut labels: Vec<Span> = Vec::new();
    let mut bottoms: Vec<Span> = Vec::new();

    for (i, label) in TAB_LABELS.iter().enumerate() {
        let active = i == selected;
        let edge = if active { lit } else { dim };
        let width = inner[i];

        tops.push(Span::styled("╭", edge));
        tops.push(Span::styled("─".repeat(width), edge));
        tops.push(Span::styled("╮", edge));

        labels.push(Span::styled("│", edge));
        labels.push(Span::styled(
            format!(" {label} "),
            if active {
                Style::default().fg(SELECTION).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            },
        ));
        labels.push(Span::styled("│", edge));

        if active {
            // Corners turn outward and the span between them is left blank:
            // that gap is what makes the tab look open to the content below.
            bottoms.push(Span::styled("╯", edge));
            bottoms.push(Span::styled(" ".repeat(width), edge));
            bottoms.push(Span::styled("╰", edge));
        } else {
            bottoms.push(Span::styled("┴", dim));
            bottoms.push(Span::styled("─".repeat(width), dim));
            bottoms.push(Span::styled("┴", dim));
        }
    }

    // Whatever's left of the row carries the rule onward, picking up the
    // wordmark when there's room for it rather than truncating it.
    let remaining = area.width as usize - boxes_width;
    let title_len = TAB_TITLE.chars().count();
    if remaining >= title_len + 3 {
        bottoms.push(Span::styled("──", dim));
        bottoms.extend(gradient_spans(TAB_TITLE, (80, 220, 220), (190, 130, 255), true));
        bottoms.push(Span::styled("─".repeat(remaining - title_len - 2), dim));
    } else {
        bottoms.push(Span::styled("─".repeat(remaining), dim));
    }

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(tops),
            Line::from(labels),
            Line::from(bottoms),
        ]),
        area,
    );
}

/// The pre-existing single-strip tab bar, kept for terminals too small to
/// fit the boxed row.
fn draw_tabs_compact(frame: &mut Frame, selected: usize, area: Rect) {
    let titles: Vec<Line> = TAB_LABELS
        .iter()
        .map(|t| Line::from(Span::raw(format!(" {t} "))))
        .collect();

    let tabs = Tabs::new(titles)
        .select(selected)
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(SELECTION)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled("│", Style::default().fg(TAB_BORDER_DIM)));
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

/// Renders a one-line accent-bar header ("▊ Title") at the top of `area`
/// (cyan when `active`) and returns the remaining, borderless space below
/// it for content. Used everywhere instead of boxing every single pane.
///
/// The bar carries the focus state in two channels at once: color, and the
/// glyph's own width. Color alone was doing the job badly — cyan-vs-gray is
/// a weak signal at a glance across a busy screen, and it disappears
/// entirely for a colorblind user or on a low-contrast terminal theme.
/// Stepping the glyph from a thin ▎ to a solid ▊ makes "which pane has the
/// keyboard" readable from the shape alone.
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
    // Both are single-cell glyphs from the same block-element run, so the
    // header's layout is identical either way — only the ink changes.
    let bar = if active { "▊" } else { "▎" };
    let header = Line::from(vec![
        Span::styled(bar, Style::default().fg(bar_color).add_modifier(Modifier::BOLD)),
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

/// Subtle alternating-row background so long lists are easier to scan. The
/// striped row steps away from the terminal's own background in whichever
/// direction is actually visible: darker on a dark terminal, lighter on a
/// light one (a near-black stripe on a white terminal is a harsh black bar,
/// not a subtle stripe).
pub fn zebra_style(index: usize, is_dark: bool) -> Style {
    if index % 2 == 1 {
        let (r, g, b) = theme::surface_base(is_dark, (22, 24, 30), (232, 233, 238));
        Style::default().bg(Color::Rgb(r, g, b))
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

/// The one color that means "this is what your keys act on right now" —
/// the selected list row and the selected tab both use it. Deliberately a
/// hue nothing else in the UI owns: source tags, playback state and the
/// cover-derived accent all shift around, so a selection tinted from any of
/// those would keep colliding with them.
pub const SELECTION: Color = Color::Rgb(0xEE, 0xBF, 0x02);

/// "This playback mode is engaged" — repeat and shuffle when they're on.
///
/// A real RGB violet rather than ANSI `Magenta`, which most terminal themes
/// render pink-red rather than purple. It also stops these from colliding
/// with the SoundCloud source tag, which is ANSI `Magenta` and can sit a few
/// columns away in the same row.
pub const MODE_ON: Color = Color::Rgb(190, 130, 255);

pub fn highlight_style() -> Style {
    Style::default()
        .bg(Color::DarkGray)
        .fg(SELECTION)
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

fn draw_prompt(frame: &mut Frame, prompt: &InputPrompt, is_dark: bool) {
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

    // An opaque fill is the point here — the popup sits over live content and
    // has to hide it — so this one flips fully rather than stepping subtly.
    let (bg, fg) = if is_dark {
        (Color::Black, Color::White)
    } else {
        (Color::White, Color::Black)
    };
    let block = theme::panel_border(Color::Cyan)
        .title(Span::styled(prompt.title.clone(), theme::accent_bold(Color::Cyan)))
        .style(Style::default().bg(bg).fg(fg));
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

    /// Renders one `draw_section` header and returns (glyph, its color).
    fn render_section_bar(active: bool) -> (String, Color) {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
        terminal
            .draw(|f| {
                draw_section(f, f.area(), "Playlists", active);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let cell = buffer.cell((0, 0)).expect("header cell must exist");
        (cell.symbol().to_string(), cell.style().fg.unwrap())
    }

    /// Renders the tab row and returns its rows as plain strings.
    fn render_tab_rows(selected: usize, width: u16, height: u16) -> Vec<String> {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| draw_tabs(f, selected, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn active_tab_opens_downward_and_others_close() {
        let rows = render_tab_rows(0, 100, 3);
        let bottom = &rows[2];

        // Dashboard is first, so its box occupies the leading columns and its
        // bottom edge must be the open form.
        assert!(
            bottom.starts_with("╯           ╰"),
            "active tab must open downward, got: {bottom}"
        );
        // A closed neighbour still ties into the rule with ┴ corners.
        assert!(bottom.contains('┴'), "inactive tabs must stay closed");
        assert!(
            !bottom.contains("╯           ╰┴─────────────╰"),
            "only one tab may be open at a time"
        );
    }

    #[test]
    fn every_tab_can_be_the_open_one() {
        for selected in 0..TAB_LABELS.len() {
            let rows = render_tab_rows(selected, 100, 3);
            assert_eq!(
                rows[2].matches('╯').count(),
                1,
                "exactly one tab opens (selected={selected})"
            );
            assert_eq!(rows[2].matches('╰').count(), 1);
        }
    }

    #[test]
    fn narrow_terminal_falls_back_instead_of_wrapping() {
        // 40 columns cannot hold the boxed row; the compact strip must take
        // over rather than the boxes spilling into a second line of garbage.
        let rows = render_tab_rows(0, 40, 3);
        assert!(
            !rows[0].contains('╭'),
            "boxes must not render when they don't fit: {}",
            rows[0]
        );
    }

    #[test]
    fn active_section_bar_is_heavier_than_inactive() {
        let (active_bar, active_fg) = render_section_bar(true);
        let (inactive_bar, inactive_fg) = render_section_bar(false);

        // The focus signal must survive with color alone stripped out —
        // that's the whole point of stepping the glyph too.
        assert_ne!(
            active_bar, inactive_bar,
            "active and inactive panes must differ by glyph, not only by color"
        );
        assert_eq!(active_bar, "▊");
        assert_eq!(inactive_bar, "▎");
        assert_ne!(active_fg, inactive_fg);
    }

    #[test]
    fn section_bar_occupies_one_cell_in_both_states() {
        // A wider glyph here would shift the title and make the header jitter
        // as focus moves between panes.
        for active in [true, false] {
            let (bar, _) = render_section_bar(active);
            assert_eq!(
                bar.chars().count(),
                1,
                "bar must stay a single cell (active={active})"
            );
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
