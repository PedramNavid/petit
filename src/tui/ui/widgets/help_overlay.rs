//! Help overlay widget showing full keybinding reference.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::tui::theme::Theme;

/// Render the help overlay.
pub fn render(frame: &mut Frame, area: Rect) {
    // Center the overlay
    let overlay_area = centered_rect(50, 60, area);

    // Clear the background
    frame.render_widget(Clear, overlay_area);

    let help_text = vec![
        Line::from(vec![Span::styled(
            "Keybindings",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Navigation",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from(""),
        key_line("Tab / Shift+Tab", "Switch between tabs"),
        key_line("j / Down", "Move selection down"),
        key_line("k / Up", "Move selection up"),
        key_line("g", "Go to top of list"),
        key_line("G", "Go to bottom of list"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Actions",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from(""),
        key_line("Enter", "Drill down (Jobs -> Runs -> Tasks)"),
        key_line("l", "View task logs (on Tasks tab)"),
        key_line("Backspace", "Go back"),
        key_line("r", "Force refresh"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "General",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from(""),
        key_line("q / Esc", "Quit"),
        key_line("?", "Toggle this help"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Press any key to close",
            Theme::text_dim(),
        )]),
    ];

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .style(Theme::border());

    let paragraph = Paragraph::new(help_text).block(block);
    frame.render_widget(paragraph, overlay_area);
}

/// Create a key-description line.
fn key_line(key: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {:20}", key), Theme::tab_active()),
        Span::styled(description.to_string(), Theme::text()),
    ])
}

/// Create a centered rectangle.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
