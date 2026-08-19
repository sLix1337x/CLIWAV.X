use crate::app::{App, LoopMode};
use crate::ui::waveform;
use crate::ui::{
    accent_color, brighten_rgb, draw_section, gradient_meter_spans, gradient_spans, muted_style,
    playback_status, source_color, source_glyph, to_rgb, MODE_ON,
};
use image::DynamicImage;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
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

/// A panel background just off the terminal's own, faintly tinted toward the
/// active accent color when one is available — reads as a low-opacity overlay
/// rather than a solid, saturated fill. The base tracks whether the terminal
/// itself is dark or light so the panel stays a subtle step away from the
/// surrounding background either way.
fn accent_bg(app: &App) -> Color {
    let base = crate::ui::theme::surface_base(app.is_dark_bg, (24, 24, 27), (235, 235, 232));
    let (br, bg_, bb) = (base.0 as u16, base.1 as u16, base.2 as u16);
    let Some((r, g, b)) = app.palette.rgb().or(app.artwork_accent) else {
        return Color::Rgb(base.0, base.1, base.2);
    };
    // ~12% accent / 88% base.
    let mix = |base: u16, accent: u8| ((base * 7 + accent as u16) / 8) as u8;
    Color::Rgb(mix(br, r), mix(bg_, g), mix(bb, b))
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

    // Extra breathing room after the glyph: several source glyphs (e.g. the
    // SoundCloud cloud) render wider than the single cell the layout budgets
    // for in some fonts, crowding a single space.
    let source_line = if track.album.is_empty() {
        format!("{}  {}", source_glyph(track.source), track.source.display_name())
    } else {
        format!(
            "{}  {}  ·  {}",
            source_glyph(track.source),
            track.source.display_name(),
            track.album
        )
    };
    let source_span = Span::styled(source_line, Style::default().fg(source_color(track.source)));

    // Letter-spaced for visual weight — the same "bigger" trick used for the
    // state headline, since terminals have no real font-size control.
    let title_line = Line::from(Span::styled(
        letter_spaced(&track.title),
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    ));
    let artist_line = Line::from(Span::styled(&track.artist, muted_style()));
    let volume_line = Line::from(volume_spans);
    let repeat_shuffle_line = Line::from(vec![
        Span::styled("Repeat: ", muted_style()),
        Span::styled(app.loop_mode.label(), repeat_style(app.loop_mode)),
        Span::styled("    Shuffle: ", muted_style()),
        Span::styled(
            if app.shuffle { "On" } else { "Off" },
            shuffle_style(app.shuffle),
        ),
    ]);

    // Live visualizer: rendered as a background layer behind the *whole*
    // panel (artwork, title/artist card, waveform band) rather than taking
    // its own reserved space — everything else below draws on top of it
    // normally, so it just shows through wherever nothing else is opaque.
    if let Some(mode) = app.visualizer_mode {
        crate::ui::visualizer::draw(frame, app, content_area, accent, mode);
    }

    // A full-width waveform band sits under the artwork+info row (art
    // column left, title/artist/volume/repeat block right, or stacked on
    // narrow terminals — same as before), so it gets the whole section's
    // width for maximum horizontal resolution rather than being squeezed
    // into whichever column happens to hold it.
    let waveform_block_h = WAVEFORM_BAND_HEIGHT + 2; // 1-row gap above, 1-row time readout below
    let (top_area, waveform_area) = if content_area.height > waveform_block_h {
        let rows = Layout::vertical([
            Constraint::Length(content_area.height - waveform_block_h),
            Constraint::Length(waveform_block_h),
        ])
        .split(content_area);
        // Inset from the section's full width — a waveform running edge to
        // edge read as too long; a margin on both sides makes it feel like
        // a deliberate element rather than a stretched-out bar.
        let inset = rows[1].width / 10;
        let band = Rect {
            x: rows[1].x + inset,
            y: rows[1].y,
            width: rows[1].width.saturating_sub(inset * 2),
            height: rows[1].height,
        };
        (rows[0], Some(band))
    } else {
        (content_area, None)
    };

    // Side-by-side on anything reasonably wide: artwork on the left (source
    // caption above it, playback state below it — a media-player-style
    // frame around the cover) and the title/artist/volume block on the
    // right, vertically centered on the same row so the two columns read as
    // one aligned unit. Narrow terminals fall back to the old stacked
    // layout — there isn't room for a legible cover next to a legible text
    // column below ~70 columns.
    if top_area.width >= NOW_PLAYING_SIDE_BY_SIDE_MIN_WIDTH {
        let right_lines = vec![
            title_line,
            artist_line,
            Line::default(),
            volume_line,
            Line::default(),
            repeat_shuffle_line,
        ];
        let right_h = right_lines.len() as u16;
        draw_side_by_side(
            frame,
            app,
            top_area,
            source_span,
            Line::from(state_spans),
            right_lines,
            right_h,
        );
    } else {
        let lines = vec![
            Line::from(state_spans),
            Line::default(),
            title_line,
            artist_line,
            Line::from(source_span),
            Line::default(),
            volume_line,
            Line::default(),
            repeat_shuffle_line,
        ];
        let text_h = lines.len() as u16;
        draw_stacked(frame, app, top_area, lines, text_h);
    }

    if let Some(waveform_area) = waveform_area {
        draw_waveform_band(frame, app, waveform_area, accent);
    }
}

