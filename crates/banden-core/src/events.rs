//! Versioned, typed event payloads emitted by the core.
//!
//! Every payload carries a `v` field so the frontend can handle schema
//! evolution explicitly. The `ACTIVITY_INTERNAL` event is consumed by the
//! application layer to append to the activity log; it is not forwarded to
//! the UI.

use crate::models::SessionConfig;
use serde::{Deserialize, Serialize};

/// Event names on the Tauri bridge (single source of truth).
pub const DEVICE_DISCOVERED: &str = "device_discovered";
pub const DEVICE_UPDATED: &str = "device_updated";
pub const DEVICE_REMOVED: &str = "device_removed";
pub const TRAFFIC_UPDATE: &str = "traffic_update";
pub const SESSION_CREATED: &str = "session_created";
pub const SESSION_STATE_CHANGED: &str = "session_state_changed";
pub const SESSION_COMPLETED: &str = "session_completed";
pub const RECOVERY_STARTED: &str = "recovery_started";
pub const RECOVERY_PROGRESS: &str = "recovery_progress";
pub const RECOVERY_COMPLETED: &str = "recovery_completed";
pub const RECOVERY_FAILED: &str = "recovery_failed";
pub const NETWORK_STATE_CHANGED: &str = "network_state_changed";
pub const SYSTEM_WARNING: &str = "system_warning";
/// Internal: app layer appends it to the activity log.
pub const ACTIVITY_INTERNAL: &str = "_activity";

/// A domain event: transport-neutral name + versioned JSON payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreEvent {
    pub name: String,
    pub payload: serde_json::Value,
}

impl CoreEvent {
    pub fn session_created(id: uuid::Uuid, config: SessionConfig) -> Self {
        Self {
            name: SESSION_CREATED.to_string(),
            payload: serde_json::json!({ "v": 1, "session_id": id, "config": config }),
        }
    }

    pub fn session_state_changed(
        id: uuid::Uuid,
        from: crate::session::SessionState,
        to: crate::session::SessionState,
        reason: Option<String>,
    ) -> Self {
        Self {
            name: SESSION_STATE_CHANGED.to_string(),
            payload: serde_json::json!({
                "v": 1,
                "session_id": id,
                "from": from.as_str(),
                "to": to.as_str(),
                "reason": reason,
            }),
        }
    }
}
