//! Recovery manager.
//!
//! Owns the global `NetworkState` classification and the restoration /
//! verification workflow. The manager never assumes success: verification
//! must be explicitly proven by the control backend, and when it cannot be,
//! the state becomes `Unknown` and the failure is surfaced.

use crate::error::{CoreError, CoreResult};
use crate::events::{self, CoreEvent};
use crate::models::NetworkStateKind;
use crate::ports::{Clock, ControlBackend, EventSink, JournalStore};
use crate::recovery::journal::{JournalEntryState, RestorationAction, MAX_ATTEMPTS};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// State captured before a backend mutates anything, plus the actions that
/// undo the mutation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct CapturedState {
    pub description: String,
    pub actions: Vec<RestorationAction>,
}

/// Executes restoration actions. Implemented by the network layer; the
/// watchdog uses its own implementation against the same journal format.
#[async_trait]
pub trait RestorationExecutor: Send + Sync {
    async fn execute(&self, action: &RestorationAction) -> Result<(), String>;
}

/// Default executor: only `NoOp` succeeds; real actions are refused so a
/// missing executor can never silently "restore" nothing.
#[derive(Default)]
pub struct StrictNoOpExecutor;

#[async_trait]
impl RestorationExecutor for StrictNoOpExecutor {
    async fn execute(&self, action: &RestorationAction) -> Result<(), String> {
        match action {
            RestorationAction::NoOp { .. } => Ok(()),
            other => Err(format!("no executor registered for action: {other:?}")),
        }
    }
}

pub use crate::models::NetworkStateKind as NetState;

struct Inner {
    network_state: NetworkStateKind,
    captured: HashMap<uuid::Uuid, CapturedState>,
}

pub struct RecoveryManager {
    backend: Arc<dyn ControlBackend>,
    executor: Arc<dyn RestorationExecutor>,
    sink: Arc<dyn EventSink>,
    journal: Arc<dyn JournalStore>,
    clock: Arc<dyn Clock>,
    inner: Mutex<Inner>,
}

impl RecoveryManager {
    pub fn new(
        backend: Arc<dyn ControlBackend>,
        sink: Arc<dyn EventSink>,
        journal: Arc<dyn JournalStore>,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Self::with_executor(backend, Arc::new(StrictNoOpExecutor), sink, journal, clock)
    }

