//! Command-level error type: user-friendly message + stable code.
//! Technical details go to tracing logs, never to the UI payload.

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl From<banden_core::CoreError> for CommandError {
    fn from(e: banden_core::CoreError) -> Self {
        tracing::error!(code = e.code(), error = %e, "core error");
        let message = match &e {
            banden_core::CoreError::SessionNotFound(_) => "That session no longer exists.".into(),
            banden_core::CoreError::InvalidSessionState(..) => {
                "The session is already shutting down or finished.".into()
            }
            banden_core::CoreError::InvalidConfig(detail) => {
                format!("Invalid configuration: {detail}")
            }
            banden_core::CoreError::Backend(detail) => {
                format!("The control backend reported a problem: {detail}")
            }
            banden_core::CoreError::RestorationFailed { action, .. } => format!(
                "Network restoration failed for: {action}. Recovery will continue; check Activity for details."
            ),
            banden_core::CoreError::VerificationFailed(_) => {
                "Network state could not be verified after restoration.".into()
            }
            banden_core::CoreError::EmergencyStopFailed(detail) => {
                format!("Emergency stop did not fully complete: {detail}")
            }
            _ => "An internal error occurred. Check the activity log for details.".into(),
        };
        Self::new(e.code(), message)
    }
}

impl From<banden_net::NetError> for CommandError {
    fn from(e: banden_net::NetError) -> Self {
        tracing::error!(error = %e, "network error");
        Self::new("network_error", "A Windows networking API call failed. The selected network interface may be unavailable.")
    }
}

impl From<banden_db::DbError> for CommandError {
    fn from(e: banden_db::DbError) -> Self {
        tracing::error!(error = %e, "database error");
        Self::new(
            "database_error",
            "BanDen could not access its local database.",
        )
    }
}

pub type CommandResult<T> = Result<T, CommandError>;
