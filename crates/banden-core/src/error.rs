//! Structured error types for the BanDen core.

use thiserror::Error;

/// Errors produced by the domain core. These are technical errors; the
/// application layer maps them to user-friendly messages and keeps the
/// technical detail for logs.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("session {0} not found")]
    SessionNotFound(uuid::Uuid),

    #[error("invalid state transition: {from} -> {to}")]
    InvalidTransition {
        from: &'static str,
        to: &'static str,
    },

    #[error("session {0} is in state {1} which does not allow this operation")]
    InvalidSessionState(uuid::Uuid, String),

    #[error("control backend failed: {0}")]
    Backend(String),

    #[error("restoration action failed after {attempts} attempts: {action}: {reason}")]
    RestorationFailed {
        action: String,
        attempts: u32,
        reason: String,
    },

    #[error("restoration verification failed: {0}")]
    VerificationFailed(String),

    #[error("emergency stop failed: {0}")]
    EmergencyStopFailed(String),

    #[error("persistence error: {0}")]
    Persistence(String),

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl CoreError {
    /// Stable machine-readable code used by the API layer.
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::SessionNotFound(_) => "session_not_found",
            CoreError::InvalidTransition { .. } => "invalid_transition",
            CoreError::InvalidSessionState(..) => "invalid_session_state",
            CoreError::Backend(_) => "backend_failure",
            CoreError::RestorationFailed { .. } => "restoration_failed",
            CoreError::VerificationFailed(_) => "verification_failed",
            CoreError::EmergencyStopFailed(_) => "emergency_stop_failed",
            CoreError::Persistence(_) => "persistence_failure",
            CoreError::InvalidConfig(_) => "invalid_config",
            CoreError::Json(_) => "serialization_failure",
            CoreError::Io(_) => "io_failure",
        }
    }
}

pub type CoreResult<T> = Result<T, CoreError>;