    pub fn with_executor(
        backend: Arc<dyn ControlBackend>,
        executor: Arc<dyn RestorationExecutor>,
        sink: Arc<dyn EventSink>,
        journal: Arc<dyn JournalStore>,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            backend,
            executor,
            sink,
            journal,
            clock,
            inner: Mutex::new(Inner {
                network_state: NetworkStateKind::Normal,
                captured: HashMap::new(),
            }),
        })
    }

    pub async fn network_state(&self) -> NetworkStateKind {
        self.inner.lock().await.network_state
    }

    async fn set_network_state(&self, to: NetworkStateKind) {
        let from = {
            let mut inner = self.inner.lock().await;
            let from = inner.network_state;
            if from == to {
                return;
            }
            inner.network_state = to;
            from
        };
        tracing::info!(from = ?from, to = ?to, "network state changed");
        self.sink
            .emit(CoreEvent {
                name: events::NETWORK_STATE_CHANGED.to_string(),
                payload: serde_json::json!({ "v": 1, "from": from, "to": to }),
            })
            .await;
    }

    /// Register a session *before* its backend mutates system state.
    pub async fn register_session(&self, id: uuid::Uuid, captured: CapturedState) {
        let now = self.clock.now();
        for action in &captured.actions {
            self.journal
                .append(crate::recovery::journal::JournalEntry::new(
                    id,
                    action.clone(),
                    now,
                ))
                .await;
        }
        self.inner.lock().await.captured.insert(id, captured);
        self.set_network_state(NetworkStateKind::Modified).await;
    }

    /// Drop a fully-restored-and-verified session.
    pub async fn clear_session(&self, id: uuid::Uuid) {
        self.journal.clear_session(id).await;
        let now_empty = {
            let mut inner = self.inner.lock().await;
            inner.captured.remove(&id);
            inner.captured.is_empty()
        };
        if now_empty {
            self.set_network_state(NetworkStateKind::Normal).await;
        }
    }

    pub async fn has_pending(&self) -> bool {
        !self.journal.pending().await.is_empty()
    }

    /// Execute all pending restoration actions for one session.
    /// Retries are driven by the caller; each call here is one attempt per
    /// pending action.
    pub async fn restore_session(&self, id: uuid::Uuid) -> CoreResult<()> {
        self.set_network_state(NetworkStateKind::Restoring).await;
        self.sink
            .emit(CoreEvent {
                name: events::RECOVERY_STARTED.to_string(),
                payload: serde_json::json!({ "v": 1, "session_id": id }),
            })
            .await;

        let result = self.restore_pending_for(id).await;

        match &result {
            Ok(()) => {
                self.sink
                    .emit(CoreEvent {
                        name: events::RECOVERY_PROGRESS.to_string(),
                        payload: serde_json::json!({
                            "v": 1, "session_id": id, "stage": "restored", "detail": null
                        }),
                    })
                    .await;
            }
            Err(e) => {
                self.sink
                    .emit(CoreEvent {
                        name: events::RECOVERY_FAILED.to_string(),
                        payload: serde_json::json!({ "v": 1, "session_id": id, "error": e.to_string() }),
                    })
                    .await;
                self.set_network_state(NetworkStateKind::Unknown).await;
            }
        }
        // Between restore and verify the state is still "Modified" unless a
        // failure moved it to Unknown.
        if result.is_ok() && self.inner.lock().await.network_state == NetworkStateKind::Restoring {
            self.set_network_state(NetworkStateKind::Modified).await;
        }
        result
    }

    async fn restore_pending_for(&self, session_id: uuid::Uuid) -> CoreResult<()> {
        let entries = self.journal.entries_for_session(session_id).await;
        let mut first_failure: Option<CoreError> = None;
        for mut entry in entries {
            if entry.state != JournalEntryState::Pending {
                continue;
            }
            tracing::info!(session = %session_id, action = %entry.action.describe(), "restoring");
            match self.executor.execute(&entry.action).await {
                Ok(()) => {
                    entry.state = JournalEntryState::Done;
                    entry.updated_at = self.clock.now();
                    entry.last_error = None;
                    self.journal.update(&entry).await;
                }
                Err(e) => {
                    entry.attempts += 1;
                    entry.updated_at = self.clock.now();
                    entry.last_error = Some(e.clone());
                    if entry.attempts >= MAX_ATTEMPTS {
                        entry.state = JournalEntryState::Failed;
                    }
                    self.journal.update(&entry).await;
                    tracing::error!(session = %session_id, error = %e, "restoration action failed");
                    if first_failure.is_none() {
                        first_failure = Some(CoreError::RestorationFailed {
                            action: entry.action.describe(),
                            attempts: entry.attempts,
                            reason: e,
                        });
                    }
                }
            }
        }
        match first_failure {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Ask the backend to prove the system matches the captured state.
    pub async fn verify_session(&self, id: uuid::Uuid) -> CoreResult<()> {
        let captured = {
            let inner = self.inner.lock().await;
            inner.captured.get(&id).cloned()
        };
        let Some(captured) = captured else {
            // Unknown session: nothing captured, verification is vacuous
            // only if the journal holds nothing for it either.
            if self.journal.entries_for_session(id).await.is_empty() {
                return Ok(());
            }
            return Err(CoreError::VerificationFailed(
                "captured state missing while journal entries exist".into(),
            ));
        };

        match self.backend.verify_restoration(id, &captured).await {
            Ok(()) => {
                let still_pending = self
                    .journal
                    .entries_for_session(id)
                    .await
                    .iter()
                    .any(|e| e.state == JournalEntryState::Pending);
                if still_pending {
                    return Err(CoreError::VerificationFailed(
                        "journal still holds pending actions".into(),
                    ));
                }
                Ok(())
            }
            Err(e) => {
                self.set_network_state(NetworkStateKind::Unknown).await;
                Err(e)
            }
        }
    }

    /// Watchdog entry point: restore everything pending, journal-only
    /// (no backend verification is possible without app state).
    pub async fn restore_all_pending(&self) -> Vec<CoreResult<()>> {
        let ids: Vec<uuid::Uuid> = self
            .journal
            .pending()
            .await
            .into_iter()
            .map(|e| e.session_id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let mut results = Vec::new();
        for id in ids {
            results.push(self.restore_session(id).await);
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::testing::{FixedClock, MemoryJournalStore, RecordingEventSink};
    use crate::recovery::journal::{JournalEntry, RestorationAction};
    use crate::simulation::SimulatedControlBackend;
    use chrono::{TimeZone, Utc};
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountingExecutor {
        calls: AtomicU32,
        fail_first_n: u32,
    }

    #[async_trait]
    impl RestorationExecutor for CountingExecutor {
        async fn execute(&self, _action: &RestorationAction) -> Result<(), String> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.fail_first_n {
                Err("injected executor failure".into())
            } else {
                Ok(())
            }
        }
    }

    async fn harness(
        fail_first_n: u32,
    ) -> (
        Arc<RecoveryManager>,
        Arc<MemoryJournalStore>,
        Arc<CountingExecutor>,
    ) {
        let sink = RecordingEventSink::new();
        let journal = MemoryJournalStore::new();
        let clock = FixedClock::new(Utc.timestamp_opt(1_700_000_000, 0).unwrap());
        let backend = Arc::new(SimulatedControlBackend::new());
        let executor = Arc::new(CountingExecutor {
            calls: AtomicU32::new(0),
            fail_first_n,
        });
        let mgr =
            RecoveryManager::with_executor(backend, executor.clone(), sink, journal.clone(), clock);
        (mgr, journal, executor)
    }

    fn captured_with(action: RestorationAction) -> CapturedState {
        CapturedState {
            description: "test".into(),
            actions: vec![action],
        }
    }

    #[tokio::test]
    async fn register_marks_modified_and_journal_pending() {
        let (mgr, journal, _ex) = harness(0).await;
        let id = uuid::Uuid::new_v4();
        mgr.register_session(
            id,
            captured_with(RestorationAction::ClearNeighborEntry {
                ip: "192.168.1.20".into(),
            }),
        )
        .await;
        assert_eq!(mgr.network_state().await, NetworkStateKind::Modified);
        assert_eq!(journal.pending().await.len(), 1);
    }

    #[tokio::test]
    async fn restore_then_clear_returns_to_normal() {
        let (mgr, _j, _ex) = harness(0).await;
        let id = uuid::Uuid::new_v4();
        mgr.register_session(
            id,
            captured_with(RestorationAction::ClearNeighborEntry {
                ip: "192.168.1.20".into(),
            }),
        )
        .await;
        mgr.restore_session(id).await.unwrap();
        mgr.verify_session(id).await.unwrap();
        mgr.clear_session(id).await;
        assert_eq!(mgr.network_state().await, NetworkStateKind::Normal);
        assert!(!mgr.has_pending().await);
    }

    #[tokio::test]
    async fn failure_sets_unknown_and_retries_succeed() {
        let (mgr, journal, ex) = harness(1).await;
        let id = uuid::Uuid::new_v4();
        mgr.register_session(
            id,
            captured_with(RestorationAction::ClearNeighborEntry {
                ip: "10.0.0.5".into(),
            }),
        )
        .await;
        assert!(mgr.restore_session(id).await.is_err());
        assert_eq!(mgr.network_state().await, NetworkStateKind::Unknown);
        assert_eq!(ex.calls.load(Ordering::SeqCst), 1);
        // Second attempt: executor now succeeds.
        mgr.restore_session(id).await.unwrap();
        assert_eq!(ex.calls.load(Ordering::SeqCst), 2);
        assert!(journal.pending().await.is_empty());
        mgr.verify_session(id).await.unwrap();
        mgr.clear_session(id).await;
        assert_eq!(mgr.network_state().await, NetworkStateKind::Normal);
    }

    #[tokio::test]
    async fn exhausted_retries_mark_entry_failed() {
        let (mgr, journal, _ex) = harness(u32::MAX).await;
        let id = uuid::Uuid::new_v4();
        mgr.register_session(
            id,
            captured_with(RestorationAction::ClearNeighborEntry {
                ip: "10.0.0.6".into(),
            }),
        )
        .await;
        for _ in 0..3 {
            let _ = mgr.restore_session(id).await;
        }
        let entries = journal.entries_for_session(id).await;
        assert_eq!(entries[0].state, JournalEntryState::Failed);
        assert_eq!(entries[0].attempts, MAX_ATTEMPTS);
        // Entry is no longer pending (Failed), but must stay visible.
        assert!(journal.pending().await.is_empty());
    }

    #[tokio::test]
    async fn verify_without_captured_but_with_journal_fails() {
        let (mgr, journal, _ex) = harness(0).await;
        let id = uuid::Uuid::new_v4();
        journal
            .append(JournalEntry::new(
                id,
                RestorationAction::NoOp { reason: "x".into() },
                Utc::now(),
            ))
            .await;
        assert!(mgr.verify_session(id).await.is_err());
    }

    #[tokio::test]
    async fn restore_all_pending_covers_every_session() {
        let (mgr, _j, _ex) = harness(0).await;
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        mgr.register_session(
            a,
            captured_with(RestorationAction::NoOp { reason: "a".into() }),
        )
        .await;
        mgr.register_session(
            b,
            captured_with(RestorationAction::NoOp { reason: "b".into() }),
        )
        .await;
        let results = mgr.restore_all_pending().await;
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.is_ok()));
    }
}
