//! Small, named style-building helpers for patterns repeated across
//! `ui/*.rs` — a thin layer, not a theming system. Add to this only when a
//! pattern is genuinely duplicated at several call sites (see
//! `RESEARCH_NOTES.md` section 4); this isn't meant to grow into a spot to
//! pre-emptively define every style anyone might someday want.

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders};

/// The rounded-corner accent-colored panel border used by the tab bar, the
/// controls box, the search input, the input-prompt popup, and the help
/// overlay — same three `Block` calls, previously retyped at each site.
/// Callers chain `.title(...)`/`.style(...)` on top as needed; `Block`'s
/// builder methods consume and return `Self`, so this composes normally.
pub fn panel_border(accent: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
}

/// Bold, accent-colored text — the "this is the emphasized value" role used
/// for active-state labels and headline accents.
pub fn accent_bold(accent: Color) -> Style {
    Style::default().fg(accent).add_modifier(Modifier::BOLD)
}
