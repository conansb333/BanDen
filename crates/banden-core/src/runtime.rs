//! Core runtime: composes the session manager, recovery manager and the
//! emergency-stop orchestrator. The application layer (Tauri) instantiates
//! this once with concrete ports.

use crate::error::{CoreError, CoreResult};
use crate::events::{self, CoreEvent};
use crate::models::{
    ActivityEvent, AppSettings, EventCategory, NetworkStateKind, SessionRecordView, SystemStatus,
    SystemWarning,
};
use crate::ports::{ActivityLog, Clock, ControlBackend, EventSink, JournalStore};
use crate::recovery::manager::RecoveryManager;
use crate::session::manager::{SessionManager, StopReason};
use crate::session::SessionState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Stages of the emergency-stop pipeline, exposed to the UI one by one.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmergencyStopStage {
    Cancelling,
    StoppingControllers,
    RestoringNetworkState,
    VerifyingNetworkState,
    Completed,
    RecoveryRequired,
}

impl EmergencyStopStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            EmergencyStopStage::Cancelling => "cancelling",
            EmergencyStopStage::StoppingControllers => "stopping_controllers",
            EmergencyStopStage::RestoringNetworkState => "restoring_network_state",
            EmergencyStopStage::VerifyingNetworkState => "verifying_network_state",
            EmergencyStopStage::Completed => "completed",
            EmergencyStopStage::RecoveryRequired => "recovery_required",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EmergencyStopOutcome {
    pub stage: EmergencyStopStage,
    pub sessions_affected: Vec<uuid::Uuid>,
    pub failures: Vec<String>,
    pub network_state: NetworkStateKind,
}

/// Inputs the app layer provides when assembling [`SystemStatus`].
#[derive(Debug, Clone, Default)]
pub struct StatusInputs {
    pub network: crate::models::NetworkSummary,
    pub device_count: u64,
    pub online_devices: u64,
    pub capture_source: String,
    pub watchdog_running: bool,
    pub latency_ms: Option<u64>,
    pub download_rate_bps: f64,
    pub upload_rate_bps: f64,
}

pub struct CoreRuntime {
    pub sessions: Arc<SessionManager>,
    pub recovery: Arc<RecoveryManager>,
    pub activity: Arc<dyn ActivityLog>,
    backend: Arc<dyn ControlBackend>,
    sink: Arc<dyn EventSink>,
    journal: Arc<dyn JournalStore>,
    clock: Arc<dyn Clock>,
    warnings: Mutex<Vec<SystemWarning>>,
    shutdown_token: CancellationToken,
    emergency_lock: Mutex<()>,
    settings: Mutex<AppSettings>,
}

