//! Application state and business logic for the TUI.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use std::time::{Duration, Instant};

use crate::storage::{StoredRun, StoredTaskState};

use super::error::TuiError;
use super::reader::{DashboardStats, JobWithStats, TuiReader};

/// The currently selected tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Jobs,
    Runs,
    Tasks,
    Events,
}

impl Tab {
    /// Get all tabs in order.
    pub fn all() -> &'static [Tab] {
        &[Tab::Jobs, Tab::Runs, Tab::Tasks, Tab::Events]
    }

    /// Get the display name for this tab.
    pub fn name(&self) -> &'static str {
        match self {
            Tab::Jobs => "Jobs",
            Tab::Runs => "Runs",
            Tab::Tasks => "Tasks",
            Tab::Events => "Events",
        }
    }

    /// Move to the next tab.
    pub fn next(&self) -> Tab {
        match self {
            Tab::Jobs => Tab::Runs,
            Tab::Runs => Tab::Tasks,
            Tab::Tasks => Tab::Events,
            Tab::Events => Tab::Jobs,
        }
    }

    /// Move to the previous tab.
    pub fn prev(&self) -> Tab {
        match self {
            Tab::Jobs => Tab::Events,
            Tab::Runs => Tab::Jobs,
            Tab::Tasks => Tab::Runs,
            Tab::Events => Tab::Tasks,
        }
    }
}

/// Main application state.
pub struct App {
    /// The database reader.
    reader: TuiReader,

    /// Current tab.
    pub current_tab: Tab,

    /// Dashboard statistics.
    pub stats: DashboardStats,

    /// Jobs with their statistics.
    pub jobs: Vec<JobWithStats>,

    /// Runs for the selected job (or all recent runs on Runs tab).
    pub runs: Vec<StoredRun>,

    /// Task states for the selected run.
    pub tasks: Vec<StoredTaskState>,

    /// List state for jobs tab.
    pub jobs_list_state: ListState,

    /// List state for runs tab.
    pub runs_list_state: ListState,

    /// List state for tasks tab.
    pub tasks_list_state: ListState,

    /// Whether the application should quit.
    pub should_quit: bool,

    /// Current error message to display.
    pub error_message: Option<String>,

    /// Time of last data refresh.
    pub last_refresh: Instant,

    /// Whether a refresh is currently in progress.
    pub refreshing: bool,

    /// Show help overlay.
    pub show_help: bool,

    /// Show log viewer overlay.
    pub show_logs: bool,

    /// Scroll position in log viewer.
    pub log_scroll: u16,
}

impl App {
    /// Create a new application with the given database reader.
    pub async fn new(reader: TuiReader) -> Result<Self, TuiError> {
        let mut app = Self {
            reader,
            current_tab: Tab::Jobs,
            stats: DashboardStats::default(),
            jobs: Vec::new(),
            runs: Vec::new(),
            tasks: Vec::new(),
            jobs_list_state: ListState::default(),
            runs_list_state: ListState::default(),
            tasks_list_state: ListState::default(),
            should_quit: false,
            error_message: None,
            last_refresh: Instant::now(),
            refreshing: false,
            show_help: false,
            show_logs: false,
            log_scroll: 0,
        };

        // Initial data load
        app.refresh().await;

        Ok(app)
    }

    /// Refresh data from the database.
    pub async fn refresh(&mut self) {
        self.refreshing = true;
        self.error_message = None;

        // Refresh stats
        match self.reader.get_stats().await {
            Ok(stats) => self.stats = stats,
            Err(e) => {
                self.error_message = Some(format!("Stats error: {}", e));
            }
        }

        // Refresh based on current tab
        match self.current_tab {
            Tab::Jobs => {
                self.refresh_jobs().await;
            }
            Tab::Runs => {
                self.refresh_runs().await;
            }
            Tab::Tasks => {
                self.refresh_tasks().await;
            }
            Tab::Events => {
                // Events tab shows recent runs as activity
                self.refresh_recent_runs().await;
            }
        }

        self.last_refresh = Instant::now();
        self.refreshing = false;
    }

