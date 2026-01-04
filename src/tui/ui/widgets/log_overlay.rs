//! Log overlay widget showing task stdout/stderr output.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::storage::StoredTaskState;
use crate::tui::theme::Theme;

/// Render the log overlay for a task.
pub fn render(frame: &mut Frame, area: Rect, task: &StoredTaskState, scroll: u16) {
    // Create a larger overlay for log content
    let overlay_area = centered_rect(80, 80, area);

    // Clear the background
    frame.render_widget(Clear, overlay_area);

    // Build log content
    let mut lines: Vec<Line> = Vec::new();

    // Task header
    lines.push(Line::from(vec![
        Span::styled("Task: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(task.task_id.as_str(), Theme::tab_active()),
    ]));

    // Exit code
    if let Some(exit_code) = task.exit_code {
        let exit_style = if exit_code == 0 {
            Theme::success()
        } else {
            Theme::failure()
        };
        lines.push(Line::from(vec![
            Span::styled("Exit Code: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(exit_code.to_string(), exit_style),
        ]));
    }

    lines.push(Line::from(""));

    // Stdout section
    lines.push(Line::from(vec![Span::styled(
        "── stdout ──────────────────────────────────────────────────────────",
        Theme::text_dim(),
    )]));

    if let Some(ref stdout) = task.stdout {
        if stdout.is_empty() {
            lines.push(Line::from(Span::styled("(empty)", Theme::text_dim())));
        } else {
            for line in stdout.lines() {
                lines.push(Line::from(Span::raw(line.to_string())));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "(no output captured)",
            Theme::text_dim(),
        )));
    }

    lines.push(Line::from(""));

    // Stderr section
    lines.push(Line::from(vec![Span::styled(
        "── stderr ──────────────────────────────────────────────────────────",
        Theme::text_dim(),
    )]));

    if let Some(ref stderr) = task.stderr {
        if stderr.is_empty() {
            lines.push(Line::from(Span::styled("(empty)", Theme::text_dim())));
        } else {
            for line in stderr.lines() {
                lines.push(Line::from(Span::styled(line.to_string(), Theme::failure())));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "(no output captured)",
            Theme::text_dim(),
        )));
    }

    lines.push(Line::from(""));

    // Error section (if present)
    if let Some(ref error) = task.error {
        lines.push(Line::from(vec![Span::styled(
            "── error ───────────────────────────────────────────────────────────",
            Theme::failure(),
        )]));
        for line in error.lines() {
            lines.push(Line::from(Span::styled(line.to_string(), Theme::failure())));
        }
        lines.push(Line::from(""));
    }

    // Help line
    lines.push(Line::from(vec![Span::styled(
        "j/k: scroll │ PgUp/PgDn: page │ g: top │ q/Esc: close",
        Theme::text_dim(),
    )]));

    let title = format!(" Task Logs: {} ", task.task_id.as_str());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Theme::border());

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, overlay_area);
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
