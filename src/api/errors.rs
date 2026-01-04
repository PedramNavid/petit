//! API error types and HTTP status mapping.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::scheduler::SchedulerError;
use crate::storage::StorageError;

/// API error response body.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

/// API error type that can be converted to HTTP responses.
#[derive(Debug)]
pub enum ApiError {
    /// Resource not found.
    NotFound(String),
    /// Request conflict (e.g., dependency not satisfied).
    Conflict(String),
    /// Too many requests (e.g., concurrency limit exceeded).
    TooManyRequests(String),
    /// Service unavailable (e.g., scheduler not running).
    ServiceUnavailable(String),
    /// Internal server error.
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, "CONFLICT", msg),
            ApiError::TooManyRequests(msg) => {
                (StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS", msg)
            }
            ApiError::ServiceUnavailable(msg) => {
                (StatusCode::SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE", msg)
            }
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", msg),
        };

        let body = ErrorResponse {
            error: message,
            code: code.to_string(),
        };

        (status, Json(body)).into_response()
    }
}

impl From<SchedulerError> for ApiError {
    fn from(err: SchedulerError) -> Self {
        match err {
            SchedulerError::JobNotFound(msg) => ApiError::NotFound(msg),
            SchedulerError::DependencyNotSatisfied(msg) => {
                ApiError::Conflict(format!("dependency not satisfied: {}", msg))
            }
            SchedulerError::MaxConcurrentRunsExceeded(msg) => {
                ApiError::TooManyRequests(format!("max concurrent runs exceeded: {}", msg))
            }
            SchedulerError::Storage(e) => e.into(),
            SchedulerError::ChannelError(msg) => ApiError::Internal(msg),
        }
    }
}

impl From<StorageError> for ApiError {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::NotFound(msg) => ApiError::NotFound(msg),
            StorageError::DuplicateKey(msg) => ApiError::Conflict(msg),
            _ => ApiError::Internal(err.to_string()),
        }
    }
}
