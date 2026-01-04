//! Help bar widget showing keybindings.

use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::theme::Theme;

/// Render the help bar.
pub fn render(frame: &mut Frame, area: Rect) {
    let spans = vec![
        Span::raw(" "),
        key_hint("q", "Quit"),
        Span::raw("  "),
        key_hint("Tab", "Switch"),
        Span::raw("  "),
        key_hint("j/k", "Navigate"),
        Span::raw("  "),
        key_hint("Enter", "Select"),
        Span::raw("  "),
        key_hint("l", "Logs"),
        Span::raw("  "),
        key_hint("Backspace", "Back"),
        Span::raw("  "),
        key_hint("r", "Refresh"),
        Span::raw("  "),
        key_hint("?", "Help"),
    ];

    let paragraph = Paragraph::new(Line::from(spans)).style(Theme::header());
    frame.render_widget(paragraph, area);
}

/// Create a key hint span.
fn key_hint(key: &str, action: &str) -> Span<'static> {
    Span::styled(format!("{}: {}", key, action), Theme::text_dim())
}
