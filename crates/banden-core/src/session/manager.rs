//! Session manager: orchestrates the lifecycle defined by
//! [`crate::session::machine`] on top of a [`ControlBackend`] and the
//! recovery manager.
//!
//! Ordering rules (safety-critical):
//! 1. The session is registered with the durable recovery journal *before*
//!    the backend mutates any system state.
//! 2. Restoration and verification always run, even when the stop is the
//!    result of an error.
//! 3. A session never disappears silently: it always ends in `Completed`
//!    or `Failed` with a recorded reason.

use crate::error::{CoreError, CoreResult};
use crate::events::{self, CoreEvent};
use crate::models::{EventCategory, SessionConfig, SessionRecordView};
use crate::ports::{Clock, ControlBackend, EventSink};
use crate::recovery::manager::RecoveryManager;
use crate::session::machine::{SessionState, SessionStateMachine, Transition};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Internal mutable session record.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: uuid::Uuid,
    pub config: SessionConfig,
    pub machine: SessionStateMachine,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl Session {
    pub fn view(&self) -> SessionRecordView {
        let state_changed_at = self
            .machine
            .history()
            .last()
            .map(|t| t.at)
            .unwrap_or(self.created_at);
        SessionRecordView {
            id: self.id,
            config: self.config.clone(),
            state: self.machine.state(),
            created_at: self.created_at,
            state_changed_at,
            started_at: self.started_at,
            ended_at: self.ended_at,
            error: self.error.clone(),
            active_duration_secs: self.compute_active_duration(state_changed_at),
        }
    }

    fn compute_active_duration(&self, now: DateTime<Utc>) -> Option<u64> {
        let start = self.started_at?;
        let end = self.ended_at.unwrap_or(now);
        Some((end - start).num_seconds().max(0) as u64)
    }
}

/// Payload shape for session start requests (transport-neutral).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartSessionRequest {
    pub config: SessionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    UserRequested,
    DurationElapsed,
    EmergencyStop,
    Fault(String),
}

impl StopReason {
    fn describe(&self) -> String {
        match self {
            StopReason::UserRequested => "user requested".into(),
            StopReason::DurationElapsed => "duration elapsed".into(),
            StopReason::EmergencyStop => "emergency stop".into(),
            StopReason::Fault(f) => f.clone(),
        }
    }
}

pub struct SessionManager {
    backend: Arc<dyn ControlBackend>,
    recovery: Arc<RecoveryManager>,
    sink: Arc<dyn EventSink>,
    clock: Arc<dyn Clock>,
    sessions: Mutex<HashMap<uuid::Uuid, Session>>,
    cancellations: Mutex<HashMap<uuid::Uuid, CancellationToken>>,
}

