//! Event sink: forwards core events to the UI (Tauri events) and the
//! activity log. Payloads stay exactly as the core produced them
//! (versioned JSON), so the contract is stable.

use async_trait::async_trait;
use banden_core::{CoreEvent, EventCategory, EventSink};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

pub struct TauriEventSink {
    app: AppHandle,
    db: Arc<banden_db::Db>,
}

impl TauriEventSink {
    pub fn new(app: AppHandle, db: Arc<banden_db::Db>) -> Self {
        Self { app, db }
    }
}

#[async_trait]
impl EventSink for TauriEventSink {
    async fn emit(&self, event: CoreEvent) {
        if event.name == banden_core::events::ACTIVITY_INTERNAL {
            // Translated into a durable activity entry, not broadcast.
            #[derive(serde::Deserialize)]
            struct Payload {
                #[serde(default)]
                category: String,
                #[serde(default)]
                message: String,
            }
            if let Ok(p) = serde_json::from_value::<Payload>(event.payload.clone()) {
                let category = match p.category.as_str() {
                    "WARNING" => EventCategory::Warning,
                    "ERROR" => EventCategory::Error,
                    "RECOVERY" => EventCategory::Recovery,
                    "NETWORK" => EventCategory::Network,
                    "SESSION" => EventCategory::Session,
                    _ => EventCategory::Info,
                };
                banden_core::ActivityLog::append(self.db.as_ref(), category, p.message, None).await;
            }
            return;
        }

        if let Err(e) = self.app.emit(&event.name, event.payload.clone()) {
            tracing::warn!(event = %event.name, error = %e, "failed to emit event");
        }
    }
}