    /// Refresh jobs list.
    async fn refresh_jobs(&mut self) {
        match self.reader.list_jobs_with_stats().await {
            Ok(jobs) => {
                let was_empty = self.jobs.is_empty();
                self.jobs = jobs;
                // Select first job if none selected
                if was_empty && !self.jobs.is_empty() {
                    self.jobs_list_state.select(Some(0));
                }
            }
            Err(e) => {
                self.error_message = Some(format!("Jobs error: {}", e));
            }
        }
    }

    /// Refresh runs list for selected job.
    async fn refresh_runs(&mut self) {
        if let Some(job_idx) = self.jobs_list_state.selected() {
            if let Some(job) = self.jobs.get(job_idx) {
                match self.reader.list_runs(&job.job.id, 50, 0).await {
                    Ok(runs) => {
                        let was_empty = self.runs.is_empty();
                        self.runs = runs;
                        if was_empty && !self.runs.is_empty() {
                            self.runs_list_state.select(Some(0));
                        }
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Runs error: {}", e));
                    }
                }
            }
        } else {
            // No job selected, show all recent runs
            self.refresh_recent_runs().await;
        }
    }

    /// Refresh recent runs across all jobs.
    async fn refresh_recent_runs(&mut self) {
        match self.reader.list_recent_runs(50).await {
            Ok(runs) => {
                let was_empty = self.runs.is_empty();
                self.runs = runs;
                if was_empty && !self.runs.is_empty() {
                    self.runs_list_state.select(Some(0));
                }
            }
            Err(e) => {
                self.error_message = Some(format!("Runs error: {}", e));
            }
        }
    }

    /// Refresh task states for selected run.
    async fn refresh_tasks(&mut self) {
        if let Some(run_idx) = self.runs_list_state.selected()
            && let Some(run) = self.runs.get(run_idx)
        {
            match self.reader.list_task_states(&run.id).await {
                Ok(tasks) => {
                    let was_empty = self.tasks.is_empty();
                    self.tasks = tasks;
                    if was_empty && !self.tasks.is_empty() {
                        self.tasks_list_state.select(Some(0));
                    }
                }
                Err(e) => {
                    self.error_message = Some(format!("Tasks error: {}", e));
                }
            }
        }
    }