impl SessionManager {
    pub fn new(
        backend: Arc<dyn ControlBackend>,
        recovery: Arc<RecoveryManager>,
        sink: Arc<dyn EventSink>,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            backend,
            recovery,
            sink,
            clock,
            sessions: Mutex::new(HashMap::new()),
            cancellations: Mutex::new(HashMap::new()),
        })
    }

    pub async fn list(&self) -> Vec<SessionRecordView> {
        let mut views: Vec<SessionRecordView> = self
            .sessions
            .lock()
            .await
            .values()
            .map(|s| s.view())
            .collect();
        views.sort_by_key(|v| std::cmp::Reverse(v.created_at));
        views
    }

    pub async fn get(&self, id: uuid::Uuid) -> CoreResult<SessionRecordView> {
        self.sessions
            .lock()
            .await
            .get(&id)
            .map(|s| s.view())
            .ok_or(CoreError::SessionNotFound(id))
    }

    async fn snapshot(&self, id: uuid::Uuid) -> CoreResult<Session> {
        self.sessions
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or(CoreError::SessionNotFound(id))
    }

    /// Sessions currently considered live (Active or mid-shutdown).
    pub async fn active_ids(&self) -> Vec<uuid::Uuid> {
        self.sessions
            .lock()
            .await
            .iter()
            .filter(|(_, s)| matches!(s.machine.state(), SessionState::Active))
            .map(|(id, _)| *id)
            .collect()
    }

    /// All sessions not in a terminal state.
    pub async fn non_terminal_sessions(&self) -> Vec<SessionRecordView> {
        self.sessions
            .lock()
            .await
            .values()
            .filter(|s| !s.machine.state().is_terminal())
            .map(|s| s.view())
            .collect()
    }

    pub async fn cancellation_token(&self, id: uuid::Uuid) -> CancellationToken {
        self.cancellations
            .lock()
            .await
            .get(&id)
            .cloned()
            .unwrap_or_default()
    }

    /// Create and drive a session through `Preparing -> Active`.
    pub async fn start(&self, config: SessionConfig) -> CoreResult<SessionRecordView> {
        config.validate()?;

        let id = uuid::Uuid::new_v4();
        let now = self.clock.now();
        let session = Session {
            id,
            config: config.clone(),
            machine: SessionStateMachine::new(),
            created_at: now,
            started_at: None,
            ended_at: None,
            error: None,
        };
        let target_label = session_label(&session);

        tracing::info!(session = %id, target = %target_label, "session starting");
        self.sessions.lock().await.insert(id, session);

        self.sink
            .emit(CoreEvent::session_created(id, config.clone()))
            .await;
        self.record_activity(format!("Control session created for {target_label}"))
            .await;

        // 1. Capture state + plan restoration BEFORE mutating anything.
        let prepared = self.snapshot(id).await?;
        let captured = match self.backend.prepare(&prepared).await {
            Ok(c) => c,
            Err(e) => {
                self.fail_session(id, format!("prepare failed: {e}"))
                    .await?;
                return Err(e);
            }
        };

        // 2. Register with the recovery journal (durable) before apply.
        self.recovery.register_session(id, captured).await;

        // 3. Apply the control.
        let to_apply = self.snapshot(id).await?;
        if let Err(e) = self.backend.apply(&to_apply).await {
            let _ = self.recovery.restore_session(id).await;
            self.recovery.clear_session(id).await;
            self.fail_session(id, format!("apply failed: {e}")).await?;
            return Err(e);
        }

        // 4. Transition to Active.
        self.transition(id, SessionState::Active, None).await?;
        {
            let mut sessions = self.sessions.lock().await;
            if let Some(s) = sessions.get_mut(&id) {
                s.started_at = Some(self.clock.now());
            }
        }
        self.record_activity(format!("Control session started for {target_label}"))
            .await;

        if self.snapshot(id).await?.config.duration_secs.is_some() {
            let token = CancellationToken::new();
            self.cancellations.lock().await.insert(id, token);
        }

        self.get(id).await
    }

    /// Cooperative stop used by the UI. Runs the full
    /// Stopping -> Restoring -> Verifying -> Completed pipeline.
    pub async fn stop(&self, id: uuid::Uuid, reason: StopReason) -> CoreResult<SessionRecordView> {
        if let Some(token) = self.cancellations.lock().await.remove(&id) {
            token.cancel();
        }

        let state = self.snapshot(id).await?.machine.state();
        match state {
            SessionState::Preparing => {
                // Nothing was applied yet; empty restore + verify, then done.
                self.transition(id, SessionState::Stopping, Some(reason.describe()))
                    .await?;
                let _ = self.recovery.restore_session(id).await;
                self.transition(id, SessionState::Restoring, None).await?;
                self.recovery.verify_session(id).await?;
                self.transition(id, SessionState::Verifying, None).await?;
                self.recovery.clear_session(id).await;
                self.complete(id).await?;
            }
            SessionState::Active => {
                self.transition(id, SessionState::Stopping, Some(reason.describe()))
                    .await?;
                self.shutdown_session(id).await?;
            }
            SessionState::RecoveryRequired => {
                // Already faulted: skip Stopping, continue the recovery path.
                self.shutdown_session(id).await?;
            }
            SessionState::Stopping | SessionState::Restoring | SessionState::Verifying => {
                // A shutdown is already in flight; report current state.
                tracing::warn!(session = %id, state = %state, "stop requested while shutting down");
            }
            SessionState::Completed | SessionState::Failed => {
                return Err(CoreError::InvalidSessionState(id, state.to_string()));
            }
        }
        self.get(id).await
    }

    /// Shared tail of stop/emergency-stop: teardown, restore, verify, finish.
    /// Expects the session to be in `Stopping` or `RecoveryRequired`.
    pub async fn shutdown_session(&self, id: uuid::Uuid) -> CoreResult<()> {
        // Teardown the backend control (idempotent).
        let teardown_err = self.backend.teardown(&self.snapshot(id).await?).await.err();

        // Restoration is mandatory regardless of teardown outcome.
        if self.snapshot(id).await?.machine.state() == SessionState::Stopping {
            self.transition(id, SessionState::Restoring, None).await?;
        }
        let restore = self.recovery.restore_session(id).await;

        // Verification is mandatory regardless of restore outcome.
        if self.snapshot(id).await?.machine.state() == SessionState::Restoring {
            self.transition(id, SessionState::Verifying, None).await?;
        }
        let verify = self.recovery.verify_session(id).await;

        let failure: Option<String> = [
            teardown_err.map(|e| format!("teardown failed: {e}")),
            restore.err().map(|e| format!("restore failed: {e}")),
            verify.err().map(|e| format!("verification failed: {e}")),
        ]
        .into_iter()
        .flatten()
        .reduce(|a, b| format!("{a}; {b}"));

        if let Some(err) = &failure {
            tracing::error!(session = %id, error = %err, "session shutdown incomplete");
            let label = session_label(&self.snapshot(id).await?);
            self.record_activity(format!("Session for {label} requires recovery: {err}"))
                .await;
            // Move to RecoveryRequired (from Verifying or Stopping).
            if self.snapshot(id).await?.machine.state() == SessionState::Stopping {
                // Stopping -> Restoring -> Verifying is the only path.
                self.transition(id, SessionState::Restoring, None).await?;
                self.transition(id, SessionState::Verifying, None).await?;
            }
            self.transition(id, SessionState::RecoveryRequired, Some(err.clone()))
                .await?;

            // One automatic retry cycle; if that also fails, mark Failed.
            let retry = match self.recovery.restore_session(id).await {
                Ok(()) => self.recovery.verify_session(id).await,
                Err(e) => Err(e),
            };
            match retry {
                Ok(()) => {
                    self.recovery.clear_session(id).await;
                    self.transition(
                        id,
                        SessionState::Restoring,
                        Some("automatic recovery retry".into()),
                    )
                    .await?;
                    self.transition(id, SessionState::Verifying, None).await?;
                    self.complete(id).await?;
                }
                Err(retry_err) => {
                    let msg = format!("automatic recovery failed: {retry_err}");
                    self.fail_session(id, msg.clone()).await?;
                    return Err(CoreError::EmergencyStopFailed(msg));
                }
            }
        } else {
            self.recovery.clear_session(id).await;
            self.complete(id).await?;
        }
        Ok(())
    }

    /// Mark a session completed (expects Verifying state) and notify.
    async fn complete(&self, id: uuid::Uuid) -> CoreResult<()> {
        self.transition(id, SessionState::Completed, None).await?;
        let label = session_label(&self.snapshot(id).await?);
        self.record_activity(format!("Control session for {label} completed"))
            .await;
        self.sink
            .emit(CoreEvent {
                name: events::SESSION_COMPLETED.to_string(),
                payload: serde_json::json!({ "v": 1, "session_id": id }),
            })
            .await;
        self.cancellations.lock().await.remove(&id);
        Ok(())
    }

    /// Route a session into `Failed`, taking the shortest legal path.
    pub async fn fail_session(&self, id: uuid::Uuid, reason: String) -> CoreResult<()> {
        let mut state = self.snapshot(id).await?.machine.state();
        // Walk to a state from which Failed is legal (max 3 hops).
        for _ in 0..3 {
            if state.allowed_targets().contains(&SessionState::Failed) {
                break;
            }
            let next = match state {
                SessionState::Preparing | SessionState::Active => SessionState::Stopping,
                _ => break,
            };
            self.transition(id, next, Some(reason.clone())).await?;
            state = next;
        }
        self.transition(id, SessionState::Failed, Some(reason))
            .await?;
        self.cancellations.lock().await.remove(&id);
        Ok(())
    }

    /// Apply a transition if legal; emit the state-change event.
    async fn transition(
        &self,
        id: uuid::Uuid,
        to: SessionState,
        reason: Option<String>,
    ) -> CoreResult<Transition> {
        let now = self.clock.now();
        let (from, t) = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(&id)
                .ok_or(CoreError::SessionNotFound(id))?;
            let from = session.machine.state();
            let t = session.machine.transition(to, now, reason.clone())?;
            if to.is_terminal() {
                session.ended_at = Some(now);
                if to == SessionState::Failed {
                    session.error = reason.clone();
                }
            }
            (from, t)
        };
        if to == SessionState::Failed {
            let label = session_label(&self.snapshot(id).await?);
            if let Some(r) = &reason {
                self.record_activity(format!("Session for {label} failed: {r}"))
                    .await;
            }
        }
        self.sink
            .emit(CoreEvent::session_state_changed(id, from, to, reason))
            .await;
        Ok(t)
    }

    async fn record_activity(&self, message: String) {
        self.sink
            .emit(CoreEvent {
                name: events::ACTIVITY_INTERNAL.to_string(),
                payload: serde_json::json!({
                    "v": 1,
                    "category": EventCategory::Session,
                    "message": message,
                }),
            })
            .await;
    }
}

