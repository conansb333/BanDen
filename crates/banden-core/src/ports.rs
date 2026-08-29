//! Ports: traits through which the core depends on the outside world.
//!
//! The core never talks to SQLite, Tauri or Win32 directly. The application
//! layer implements these traits. Tests provide in-memory fakes
//! (see [`crate::ports::testing`]).

use crate::error::CoreResult;
use crate::events::CoreEvent;
use crate::models::{ActivityEvent, EventCategory};
use crate::recovery::journal::JournalEntry;
use crate::session::Session;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;

/// Sink receiving every domain event. The app layer forwards these to the
/// frontend (Tauri events) and to the activity log.
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: CoreEvent);
}

/// Persistence for the recovery journal. This is the store the watchdog
/// reads after an abnormal termination, so it must be durable (SQLite).
#[async_trait]
pub trait JournalStore: Send + Sync {
    async fn append(&self, entry: JournalEntry);
    async fn update(&self, entry: &JournalEntry);
    /// All entries that still require restoration work.
    async fn pending(&self) -> Vec<JournalEntry>;
    async fn entries_for_session(&self, session_id: uuid::Uuid) -> Vec<JournalEntry>;
    async fn clear_session(&self, session_id: uuid::Uuid);
}

/// User-facing activity log (distinct from developer logs).
#[async_trait]
pub trait ActivityLog: Send + Sync {
    async fn append(
        &self,
        category: EventCategory,
        message: String,
        details: Option<serde_json::Value>,
    );
    async fn recent(
        &self,
        limit: u64,
        category: Option<EventCategory>,
        search: Option<String>,
    ) -> Vec<ActivityEvent>;
}

/// A pluggable control backend. Implementations perform the actual
/// (potentially system-mutating) traffic control for a session.
///
/// Safety contract:
/// - `prepare` MUST capture everything needed to undo `apply` and return the
///   restoration actions before any system state is modified.
/// - `apply` MUST be idempotent per session.
/// - `teardown` MUST be safe to call multiple times.
/// - `verify_restoration` MUST fail (not return Ok) when it cannot prove
///   the system is back to the captured state.
#[async_trait]
pub trait ControlBackend: Send + Sync {
    fn name(&self) -> &'static str;

    async fn prepare(
        &self,
        session: &Session,
    ) -> CoreResult<crate::recovery::manager::CapturedState>;

    async fn apply(&self, session: &Session) -> CoreResult<()>;

    async fn teardown(&self, session: &Session) -> CoreResult<()>;

    async fn verify_restoration(
        &self,
        session_id: uuid::Uuid,
        captured: &crate::recovery::manager::CapturedState,
    ) -> CoreResult<()>;
}

/// Clock abstraction so time-dependent logic is testable.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// Wall-clock implementation.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub type Shared<T> = Arc<T>;

/// In-memory fakes used by unit tests across crates.
pub mod testing {
    use super::*;
    use std::sync::Mutex;
    use tokio::sync::RwLock;

    /// Records every emitted event for assertions.
    #[derive(Default)]
    pub struct RecordingEventSink {
        pub events: RwLock<Vec<CoreEvent>>,
    }

    impl RecordingEventSink {
        pub fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        pub async fn names(&self) -> Vec<String> {
            self.events
                .read()
                .await
                .iter()
                .map(|e| e.name.clone())
                .collect()
        }
    }

    #[async_trait]
    impl EventSink for RecordingEventSink {
        async fn emit(&self, event: CoreEvent) {
            self.events.write().await.push(event);
        }
    }

    /// In-memory journal store.
    #[derive(Default)]
    pub struct MemoryJournalStore {
        pub entries: Mutex<Vec<JournalEntry>>,
        pub next_id: std::sync::atomic::AtomicI64,
    }

    impl MemoryJournalStore {
        pub fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }
    }

    #[async_trait]
    impl JournalStore for MemoryJournalStore {
        async fn append(&self, entry: JournalEntry) {
            let mut e = entry;
            let id = self
                .next_id
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            e.id = id;
            self.entries.lock().unwrap().push(e);
        }

        async fn update(&self, entry: &JournalEntry) {
            let mut all = self.entries.lock().unwrap();
            if let Some(slot) = all.iter_mut().find(|e| e.id == entry.id) {
                *slot = entry.clone();
            }
        }

        async fn pending(&self) -> Vec<JournalEntry> {
            self.entries
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.state == crate::recovery::journal::JournalEntryState::Pending)
                .cloned()
                .collect()
        }

        async fn entries_for_session(&self, session_id: uuid::Uuid) -> Vec<JournalEntry> {
            self.entries
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.session_id == session_id)
                .cloned()
                .collect()
        }

        async fn clear_session(&self, session_id: uuid::Uuid) {
            self.entries
                .lock()
                .unwrap()
                .retain(|e| e.session_id != session_id);
        }
    }

    /// In-memory activity log.
    #[derive(Default)]
    pub struct MemoryActivityLog {
        pub events: Mutex<Vec<ActivityEvent>>,
        next_id: std::sync::atomic::AtomicI64,
    }

    impl MemoryActivityLog {
        pub fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }
    }

    #[async_trait]
    impl ActivityLog for MemoryActivityLog {
        async fn append(
            &self,
            category: EventCategory,
            message: String,
            details: Option<serde_json::Value>,
        ) {
            let id = self
                .next_id
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            self.events.lock().unwrap().push(ActivityEvent {
                id,
                timestamp: Utc::now(),
                category,
                message,
                details,
            });
        }

        async fn recent(
            &self,
            limit: u64,
            category: Option<EventCategory>,
            search: Option<String>,
        ) -> Vec<ActivityEvent> {
            let mut all: Vec<ActivityEvent> = self
                .events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| category.map_or(true, |c| e.category == c))
                .filter(|e| {
                    search.as_deref().map_or(true, |s| {
                        e.message.to_lowercase().contains(&s.to_lowercase())
                    })
                })
                .cloned()
                .collect();
            all.reverse();
            all.truncate(limit as usize);
            all
        }
    }

    /// Deterministic clock for tests.
    pub struct FixedClock(pub Mutex<DateTime<Utc>>);

    impl FixedClock {
        pub fn new(t: DateTime<Utc>) -> Arc<Self> {
            Arc::new(Self(Mutex::new(t)))
        }

        pub fn advance(&self, secs: i64) {
            let mut t = self.0.lock().unwrap();
            *t += chrono::Duration::seconds(secs);
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            *self.0.lock().unwrap()
        }
    }
}
