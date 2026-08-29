//! The recovery journal.
//!
//! Every system mutation a session plans is recorded here *before* it is
//! executed. The journal is the contract between the application and the
//! independent watchdog: if the main process dies, the watchdog replays the
//! pending entries to restore the network.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single, idempotent restoration step. Actions are intentionally small
/// and self-describing so a watchdog process can execute them without the
/// full application runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RestorationAction {
    /// Bookkeeping only: nothing external was modified.
    NoOp { reason: String },
    /// Remove a neighbor/ARP entry we added for `ip`.
    ClearNeighborEntry { ip: String },
    /// Restore a neighbor/ARP entry we replaced (`ip` should map to `mac`).
    RestoreNeighborEntry { ip: String, mac: String },
    /// Kill a process/service we started.
    StopProcess { name: String },
}

impl RestorationAction {
    pub fn describe(&self) -> String {
        match self {
            RestorationAction::NoOp { reason } => format!("no-op ({reason})"),
            RestorationAction::ClearNeighborEntry { ip } => {
                format!("clear neighbor entry for {ip}")
            }
            RestorationAction::RestoreNeighborEntry { ip, mac } => {
                format!("restore neighbor entry {ip} -> {mac}")
            }
            RestorationAction::StopProcess { name } => format!("stop process {name}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JournalEntryState {
    Pending,
    Done,
    /// Exhausted retries; requires human attention.
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JournalEntry {
    /// 0 for entries not yet persisted; assigned by the store.
    pub id: i64,
    pub session_id: uuid::Uuid,
    pub action: RestorationAction,
    pub state: JournalEntryState,
    pub attempts: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_error: Option<String>,
}

impl JournalEntry {
    pub fn new(session_id: uuid::Uuid, action: RestorationAction, now: DateTime<Utc>) -> Self {
        Self {
            id: 0,
            session_id,
            action,
            state: JournalEntryState::Pending,
            attempts: 0,
            created_at: now,
            updated_at: now,
            last_error: None,
        }
    }
}

pub const MAX_ATTEMPTS: u32 = 3;
