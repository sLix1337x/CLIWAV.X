use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

const BINDINGS: &[(&str, &str)] = &[
    ("1  2  3  4  5  6  7", "Jump to Dashboard / Now Playing / Library / Queue / SoundCloud / EQ / Search"),
    ("Tab / Shift+Tab", "Switch tabs (Dashboard: toggle pane; Search: toggle typing/browsing; Library & SoundCloud: toggle pane)"),
    ("Up/Down, k/j", "Navigate list"),
    ("Left/Right (Dashboard)", "Switch Search / Tracks / Likes / Reposts / Library"),
    ("Left/Right (EQ)", "Select band"),
    ("Up/Down (EQ)", "Adjust selected band's gain"),
    ("[ / ] (EQ)", "Cycle built-in EQ presets"),
    ("Shift+Left/Right", "Rewind / fast-forward the current track (5s)"),
    ("Enter (Search, typing)", "Run search, then switch to browsing results"),
    ("Enter (elsewhere)", "Play selected track, drill into a SoundCloud category, or open Library"),
    ("a", "Add selected track to queue"),
    ("s", "Save selected track to library"),
    ("p", "Add selected search track to current playlist (Search tab)"),
    ("Space", "Pause / resume"),
    ("n", "Next track"),
    ("n (Library tab)", "Create a new playlist"),
    ("d (Library tab)", "Delete selected playlist / remove saved track"),
    ("Ctrl+Z", "Undo the last playlist deletion"),
    ("+ / -", "Volume up / down"),
    ("f", "Cycle source filter (local, YouTube, SoundCloud, Spotify, all)"),
    ("g", "Toggle game mode (reduces TUI refresh rate)"),
    ("l", "Cycle repeat mode (Off -> Track -> All)"),
    ("x", "Toggle shuffle"),
    ("t", "Cycle accent palette (auto from artwork, teal, magenta, amber, violet)"),
    ("m (SoundCloud/Dashboard)", "Load the next page of Tracks/Likes/Reposts"),
    ("S", "Set up Spotify credentials"),
    ("C", "Set your SoundCloud username"),
    ("r", "Rescan local library"),
    ("?", "Toggle this help screen (only outside the search box)"),
    ("Q, Ctrl+Q, Ctrl+C", "Quit"),
];

pub fn draw(frame: &mut Frame) {
    let area = frame.area();
    let width = (area.width * 9 / 10).max(60).min(area.width);
    let height = (BINDINGS.len() as u16 + 4).min(area.height);
    let popup_area = Rect {
        x: (area.width.saturating_sub(width)) / 2,
        y: (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, popup_area);

    let key_col = BINDINGS.iter().map(|(k, _)| k.len()).max().unwrap_or(0);

    let mut lines: Vec<Line> = BINDINGS
        .iter()
        .map(|(key, action)| {
            Line::from(vec![
                Span::styled(
                    format!("  {:<width$}", key, width = key_col),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {}", action), Style::default().fg(Color::White)),
            ])
        })
        .collect();
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "  Press ?, Esc, or q to close",
        Style::default().fg(Color::Gray),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Keybindings ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Black));
    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, popup_area);
}
