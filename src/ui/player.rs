use crate::app::{App, LoopMode};
use crate::ui::{
    accent_color, brighten_rgb, draw_section, gradient_meter_spans, gradient_spans, muted_style,
    playback_status, source_color, source_glyph, to_rgb,
};
use image::DynamicImage;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};
use ratatui::Frame;
use ratatui_image::{Image, Resize};

pub fn format_db(db: f64) -> String {
    if db.is_infinite() {
        "-inf dB".to_string()
    } else {
        format!("{:+.1} dB", db)
    }
}

/// "2:34" from seconds; "--:--" for unknown/unavailable durations.
fn format_time(secs: f64) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return "--:--".to_string();
    }
    let total = secs as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

/// A near-black panel background, only faintly tinted toward the active
/// accent color when one is available — reads as a low-opacity overlay over
/// the terminal's own background rather than a solid, saturated fill.
fn accent_bg(app: &App) -> Color {
    const BASE: (u16, u16, u16) = (24, 24, 27);
    let Some((r, g, b)) = app.palette.rgb().or(app.artwork_accent) else {
        return Color::Rgb(BASE.0 as u8, BASE.1 as u8, BASE.2 as u8);
    };
    // ~12% accent / 88% near-black base.
    let mix = |base: u16, accent: u8| ((base * 7 + accent as u16) / 8) as u8;
    Color::Rgb(mix(BASE.0, r), mix(BASE.1, g), mix(BASE.2, b))
}

