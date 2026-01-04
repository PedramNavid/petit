//! Tasks tab renderer.

use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::storage::TaskRunStatus;
use crate::tui::app::App;
use crate::tui::theme::Theme;

/// Render the tasks tab.
pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    if app.runs_list_state.selected().is_none() {
        // No run selected
        let message = Paragraph::new("Select a run from the Runs tab to view tasks")
            .style(Theme::text_dim())
            .block(
                Block::default()
                    .title(" Tasks ")
                    .borders(Borders::ALL)
                    .style(Theme::border()),
            );
        frame.render_widget(message, area);
        return;
    }

    let items: Vec<ListItem> = app
        .tasks
        .iter()
        .map(|task| {
            // Status indicator and style
            let (status_icon, status_style) = match task.status {
                TaskRunStatus::Completed => ("✓", Theme::success()),
                TaskRunStatus::Failed => ("✗", Theme::failure()),
                TaskRunStatus::Running => ("⟳", Theme::running()),
                TaskRunStatus::Pending => ("…", Theme::pending()),
                TaskRunStatus::Skipped => ("⊘", Theme::skipped()),
            };

            // Duration
            let duration = task
                .duration
                .map(App::format_duration)
                .unwrap_or_else(|| "-".to_string());

            // Attempts
            let attempts = if task.attempts > 1 {
                format!(" ({}x)", task.attempts)
            } else {
                String::new()
            };

            // Logs indicator
            let has_logs = task.stdout.is_some() || task.stderr.is_some();
            let logs_icon = if has_logs { "📋" } else { "  " };

            // Error (truncated)
            let error_span = if let Some(ref error) = task.error {
                let truncated = if error.len() > 30 {
                    format!("{}...", &error[..27])
                } else {
                    error.clone()
                };
                vec![Span::raw(" │ "), Span::styled(truncated, Theme::failure())]
            } else {
                vec![]
            };

            let mut spans = vec![
                Span::styled(format!("{} ", status_icon), status_style),
                Span::styled(format!("{:<25}", task.task_id.as_str()), Theme::text()),
                Span::styled(format!("{:>8}", duration), Theme::text_dim()),
                Span::styled(attempts, Theme::text_dim()),
                Span::raw(" "),
                Span::raw(logs_icon),
            ];
            spans.extend(error_span);

            ListItem::new(Line::from(spans))
        })
        .collect();

    let title = if let Some(run) = app.selected_run() {
        format!(
            " Tasks for run {} ({}) ",
            &run.id.to_string()[..8],
            app.tasks.len()
        )
    } else {
        format!(" Tasks ({}) ", app.tasks.len())
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .style(Theme::border()),
        )
        .highlight_style(Theme::selected().add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(list, area, &mut app.tasks_list_state);
}