fn session_label(s: &Session) -> String {
    s.config
        .target_label
        .clone()
        .unwrap_or_else(|| s.config.target_ip.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::testing::{FixedClock, MemoryJournalStore, RecordingEventSink};
    use crate::recovery::manager::RecoveryManager;
    use crate::simulation::SimulatedControlBackend;
    use chrono::TimeZone;

    fn clock() -> Arc<FixedClock> {
        FixedClock::new(Utc.timestamp_opt(1_700_000_000, 0).unwrap())
    }

    pub(crate) async fn harness() -> (
        Arc<SessionManager>,
        Arc<RecoveryManager>,
        Arc<RecordingEventSink>,
        Arc<FixedClock>,
    ) {
        let sink = RecordingEventSink::new();
        let journal = MemoryJournalStore::new();
        let clock_fn = clock();
        let backend = Arc::new(SimulatedControlBackend::default());
        let recovery = RecoveryManager::new(
            backend.clone(),
            sink.clone(),
            journal.clone(),
            clock_fn.clone(),
        );
        let manager =
            SessionManager::new(backend, recovery.clone(), sink.clone(), clock_fn.clone());
        (manager, recovery, sink, clock_fn)
    }

    fn config() -> SessionConfig {
        SessionConfig {
            target_mac: "AA:BB:CC:DD:EE:FF".into(),
            target_ip: "192.168.1.20".into(),
            target_label: Some("Laptop".into()),
            download_limit_bps: Some(10_000_000),
            upload_limit_bps: Some(5_000_000),
            duration_secs: None,
            blocked_apps: Vec::new(),
            allowed_apps: Vec::new(),
            priority: None,
        }
    }

    #[tokio::test]
    async fn start_reaches_active_and_registers_journal() {
        let (manager, recovery, _sink, _clock) = harness().await;
        let view = manager.start(config()).await.unwrap();
        assert_eq!(view.state, SessionState::Active);
        assert_eq!(
            recovery.network_state().await,
            crate::models::NetworkStateKind::Modified
        );
        manager
            .stop(view.id, StopReason::UserRequested)
            .await
            .unwrap();
        assert_eq!(
            manager.get(view.id).await.unwrap().state,
            SessionState::Completed
        );
        assert_eq!(
            recovery.network_state().await,
            crate::models::NetworkStateKind::Normal
        );
    }

    #[tokio::test]
    async fn invalid_config_rejected_before_any_state_change() {
        let (manager, _r, _s, _c) = harness().await;
        let mut c = config();
        c.target_mac = "not-a-mac".into();
        assert!(manager.start(c).await.is_err());
        assert!(manager.list().await.is_empty());
    }

    #[tokio::test]
    async fn stopping_twice_is_idempotent() {
        let (manager, _r, _s, _c) = harness().await;
        let view = manager.start(config()).await.unwrap();
        manager
            .stop(view.id, StopReason::UserRequested)
            .await
            .unwrap();
        // Second stop on a completed session is an explicit error, not a panic.
        let err = manager
            .stop(view.id, StopReason::UserRequested)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "invalid_session_state");
    }

    #[tokio::test]
    async fn duration_elapsed_stop_completes() {
        let (manager, _r, _s, _c) = harness().await;
        let mut c = config();
        c.duration_secs = Some(1);
        let view = manager.start(c).await.unwrap();
        manager
            .stop(view.id, StopReason::DurationElapsed)
            .await
            .unwrap();
        assert_eq!(
            manager.get(view.id).await.unwrap().state,
            SessionState::Completed
        );
    }
}
