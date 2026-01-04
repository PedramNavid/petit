//! Common test utilities shared across integration tests.

use petit::{RunId, RunStatus, Storage, StoredRun};
use std::time::Duration;

/// Wait for a run to reach an expected status, polling storage.
///
/// This is more reliable than fixed sleeps since execution time can vary.
/// Polls storage every 10ms and times out after the specified duration.
///
/// # Panics
///
/// Panics if the timeout is reached before the run reaches the expected status.
pub async fn wait_for_run_status(
    storage: &dyn Storage,
    run_id: &RunId,
    expected: RunStatus,
    timeout: Duration,
) -> StoredRun {
    let start = tokio::time::Instant::now();
    loop {
        let run = storage.get_run(run_id).await.unwrap();
        if run.status == expected {
            return run;
        }
        if start.elapsed() > timeout {
            panic!(
                "Timeout waiting for run {} to reach {:?}, current status: {:?}",
                run_id, expected, run.status
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