const NOW_PLAYING_SIDE_BY_SIDE_MIN_WIDTH: u16 = 70;
/// Rows the mirrored waveform bars themselves occupy (excludes the gap row
/// above and the time-readout row below).
const WAVEFORM_BAND_HEIGHT: u16 = 4;

/// Full-width band under the artwork+info row: the waveform (or, until one
/// is loaded/cached/supported for this track, today's plain progress bar as
/// a seamless fallback) plus an elapsed/total time readout underneath.
fn draw_waveform_band(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    if area.width == 0 {
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(WAVEFORM_BAND_HEIGHT),
        Constraint::Length(1),
    ])
    .split(area);
    let bars_area = rows[1];
    let readout_area = rows[2];

    let progress_ratio = if app.playback_dur > 0.0 {
        (app.playback_pos / app.playback_dur).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };

    // While the visualizer is on, it's already rendered as the panel's
    // background (see `draw` above, which draws it before this function is
    // even called) — this band just shows the time readout on top of it,
    // same as everywhere else in the panel, instead of also drawing the
    // static waveform/fallback bar over it.
    if app.visualizer_mode.is_none() {
        match &app.waveform {
            Some(samples) => {
                let lines = waveform::render(samples, bars_area.width, bars_area.height, progress_ratio, accent);
                frame.render_widget(Paragraph::new(lines), bars_area);
            }
            None => {
                // Fallback: today's flat bar, a single row centered in the
                // band's vertical space so it doesn't look stranded at the top.
                let progress_pct = (progress_ratio * 100.0) as u8;
                let spans = gradient_meter_spans(progress_pct, bars_area.width as usize, accent);
                let fallback_area = Rect {
                    x: bars_area.x,
                    y: bars_area.y + bars_area.height / 2,
                    width: bars_area.width,
                    height: 1,
                };
                frame.render_widget(Paragraph::new(Line::from(spans)), fallback_area);
            }
        }
    }

    let elapsed = format_time(app.playback_pos);
    let total = format_time(app.playback_dur);
    let gap = " ".repeat(
        (readout_area.width as usize).saturating_sub(elapsed.chars().count() + total.chars().count()),
    );
    let readout = Line::from(vec![
        Span::styled(elapsed, muted_style()),
        Span::raw(gap),
        Span::styled(total, muted_style()),
    ]);
    frame.render_widget(Paragraph::new(readout), readout_area);
}
/// Horizontal gap between the artwork column and the info column.
const NOW_PLAYING_GAP: u16 = 3;
/// Artwork never grows past this even on very wide terminals — it's a
/// side column sharing space with text, not the whole show.
const NOW_PLAYING_MAX_ART_WIDTH: u16 = 36;
/// Text column never grows past this either — without a cap it stretched to
/// fill the rest of a wide terminal, which just pinned the whole art+info
/// group against the left edge instead of it reading as one centered unit.
const NOW_PLAYING_MAX_TEXT_WIDTH: u16 = 50;

/// Artwork column: a source caption above the cover, playback state below
/// it — framed like a media player's now-playing card, not just a bare
/// image. `right_lines` is the title/artist/timeline/volume block.
fn draw_side_by_side<'a>(
    frame: &mut Frame,
    app: &App,
    content_area: Rect,
    source_caption: Span<'a>,
    state_caption: Line<'a>,
    right_lines: Vec<Line<'a>>,
    right_h: u16,
) {
    let art_w = ((content_area.width.saturating_sub(NOW_PLAYING_GAP)) * 2 / 5)
        .min(NOW_PLAYING_MAX_ART_WIDTH);
    let text_w = content_area
        .width
        .saturating_sub(art_w + NOW_PLAYING_GAP)
        .min(NOW_PLAYING_MAX_TEXT_WIDTH);
    // The art+gap+text group is centered as a single block within the
    // section — without this, capping `text_w` above just left a wide gap
    // on the right instead of fixing the "everything hugs the left edge"
    // problem it was meant to solve.
    let block_w = art_w + NOW_PLAYING_GAP + text_w;
    let block_x = content_area.x + content_area.width.saturating_sub(block_w) / 2;

    // Reserve the caption rows (source above, state below) plus a 1-row gap
    // on each side of the cover, so the whole card gets a vertical budget.
    let art_budget = content_area.height.saturating_sub(4);
    let art_h = if art_w == 0 || art_budget == 0 {
        0
    } else {
        square_art_height(app, art_w, art_budget)
    };
    let left_h = art_h + 4; // source + gap + art + gap + state

    // Both columns center on the same row of `content_area` — that's what
    // keeps the artwork card and the text block visually aligned regardless
    // of how tall either one ends up being.
    let center_y = content_area.y + content_area.height / 2;
    let left_y = center_y.saturating_sub(left_h / 2).max(content_area.y);
    let right_y = center_y.saturating_sub(right_h / 2).max(content_area.y);

    let caption_area = |y: u16| Rect {
        x: block_x,
        y,
        width: art_w,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(source_caption).alignment(Alignment::Center),
        caption_area(left_y),
    );

    let art_y = left_y + 2;
    if art_h > 0 {
        let art_area = Rect {
            x: block_x,
            y: art_y,
            width: art_w,
            height: art_h,
        };
        draw_artwork(frame, app, art_area);
    }

    frame.render_widget(
        Paragraph::new(state_caption).alignment(Alignment::Center),
        caption_area(art_y + art_h + 1),
    );

    // Height capped to the block's actual content (not stretched to fill
    // the rest of `content_area`) — a `Paragraph` blanks every cell in its
    // Rect, including empty padding, which would paint over the visualizer
    // background below the text wherever this block doesn't actually reach.
    let text_area = Rect {
        x: block_x + art_w + NOW_PLAYING_GAP,
        y: right_y,
        width: text_w,
        height: right_h.min(content_area.height.saturating_sub(right_y - content_area.y)),
    };
    frame.render_widget(
        Paragraph::new(right_lines).alignment(Alignment::Left),
        text_area,
    );
}