/// Spaces out a short label's letters for a bit of visual weight in place of
/// real font-size control, which terminals don't offer — "PLAYING" becomes
/// "P L A Y I N G". Only used for short state words, not full titles.
fn letter_spaced(s: &str) -> String {
    s.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let content_area = draw_section(frame, area, "Now Playing", true);

    if app.current_track.is_none() {
        let (icon, _, state_color) = playback_status(app);
        let para = Paragraph::new(vec![
            Line::default(),
            Line::from(Span::styled("♫", Style::default().fg(Color::DarkGray))),
            Line::from(Span::styled(
                format!("{icon} Nothing playing."),
                Style::default().fg(state_color),
            )),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(para, content_area);
        return;
    }

    let track = app.current_track.as_ref().unwrap();
    let accent = accent_color(app);
    let (icon, label, state_color) = playback_status(app);

    // Gradient state headline ("▶  P L A Y I N G"), same gradient treatment
    // as the persistent hero row.
    let state_base = to_rgb(state_color);
    let mut state_spans = vec![Span::styled(
        format!("{icon}  "),
        Style::default().fg(state_color).add_modifier(Modifier::BOLD),
    )];
    state_spans.extend(gradient_spans(
        &letter_spaced(&label.to_uppercase()),
        state_base,
        brighten_rgb(state_base, 0.5),
        true,
    ));

    let mut volume_spans = vec![Span::styled("Vol ", muted_style())];
    volume_spans.extend(gradient_meter_spans(app.volume, 24, accent));
    volume_spans.push(Span::styled(
        format!("  {}%  ({})", app.volume, format_db(app.volume_db())),
        Style::default().fg(accent),
    ));

    // Progress bar: same gradient-meter treatment, driven by the polled
    // position/duration. Shows "--:--" until mpv reports a duration.
    let progress_pct = if app.playback_dur > 0.0 {
        ((app.playback_pos / app.playback_dur).clamp(0.0, 1.0) * 100.0) as u8
    } else {
        0
    };
    let mut progress_spans = gradient_meter_spans(progress_pct, 24, accent);
    progress_spans.push(Span::styled(
        format!(
            "  {} / {}",
            format_time(app.playback_pos),
            format_time(app.playback_dur)
        ),
        muted_style(),
    ));

    let source_line = if track.album.is_empty() {
        format!("{} {}", source_glyph(track.source), track.source.as_str())
    } else {
        format!(
            "{} {}  ·  {}",
            source_glyph(track.source),
            track.source.as_str(),
            track.album
        )
    };

    let lines = vec![
        Line::from(state_spans),
        Line::default(),
        Line::from(Span::styled(
            &track.title,
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(&track.artist, muted_style())),
        Line::from(Span::styled(
            source_line,
            Style::default().fg(source_color(track.source)),
        )),
        Line::default(),
        Line::from(progress_spans),
        Line::from(volume_spans),
        Line::from(vec![
            Span::styled("Repeat: ", muted_style()),
            Span::styled(app.loop_mode.label(), repeat_style(app.loop_mode)),
            Span::styled("    Shuffle: ", muted_style()),
            Span::styled(
                if app.shuffle { "On" } else { "Off" },
                shuffle_style(app.shuffle),
            ),
        ]),
    ];

    // Centered hero layout: artwork (capped width, horizontally centered)
    // stacked over the info block, the whole group vertically centered.
    let text_h = lines.len() as u16;
    let art_w = content_area.width.min(44);
    // When there's no vertical budget for artwork (e.g. a 80x24 terminal),
    // collapse it entirely rather than showing a useless 1-row sliver that
    // pushes the last info line off the bottom.
    let art_budget = content_area.height.saturating_sub(text_h + 1);
    let art_h = if art_budget == 0 {
        0
    } else {
        square_art_height(app, art_w, art_budget)
    };
    let separator = if art_h == 0 { 0 } else { 1 };
    let total_h = art_h + separator + text_h;
    let top = content_area.y + content_area.height.saturating_sub(total_h) / 2;

    if art_h > 0 {
        let art_area = Rect {
            x: content_area.x + content_area.width.saturating_sub(art_w) / 2,
            y: top,
            width: art_w,
            height: art_h,
        };
        draw_artwork(frame, app, art_area);
    }

    let text_area = Rect {
        x: content_area.x,
        y: top + art_h + separator,
        width: content_area.width,
        height: content_area
            .height
            .saturating_sub(top - content_area.y + art_h + separator),
    };
    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center),
        text_area,
    );
}

/// Public so `ui::soundcloud`'s sidebar can reuse the same artwork box. No
/// border/frame around the cover — it reads better floating on the
/// background, like a real album-art tile, rather than boxed in an outline
/// that's also tinted by (and can visually clash with) the accent color.
pub fn draw_artwork(frame: &mut Frame, app: &App, area: Rect) {
    match &app.artwork {
        Some(img) => render_cover(frame, app, img, area),
        None => {
            let para = Paragraph::new(vec![
                Line::default(),
                Line::from(Span::styled("♫", Style::default().fg(Color::DarkGray))),
                Line::from(Span::styled("no artwork", muted_style())),
            ])
            .alignment(Alignment::Center);
            frame.render_widget(para, area);
        }
    }
}

/// How tall (in terminal rows) a square image should render at the given
/// width, using the picker's detected cell pixel size — without this, a
/// generic `Fit` render inside a too-tall box leaves a lot of blank space
/// below the actual pixels before anything else starts.
fn square_art_height(app: &App, width: u16, max_height: u16) -> u16 {
    let (fw, fh) = app.picker.font_size();
    let fw = (fw as u32).max(1);
    let fh = (fh as u32).max(1);
    let height = ((width as u32 * fw) / fh) as u16;
    height.clamp(1, max_height.max(1))
}

/// Renders the decoded cover, re-encoding to the terminal's graphics protocol
/// only when the render area's size actually changes (cached in
/// `app.artwork_cache`, a `RefCell` — that's what lets this take `&App`
/// instead of threading `&mut App` through the whole UI module).
fn render_cover(frame: &mut Frame, app: &App, img: &DynamicImage, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut cache = app.artwork_cache.borrow_mut();
    let needs_encode = cache.as_ref().map(|(a, _)| *a != area).unwrap_or(true);
    if needs_encode {
        match app.picker.new_protocol(img.clone(), area, Resize::Fit(None)) {
            Ok(protocol) => *cache = Some((area, protocol)),
            Err(_) => return,
        }
    }
    if let Some((_, protocol)) = &*cache {
        frame.render_widget(Image::new(protocol), area);
    }
}

/// Compact artwork + "what's playing" block for sidebars and dashboard
/// panes (the SoundCloud tab's left column, the Dashboard's Now Playing
/// pane) that have room for more than a label but not the full Now Playing
/// tab's layout.
pub fn draw_mini_now_playing(frame: &mut Frame, app: &App, area: Rect) {
    let art_h = square_art_height(app, area.width, area.height.saturating_sub(3));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(art_h), Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    draw_artwork(frame, app, rows[0]);

    let (icon, label, state_color) = playback_status(app);
    let content = match &app.current_track {
        Some(track) => vec![
            Line::from(Span::styled(
                format!("{icon} {label}"),
                Style::default().fg(state_color).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                &track.title,
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(&track.artist, muted_style())),
        ],
        None => vec![Line::from(Span::styled(
            format!("{icon} {label}"),
            Style::default().fg(state_color),
        ))],
    };
    // rows[1] sits immediately under the artwork; rows[2] absorbs any
    // leftover space so the text block never gets pushed down by it.
    frame.render_widget(Paragraph::new(content).alignment(Alignment::Center), rows[1]);
}

fn repeat_style(mode: LoopMode) -> Style {
    if matches!(mode, LoopMode::Off) {
        Style::default().fg(Color::Gray)
    } else {
        Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
    }
}

fn shuffle_style(on: bool) -> Style {
    if on {
        Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    }
}

/// The hero row above the volume panel: playback state and the current
/// track, bold and on its own tinted background so it stands out from
/// everything else on screen — always visible regardless of the open tab.
pub fn draw_now_playing_line(frame: &mut Frame, app: &App, area: Rect) {
    let (icon, label, state_color) = playback_status(app);
    let bg = accent_bg(app);

    // The state word ("P L A Y I N G") gets a left-to-right gradient from its
    // state color toward a brightened shade of it — a small headline-grade
    // touch instead of one flat bold color.
    let state_base = to_rgb(state_color);
    let state_spans = gradient_spans(
        &letter_spaced(&label.to_uppercase()),
        state_base,
        brighten_rgb(state_base, 0.5),
        true,
    );
    let state_line = |suffix: Vec<Span<'static>>| {
        let mut spans = vec![Span::styled(
            format!("{icon}  "),
            Style::default().fg(state_color).add_modifier(Modifier::BOLD),
        )];
        spans.extend(state_spans.clone());
        spans.extend(suffix);
        Line::from(spans)
    };

    let mut lines = match &app.current_track {
        Some(track) => vec![
            state_line(Vec::new()),
            Line::from(vec![
                Span::styled(
                    &track.title,
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  by {}", track.artist), muted_style()),
            ]),
        ],
        None => vec![state_line(vec![Span::styled(
            " — Nothing loaded",
            Style::default().fg(state_color).add_modifier(Modifier::BOLD),
        )])],
    };

    // Vertically center by padding with blank lines — Paragraph has no
    // vertical-alignment option of its own, only horizontal.
    let top_pad = area.height.saturating_sub(lines.len() as u16) / 2;
    for _ in 0..top_pad {
        lines.insert(0, Line::default());
    }

    let para = Paragraph::new(lines)
        .style(Style::default().bg(bg))
        .block(
            Block::default()
                .borders(Borders::NONE)
                .padding(Padding::horizontal(2))
                .style(Style::default().bg(bg)),
        )
        .alignment(Alignment::Center);
    frame.render_widget(para, area);
}

pub fn draw_controls(frame: &mut Frame, app: &App, area: Rect) {
    let sep = || Span::styled("  │  ", muted_style());
    let accent = accent_color(app);

    let mut spans = vec![Span::styled("Vol ", muted_style())];
    spans.extend(gradient_meter_spans(app.volume, 14, accent));
    spans.extend([
        Span::styled(
            format!(" {}% ({})", app.volume, format_db(app.volume_db())),
            Style::default().fg(accent),
        ),
        sep(),
        Span::styled("Repeat ", muted_style()),
        Span::styled(app.loop_mode.label(), repeat_style(app.loop_mode)),
        sep(),
        Span::styled("Shuffle ", muted_style()),
        Span::styled(
            if app.shuffle { "On" } else { "Off" },
            shuffle_style(app.shuffle),
        ),
    ]);
    let line = Line::from(spans);

    let hint = Line::from(Span::styled(
        "[space] pause  [n] next  [l] repeat  [x] shuffle  [+/-] volume  [t] theme  [?] help  [Q] quit",
        muted_style(),
    ));

    let para = Paragraph::new(vec![line, hint])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(accent)),
        )
        .alignment(Alignment::Center);
    frame.render_widget(para, area);
}
