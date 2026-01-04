//! API integration tests.
//!
//! These tests verify the HTTP API server starts and responds correctly.

use crate::common::wait_for_run_status;
use petit::api::{build_router, create_api_state};
use petit::core::dag::Dag;
use petit::core::job::Job;
use petit::scheduler::Scheduler;
use petit::storage::InMemoryStorage;
use petit::{RunId, RunStatus};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

/// Create a test API state with a simple job.
/// Returns both the API state and the storage for testing purposes.
async fn create_test_state() -> (petit::api::ApiState<InMemoryStorage>, Arc<InMemoryStorage>) {
    let storage = Arc::new(InMemoryStorage::new());
    let mut scheduler = Scheduler::with_storage(Arc::clone(&storage));

    // Create a simple test job
    let dag = Dag::new("test_dag", "Test DAG");
    let job = Job::new("test_job", "Test Job", dag);
    scheduler.register(job.clone());

    let (handle, _task) = scheduler.start().await;

    let state = create_api_state(handle, Arc::clone(&storage), vec![job]);
    (state, storage)
}

/// Test: Health endpoint responds with status ok.
#[tokio::test]
async fn test_health_endpoint() {
    let (state, _storage) = create_test_state().await;
    let router = build_router(state);

    let request = Request::builder()
        .uri("/api/health")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "ok");
    assert!(json["version"].is_string());
}

/// Test: Scheduler state endpoint returns running state.
#[tokio::test]
async fn test_scheduler_state_endpoint() {
    let (state, _storage) = create_test_state().await;
    let router = build_router(state);

    let request = Request::builder()
        .uri("/api/scheduler/state")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["state"], "running");
    assert_eq!(json["is_running"], true);
    assert_eq!(json["is_paused"], false);
}

/// Test: List jobs endpoint returns registered jobs.
#[tokio::test]
async fn test_list_jobs_endpoint() {
    let (state, _storage) = create_test_state().await;
    let router = build_router(state);

    let request = Request::builder()
        .uri("/api/jobs")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["count"], 1);
    assert!(json["jobs"].is_array());
    assert_eq!(json["jobs"][0]["id"], "test_job");
    assert_eq!(json["jobs"][0]["name"], "Test Job");
}

/// Test: Get specific job by ID.
#[tokio::test]
async fn test_get_job_endpoint() {
    let (state, _storage) = create_test_state().await;
    let router = build_router(state);

    let request = Request::builder()
        .uri("/api/jobs/test_job")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["id"], "test_job");
    assert_eq!(json["name"], "Test Job");
}

/// Test: Get non-existent job returns 404.
#[tokio::test]
async fn test_get_nonexistent_job_returns_404() {
    let (state, _storage) = create_test_state().await;
    let router = build_router(state);

    let request = Request::builder()
        .uri("/api/jobs/nonexistent")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// Test: Pause and resume scheduler via API.
#[tokio::test]
async fn test_pause_resume_scheduler() {
    let (state, _storage) = create_test_state().await;
    let router = build_router(state);

    // Pause the scheduler
    let request = Request::builder()
        .method("POST")
        .uri("/api/scheduler/pause")
        .body(Body::empty())
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Check state is paused
    let request = Request::builder()
        .uri("/api/scheduler/state")
        .body(Body::empty())
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["is_paused"], true);

    // Resume the scheduler
    let request = Request::builder()
        .method("POST")
        .uri("/api/scheduler/resume")
        .body(Body::empty())
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Check state is running again
    let request = Request::builder()
        .uri("/api/scheduler/state")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["is_running"], true);
}

/// Test: Trigger job creates a run.
#[tokio::test]
async fn test_trigger_job_endpoint() {
    let (state, _storage) = create_test_state().await;
    let router = build_router(state);

    let request = Request::builder()
        .method("POST")
        .uri("/api/jobs/test_job/trigger")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["job_id"], "test_job");
    assert!(json["run_id"].is_string());
    assert!(json["message"].as_str().unwrap().contains("triggered"));
}

/// Test: List runs for a job.
#[tokio::test]
async fn test_list_runs_endpoint() {
    let (state, storage) = create_test_state().await;
    let router = build_router(state);

    // First trigger a job to create a run
    let request = Request::builder()
        .method("POST")
        .uri("/api/jobs/test_job/trigger")
        .body(Body::empty())
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let run_id_str = json["run_id"].as_str().unwrap();
    let run_id = RunId::from_string(run_id_str).unwrap();

    // Wait for the run to be recorded with a status (any status is fine)
    // We just need to ensure it's been persisted to storage
    wait_for_run_status(
        storage.as_ref(),
        &run_id,
        RunStatus::Completed,
        Duration::from_secs(5),
    )
    .await;

    // Now list runs
    let request = Request::builder()
        .uri("/api/jobs/test_job/runs")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert!(json["runs"].is_array());
    // At least one run should exist
    assert!(json["count"].as_u64().unwrap() >= 1);
}

/// Test: Invalid run ID returns appropriate error.
#[tokio::test]
async fn test_invalid_run_id_returns_error() {
    let (state, _storage) = create_test_state().await;
    let router = build_router(state);

    let request = Request::builder()
        .uri("/api/runs/not-a-valid-uuid")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    // Should return 404 (or could be 400 for bad request)
    assert!(
        response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::BAD_REQUEST
    );
}