/// Pre-side-by-side layout, kept for terminals too narrow to fit a legible
/// cover next to a legible text column: artwork centered above the (also
/// centered) info block, the whole group vertically centered as a unit.
fn draw_stacked<'a>(
    frame: &mut Frame,
    app: &App,
    content_area: Rect,
    lines: Vec<Line<'a>>,
    text_h: u16,
) {
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

    // Same reasoning as the side-by-side layout's text_area: capped to the
    // block's actual content height so a stretched-out Paragraph doesn't
    // blank the visualizer background below it.
    let text_area = Rect {
        x: content_area.x,
        y: top + art_h + separator,
        width: content_area.width,
        height: text_h.min(
            content_area
                .height
                .saturating_sub(top - content_area.y + art_h + separator),
        ),
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

/// Off is plain white — it's the resting state, and dimming it made the
/// whole repeat control look disabled rather than merely inactive. Track and
/// All light up in violet so an engaged mode is spottable without reading.
fn repeat_style(mode: LoopMode) -> Style {
    if matches!(mode, LoopMode::Off) {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(MODE_ON).add_modifier(Modifier::BOLD)
    }
}

/// Matches `repeat_style`: the two controls sit side by side in the same
/// panel, so an engaged shuffle has to read as the same kind of state as an
/// engaged repeat rather than a different colour of thing.
fn shuffle_style(on: bool) -> Style {
    if on {
        Style::default().fg(MODE_ON).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    }
}

/// The hero row above the volume panel: playback state and the current
/// track, bold and on its own tinted background so it stands out from
/// everything else on screen — always visible regardless of the open tab.
pub fn draw_now_playing_line(frame: &mut Frame, app: &App, area: Rect) {
    let (icon, label, state_color) = playback_status(app);
    let bg = accent_bg(app);
    let accent = accent_color(app);

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
        Some(track) => {
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
            vec![
                state_line(Vec::new()),
                Line::from(vec![
                    Span::styled(
                        &track.title,
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("  by {}", track.artist), muted_style()),
                ]),
                Line::from(progress_spans),
            ]
        }
        None => vec![state_line(vec![Span::styled(
            " — Nothing loaded",
            Style::default().fg(state_color).add_modifier(Modifier::BOLD),
        )])],
    };

    // Vertically center by padding with blank lines — Paragraph has no
    // vertical-alignment option of its own, only horizontal. Rounds the
    // top padding up (not down) so an odd leftover row goes above the
    // content instead of silently padding out the bottom.
    let top_pad = area.height.saturating_sub(lines.len() as u16).div_ceil(2);
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
        "[space] pause  [n] next  [l] repeat  [x] shuffle  [+/-] volume  [v] cycle visualizer  [t] theme  [?] help  [Q] quit",
        muted_style(),
    ));

    let para = Paragraph::new(vec![line, hint])
        .block(crate::ui::theme::panel_border(accent))
        .alignment(Alignment::Center);
    frame.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_engaged_repeat_mode_is_violet() {
        assert_eq!(repeat_style(LoopMode::Off).fg, Some(Color::White));
        assert_eq!(repeat_style(LoopMode::Track).fg, Some(MODE_ON));
        assert_eq!(repeat_style(LoopMode::Queue).fg, Some(MODE_ON));
    }

    #[test]
    fn shuffle_uses_the_same_states_as_repeat() {
        // The two sit side by side, so "on" has to look like the same kind
        // of thing in both rather than two different colours of engaged.
        assert_eq!(shuffle_style(true).fg, repeat_style(LoopMode::Track).fg);
        assert_eq!(shuffle_style(false).fg, repeat_style(LoopMode::Off).fg);
    }
}