    /// Handle a key event.
    pub async fn handle_key(&mut self, key: KeyEvent) {
        // Close help overlay first
        if self.show_help {
            self.show_help = false;
            return;
        }

        // Handle log viewer overlay
        if self.show_logs {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                    self.show_logs = false;
                    self.log_scroll = 0;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.log_scroll = self.log_scroll.saturating_add(1);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.log_scroll = self.log_scroll.saturating_sub(1);
                }
                KeyCode::PageDown => {
                    self.log_scroll = self.log_scroll.saturating_add(10);
                }
                KeyCode::PageUp => {
                    self.log_scroll = self.log_scroll.saturating_sub(10);
                }
                KeyCode::Char('g') | KeyCode::Home => {
                    self.log_scroll = 0;
                }
                _ => {}
            }
            return;
        }

        match key.code {
            // Quit
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
            }

            // Tab navigation
            KeyCode::Tab => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.current_tab = self.current_tab.prev();
                } else {
                    self.current_tab = self.current_tab.next();
                }
                self.refresh().await;
            }
            KeyCode::BackTab => {
                self.current_tab = self.current_tab.prev();
                self.refresh().await;
            }

            // List navigation
            KeyCode::Down | KeyCode::Char('j') => {
                self.select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.select_prev();
            }
            KeyCode::Char('g') => {
                self.select_first();
            }
            KeyCode::Char('G') => {
                self.select_last();
            }

            // Drill down / go back / view logs
            KeyCode::Enter => {
                self.drill_down().await;
            }
            KeyCode::Backspace => {
                self.go_back().await;
            }

            // View logs (on Tasks tab)
            KeyCode::Char('l') => {
                if self.current_tab == Tab::Tasks && self.tasks_list_state.selected().is_some() {
                    self.show_logs = true;
                    self.log_scroll = 0;
                }
            }

            // Force refresh
            KeyCode::Char('r') => {
                self.refresh().await;
            }

            // Help
            KeyCode::Char('?') => {
                self.show_help = true;
            }

            _ => {}
        }
    }

    /// Select next item in current list.
    fn select_next(&mut self) {
        let (list_state, len) = self.current_list_state();
        if len == 0 {
            return;
        }
        let i = match list_state.selected() {
            Some(i) => (i + 1).min(len - 1),
            None => 0,
        };
        list_state.select(Some(i));
    }

    /// Select previous item in current list.
    fn select_prev(&mut self) {
        let (list_state, len) = self.current_list_state();
        if len == 0 {
            return;
        }
        let i = match list_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        list_state.select(Some(i));
    }

    /// Select first item in current list.
    fn select_first(&mut self) {
        let (list_state, len) = self.current_list_state();
        if len > 0 {
            list_state.select(Some(0));
        }
    }

    /// Select last item in current list.
    fn select_last(&mut self) {
        let (list_state, len) = self.current_list_state();
        if len > 0 {
            list_state.select(Some(len - 1));
        }
    }

    /// Get the current list state and length based on active tab.
    fn current_list_state(&mut self) -> (&mut ListState, usize) {
        match self.current_tab {
            Tab::Jobs => (&mut self.jobs_list_state, self.jobs.len()),
            Tab::Runs | Tab::Events => (&mut self.runs_list_state, self.runs.len()),
            Tab::Tasks => (&mut self.tasks_list_state, self.tasks.len()),
        }
    }

    /// Drill down from current selection.
    async fn drill_down(&mut self) {
        match self.current_tab {
            Tab::Jobs => {
                // Go to runs for selected job
                if self.jobs_list_state.selected().is_some() {
                    self.current_tab = Tab::Runs;
                    self.runs_list_state.select(None);
                    self.refresh().await;
                }
            }
            Tab::Runs => {
                // Go to tasks for selected run
                if self.runs_list_state.selected().is_some() {
                    self.current_tab = Tab::Tasks;
                    self.tasks_list_state.select(None);
                    self.refresh().await;
                }
            }
            Tab::Tasks | Tab::Events => {
                // No drill-down from tasks or events
            }
        }
    }

    /// Go back to previous view.
    async fn go_back(&mut self) {
        match self.current_tab {
            Tab::Tasks => {
                self.current_tab = Tab::Runs;
                self.refresh().await;
            }
            Tab::Runs => {
                self.current_tab = Tab::Jobs;
                self.refresh().await;
            }
            Tab::Jobs | Tab::Events => {
                // No going back from jobs or events
            }
        }
    }

    /// Get the selected job.
    pub fn selected_job(&self) -> Option<&JobWithStats> {
        self.jobs_list_state
            .selected()
            .and_then(|i| self.jobs.get(i))
    }

    /// Get the selected run.
    pub fn selected_run(&self) -> Option<&StoredRun> {
        self.runs_list_state
            .selected()
            .and_then(|i| self.runs.get(i))
    }

    /// Get the selected task.
    pub fn selected_task(&self) -> Option<&StoredTaskState> {
        self.tasks_list_state
            .selected()
            .and_then(|i| self.tasks.get(i))
    }

    /// Format duration for display.
    pub fn format_duration(duration: Duration) -> String {
        let secs = duration.as_secs();
        if secs < 60 {
            format!("{}s", secs)
        } else if secs < 3600 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
        }
    }

    /// Format relative time (e.g., "2m ago").
    pub fn format_relative(instant: std::time::SystemTime) -> String {
        let now = std::time::SystemTime::now();
        match now.duration_since(instant) {
            Ok(duration) => {
                let secs = duration.as_secs();
                if secs < 60 {
                    format!("{}s ago", secs)
                } else if secs < 3600 {
                    format!("{}m ago", secs / 60)
                } else if secs < 86400 {
                    format!("{}h ago", secs / 3600)
                } else {
                    format!("{}d ago", secs / 86400)
                }
            }
            Err(_) => "future".to_string(),
        }
    }
}
