//! Main layout rendering.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
};

use crate::tui::app::{App, Tab};

use super::tabs;
use super::widgets::{help_bar, help_overlay, log_overlay, status_bar, tab_bar};

/// Render the entire application.
pub fn render(app: &mut App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Status bar
            Constraint::Length(1), // Tab bar
            Constraint::Min(0),    // Main content
            Constraint::Length(1), // Help bar
        ])
        .split(frame.area());

    // Render status bar
    status_bar::render(app, frame, chunks[0]);

    // Render tab bar
    tab_bar::render(app, frame, chunks[1]);

    // Render main content based on current tab
    render_tab_content(app, frame, chunks[2]);

    // Render help bar
    help_bar::render(frame, chunks[3]);

    // Render help overlay if active
    if app.show_help {
        help_overlay::render(frame, frame.area());
    }

    // Render log overlay if active
    if app.show_logs
        && let Some(task) = app.selected_task()
    {
        log_overlay::render(frame, frame.area(), task, app.log_scroll);
    }
}

/// Render the content for the current tab.
fn render_tab_content(app: &mut App, frame: &mut Frame, area: Rect) {
    match app.current_tab {
        Tab::Jobs => tabs::jobs::render(app, frame, area),
        Tab::Runs => tabs::runs::render(app, frame, area),
        Tab::Tasks => tabs::tasks::render(app, frame, area),
        Tab::Events => tabs::events::render(app, frame, area),
    }
}