impl CoreRuntime {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        backend: Arc<dyn ControlBackend>,
        recovery: Arc<RecoveryManager>,
        sessions: Arc<SessionManager>,
        activity: Arc<dyn ActivityLog>,
        sink: Arc<dyn EventSink>,
        journal: Arc<dyn JournalStore>,
        clock: Arc<dyn Clock>,
        settings: AppSettings,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions,
            recovery,
            activity,
            backend,
            sink,
            journal,
            clock,
            warnings: Mutex::new(Vec::new()),
            shutdown_token: CancellationToken::new(),
            emergency_lock: Mutex::new(()),
            settings: Mutex::new(settings),
        })
    }

    pub fn shutdown_token(&self) -> &CancellationToken {
        &self.shutdown_token
    }

    pub async fn settings(&self) -> AppSettings {
        self.settings.lock().await.clone()
    }

    pub async fn set_settings(&self, s: AppSettings) {
        *self.settings.lock().await = s;
    }

    pub async fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    pub async fn record_activity(
        &self,
        category: EventCategory,
        message: String,
        details: Option<serde_json::Value>,
    ) {
        self.activity.append(category, message, details).await;
    }

    pub async fn push_warning(&self, code: &str, message: String) {
        let warning = SystemWarning {
            code: code.to_string(),
            message,
            timestamp: self.clock.now(),
        };
        tracing::warn!(code = warning.code, "{}", warning.message);
        let mut warnings = self.warnings.lock().await;
        warnings.retain(|w| w.code != warning.code);
        warnings.push(warning.clone());
        if warnings.len() > 50 {
            let excess = warnings.len() - 50;
            warnings.drain(0..excess);
        }
        self.sink
            .emit(CoreEvent {
                name: events::SYSTEM_WARNING.to_string(),
                payload: serde_json::json!({ "v": 1, "warning": warning }),
            })
            .await;
    }

    pub async fn clear_warning(&self, code: &str) {
        self.warnings.lock().await.retain(|w| w.code != code);
    }

    pub async fn recent_activity(
        &self,
        limit: u64,
        category: Option<EventCategory>,
        search: Option<String>,
    ) -> Vec<ActivityEvent> {
        self.activity.recent(limit, category, search).await
    }

    /// Check for leftover journal entries from a previous (crashed) run and
    /// restore them before the app becomes interactive.
    pub async fn startup_recover(&self) -> Vec<String> {
        let mut notes = Vec::new();
        let pending = self.journal.pending().await;
        if pending.is_empty() {
            return notes;
        }
        let count = pending.len();
        self.record_activity(
            EventCategory::Recovery,
            format!("Detected {count} unfinished restoration actions from a previous run"),
            None,
        )
        .await;
        let results = self.recovery.restore_all_pending().await;
        for r in results {
            if let Err(e) = r {
                notes.push(e.to_string());
            }
        }
        if notes.is_empty() {
            self.record_activity(
                EventCategory::Recovery,
                "Restored network state left behind by a previous run".into(),
                None,
            )
            .await;
        } else {
            self.push_warning(
                "startup_recovery_incomplete",
                "Network state left by a previous run could not be fully restored".into(),
            )
            .await;
        }
        notes
    }

    /// Start a session; arms the duration timer when configured.
    pub async fn start_session(
        self: &Arc<Self>,
        config: crate::models::SessionConfig,
    ) -> CoreResult<SessionRecordView> {
        let duration = config.duration_secs;
        let view = self.sessions.start(config).await?;
        if let Some(secs) = duration {
            let rt = Arc::downgrade(self);
            let id = view.id;
            tokio::spawn(async move {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(secs)) => {}
                    _ = async {
                        while let Some(r) = rt.upgrade() {
                            if r.shutdown_token().is_cancelled() {
                                break;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        }
                    } => {}
                }
                if let Some(rt) = rt.upgrade() {
                    let still_active = rt
                        .sessions
                        .get(id)
                        .await
                        .map(|v| v.state == SessionState::Active)
                        .unwrap_or(false);
                    if still_active {
                        tracing::info!(session = %id, "duration elapsed, stopping");
                        if let Err(e) = rt.sessions.stop(id, StopReason::DurationElapsed).await {
                            tracing::error!(session = %id, error = %e, "duration stop failed");
                        }
                    }
                }
            });
        }
        Ok(view)
    }

    pub async fn stop_session(
        &self,
        id: uuid::Uuid,
        reason: StopReason,
    ) -> CoreResult<SessionRecordView> {
        self.sessions.stop(id, reason).await
    }

    /// Graceful shutdown of every non-terminal session. Used on app exit.
    pub async fn shutdown_all(&self, reason: StopReason) -> Vec<CoreResult<SessionRecordView>> {
        let ids: Vec<uuid::Uuid> = self
            .sessions
            .non_terminal_sessions()
            .await
            .into_iter()
            .map(|s| s.id)
            .collect();
        let mut results = Vec::new();
        for id in ids {
            results.push(self.sessions.stop(id, reason.clone()).await);
        }
        results
    }

    /// The emergency stop pipeline. Serialized by `emergency_lock` so a
    /// double press cannot interleave two restore passes.
    pub async fn emergency_stop(&self) -> CoreResult<EmergencyStopOutcome> {
        let _guard = self.emergency_lock.lock().await;
        tracing::warn!("EMERGENCY STOP initiated");
        self.record_activity(
            EventCategory::Recovery,
            "Emergency stop initiated".into(),
            None,
        )
        .await;

        let stage = |s: EmergencyStopStage, detail: &str| {
            self.sink.emit(CoreEvent {
                name: events::RECOVERY_PROGRESS.to_string(),
                payload: serde_json::json!({
                    "v": 1,
                    "stage": s.as_str(),
                    "detail": detail,
                }),
            })
        };

        // 1. Cancel pending operations.
        stage(
            EmergencyStopStage::Cancelling,
            "cancelling active operations",
        )
        .await;
        let non_terminal: Vec<SessionRecordView> = self.sessions.non_terminal_sessions().await;
        let affected: Vec<uuid::Uuid> = non_terminal.iter().map(|s| s.id).collect();

        // 2. Stop every control engine (full stop pipeline per session).
        stage(
            EmergencyStopStage::StoppingControllers,
            &format!("stopping {} session(s)", affected.len()),
        )
        .await;
        let mut failures: Vec<String> = Vec::new();
        for view in &non_terminal {
            match self.sessions.stop(view.id, StopReason::EmergencyStop).await {
                Ok(_) => {}
                Err(CoreError::SessionNotFound(_)) => {}
                Err(e) => failures.push(format!("session {}: {e}", view.id)),
            }
        }

        // 3. Restore anything the journal still holds.
        stage(
            EmergencyStopStage::RestoringNetworkState,
            "restoring network state",
        )
        .await;
        for result in self.recovery.restore_all_pending().await {
            if let Err(e) = result {
                failures.push(e.to_string());
            }
        }

        // 4. Verify the resulting network state.
        stage(
            EmergencyStopStage::VerifyingNetworkState,
            "verifying network state",
        )
        .await;
        let network_state = self.recovery.network_state().await;
        let journal_clean = !self.recovery.has_pending().await;
        let all_terminal = self.sessions.non_terminal_sessions().await.is_empty();

        let outcome_stage = if failures.is_empty()
            && journal_clean
            && all_terminal
            && network_state == NetworkStateKind::Normal
        {
            EmergencyStopStage::Completed
        } else {
            if network_state == NetworkStateKind::Unknown || !journal_clean {
                failures.push("network state could not be fully verified".into());
            }
            if !all_terminal {
                failures.push("some sessions did not reach a terminal state".into());
            }
            EmergencyStopStage::RecoveryRequired
        };

        stage(outcome_stage, &format!("{} failure(s)", failures.len())).await;
        self.sink
            .emit(CoreEvent {
                name: if outcome_stage == EmergencyStopStage::Completed {
                    events::RECOVERY_COMPLETED.to_string()
                } else {
                    events::RECOVERY_FAILED.to_string()
                },
                payload: serde_json::json!({ "v": 1, "outcome": outcome_stage.as_str() }),
            })
            .await;

        let message = if outcome_stage == EmergencyStopStage::Completed {
            "Emergency stop completed; network state verified".to_string()
        } else {
            format!(
                "Emergency stop finished with recovery required: {}",
                failures.join("; ")
            )
        };
        let category = if outcome_stage == EmergencyStopStage::Completed {
            EventCategory::Recovery
        } else {
            EventCategory::Error
        };
        self.record_activity(category, message, None).await;

        Ok(EmergencyStopOutcome {
            stage: outcome_stage,
            sessions_affected: affected,
            failures,
            network_state,
        })
    }

    /// Assemble the aggregate system status.
    pub async fn status(&self, inputs: StatusInputs) -> SystemStatus {
        let active = self.sessions.active_ids().await.len() as u64;
        let warnings = self.warnings.lock().await.clone();
        SystemStatus {
            network: inputs.network,
            network_state: self.recovery.network_state().await,
            device_count: inputs.device_count,
            online_devices: inputs.online_devices,
            active_sessions: active,
            capture_source: inputs.capture_source,
            control_backend: self.backend.name().to_string(),
            watchdog_running: inputs.watchdog_running,
            latency_ms: inputs.latency_ms,
            download_rate_bps: inputs.download_rate_bps,
            upload_rate_bps: inputs.upload_rate_bps,
            warnings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::testing::{
        FixedClock, MemoryActivityLog, MemoryJournalStore, RecordingEventSink,
    };
    use crate::recovery::manager::RecoveryManager;
    use crate::session::manager::SessionManager;
    use crate::simulation::SimulatedControlBackend;
    use chrono::{TimeZone, Utc};

    async fn runtime() -> (Arc<CoreRuntime>, Arc<RecordingEventSink>) {
        let sink = RecordingEventSink::new();
        let journal = MemoryJournalStore::new();
        let clock = FixedClock::new(Utc.timestamp_opt(1_700_000_000, 0).unwrap());
        let backend = Arc::new(SimulatedControlBackend::new());
        let recovery = RecoveryManager::new(
            backend.clone(),
            sink.clone(),
            journal.clone(),
            clock.clone(),
        );
        let sessions = SessionManager::new(
            backend.clone(),
            recovery.clone(),
            sink.clone(),
            clock.clone(),
        );
        let activity = MemoryActivityLog::new();
        let rt = CoreRuntime::new(
            backend,
            recovery,
            sessions,
            activity,
            sink.clone(),
            journal,
            clock,
            AppSettings::default(),
        );
        (rt, sink)
    }

    fn config(target_ip: &str) -> crate::models::SessionConfig {
        crate::models::SessionConfig {
            target_mac: "AA:BB:CC:DD:EE:FF".into(),
            target_ip: target_ip.into(),
            target_label: Some("Laptop".into()),
            download_limit_bps: Some(1_000_000),
            upload_limit_bps: None,
            duration_secs: None,
            blocked_apps: Vec::new(),
            allowed_apps: Vec::new(),
            priority: None,
        }
    }

    #[tokio::test]
    async fn emergency_stop_with_no_sessions_completes() {
        let (rt, _sink) = runtime().await;
        let outcome = rt.emergency_stop().await.unwrap();
        assert_eq!(outcome.stage, EmergencyStopStage::Completed);
        assert!(outcome.failures.is_empty());
    }

    #[tokio::test]
    async fn emergency_stop_stops_active_sessions_and_verifies() {
        let (rt, sink) = runtime().await;
        let a = rt.start_session(config("192.168.1.20")).await.unwrap();
        let b = rt.start_session(config("192.168.1.21")).await.unwrap();
        assert_eq!(a.state, SessionState::Active);

        let outcome = rt.emergency_stop().await.unwrap();
        assert_eq!(outcome.stage, EmergencyStopStage::Completed);
        assert_eq!(outcome.sessions_affected.len(), 2);
        assert_eq!(
            rt.sessions.get(a.id).await.unwrap().state,
            SessionState::Completed
        );
        assert_eq!(
            rt.sessions.get(b.id).await.unwrap().state,
            SessionState::Completed
        );
        assert_eq!(rt.recovery.network_state().await, NetworkStateKind::Normal);

        let names = sink.names().await;
        assert!(names.contains(&events::RECOVERY_COMPLETED.to_string()));
        assert!(names.contains(&events::RECOVERY_PROGRESS.to_string()));
    }

    #[tokio::test]
    async fn emergency_stop_reports_recovery_required_on_verify_failure() {
        use crate::simulation::FlakyControlBackend;
        let sink = RecordingEventSink::new();
        let journal = MemoryJournalStore::new();
        let clock = FixedClock::new(Utc.timestamp_opt(1_700_000_000, 0).unwrap());
        let backend = Arc::new(FlakyControlBackend {
            fail_prepare: false,
            fail_apply: false,
            fail_teardown: true,
            fail_verify: true,
            ..Default::default()
        });
        let recovery = RecoveryManager::new(
            backend.clone(),
            sink.clone(),
            journal.clone(),
            clock.clone(),
        );
        let sessions = SessionManager::new(
            backend.clone(),
            recovery.clone(),
            sink.clone(),
            clock.clone(),
        );
        let rt = CoreRuntime::new(
            backend,
            recovery,
            sessions,
            MemoryActivityLog::new(),
            sink,
            journal,
            clock,
            AppSettings::default(),
        );

        let s = rt.start_session(config("192.168.1.30")).await.unwrap();
        assert_eq!(s.state, SessionState::Active);
        let outcome = rt.emergency_stop().await.unwrap();
        assert_eq!(outcome.stage, EmergencyStopStage::RecoveryRequired);
        assert!(!outcome.failures.is_empty());
        // The session must still be in an explicit state, never floating.
        let final_state = rt.sessions.get(s.id).await.unwrap().state;
        assert!(final_state.is_terminal());
    }

    #[tokio::test]
    async fn shutdown_all_used_for_graceful_exit() {
        let (rt, _sink) = runtime().await;
        rt.start_session(config("192.168.1.40")).await.unwrap();
        let results = rt.shutdown_all(StopReason::UserRequested).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok());
        assert_eq!(rt.recovery.network_state().await, NetworkStateKind::Normal);
    }
}
