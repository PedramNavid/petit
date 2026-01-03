pub mod config;
pub mod core;
pub mod events;
pub mod execution;
pub mod scheduler;
pub mod storage;
pub mod testing;

#[cfg(feature = "api")]
pub mod api;

#[cfg(feature = "tui")]
pub mod tui;

pub use core::context::{ContextError, ContextReader, ContextWriter, TaskContext};
pub use core::dag::{Dag, DagBuilder, DagError, TaskCondition, TaskNode};
pub use core::environment::Environment;
pub use core::job::{DependencyCondition, Job, JobBuilder, JobDependency, JobError};
pub use core::retry::{RetryCondition, RetryPolicy};
pub use core::schedule::{Schedule, ScheduleError};
pub use core::task::{Task, TaskError};
pub use core::types::{DagId, JobId, RunId, TaskId};

pub use events::{Event, EventBus, EventHandler};

pub use execution::{
    CommandTask, CommandTaskBuilder, DagExecutor, DagResult, TaskExecutor, TaskResult, TaskStatus,
};

pub use storage::{
    InMemoryStorage, RunStatus, Storage, StorageError, StoredJob, StoredRun, StoredTaskState,
    TaskRunStatus,
};

#[cfg(feature = "sqlite")]
pub use storage::SqliteStorage;

pub use config::{
    ConfigError, GlobalConfig, JobConfig, JobConfigBuilder, RetryConfig, ScheduleConfig,
    TaskConfig, YamlLoader, load_jobs_from_directory,
};

pub use scheduler::{Scheduler, SchedulerError, SchedulerHandle, SchedulerState};
pub use testing::{FailingTask, FailureInjection, MockTaskContext, TestHarness, TestResult};

#[cfg(feature = "api")]
pub use api::{ApiConfig, ApiState, build_router, create_api_state, start_server};
