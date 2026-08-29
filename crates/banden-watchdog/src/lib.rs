//! BanDen independent recovery watchdog.
//!
//! The watchdog is a *separate process* spawned by the main application.
//! It watches two liveness signals:
//!
//! 1. a heartbeat file the app touches periodically,
//! 2. the parent process handle.
//!
//! When both indicate the app is gone (or the heartbeat goes stale while
//! the parent cannot be proven alive) and the recovery journal still holds
//! pending restoration actions, the watchdog executes them itself. It does
//! not depend on the UI process being alive — that is the entire point.
//!
//! The watchdog deliberately knows nothing about the rest of the
//! application: only the journal store, the restoration executor and the
//! activity log.

use banden_core::{EventCategory, JournalStore, RestorationExecutor};
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Parent alive and heartbeat fresh.
    Healthy,
    /// Heartbeat file missing or stale.
    HeartbeatStale,
    /// Parent process no longer running.
    ParentDead,
    /// Liveness cannot be determined (treated as failure signal).
    Unknown,
}

/// Pure decision function — fully unit-tested without processes or files.
///
/// The heartbeat is the primary signal (written by the app's live loop);
/// the parent handle is secondary. A fresh heartbeat proves the app was
/// alive moments ago even when the process query fails.
pub fn evaluate(
    parent_alive: Option<bool>,
    heartbeat_age: Option<Duration>,
    stale_after: Duration,
) -> Decision {
    match (parent_alive, heartbeat_age) {
        (Some(false), _) => Decision::ParentDead,
        (_, Some(age)) if age > stale_after => Decision::HeartbeatStale,
        (_, Some(_)) => Decision::Healthy,
        (Some(true), None) => Decision::HeartbeatStale,
        (None, None) => Decision::Unknown,
    }
}

/// Is a Windows process alive? `None` when the OS answer is unavailable.
pub fn parent_alive(pid: u32) -> Option<bool> {
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut exit_code: u32 = 0;
        if GetExitCodeProcess(handle, &mut exit_code).is_err() {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            return None;
        }
        let _ = windows::Win32::Foundation::CloseHandle(handle);
        Some(exit_code == 259 /* STILL_ACTIVE */)
    }
}

/// Age of the heartbeat file's last modification; None when missing.
pub fn heartbeat_age(path: &Path) -> Option<Duration> {
    let modified = path.metadata().ok()?.modified().ok()?;
    SystemTime::now().duration_since(modified).ok()
}

pub struct WatchdogConfig {
    pub db_path: PathBuf,
    pub heartbeat_path: PathBuf,
    pub parent_pid: u32,
    pub poll_interval: Duration,
    pub stale_after: Duration,
}

impl WatchdogConfig {
    pub fn parse_args(args: &[String]) -> Result<Self, String> {
        let mut db_path = None;
        let mut heartbeat_path = None;
        let mut parent_pid = None;
        let mut poll_ms: u64 = 2000;
        let mut stale_secs: u64 = 15;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--db" if i + 1 < args.len() => {
                    db_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                }
                "--heartbeat" if i + 1 < args.len() => {
                    heartbeat_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                }
                "--parent-pid" if i + 1 < args.len() => {
                    parent_pid = args[i + 1]
                        .parse()
                        .map(Some)
                        .map_err(|e| format!("bad pid: {e}"))?;
                    i += 2;
                }
                "--interval-ms" if i + 1 < args.len() => {
                    poll_ms = args[i + 1]
                        .parse()
                        .map_err(|e| format!("bad interval: {e}"))?;
                    i += 2;
                }
                "--stale-secs" if i + 1 < args.len() => {
                    stale_secs = args[i + 1]
                        .parse()
                        .map_err(|e| format!("bad stale-secs: {e}"))?;
                    i += 2;
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }

        Ok(Self {
            db_path: db_path.ok_or("--db is required")?,
            heartbeat_path: heartbeat_path.ok_or("--heartbeat is required")?,
            parent_pid: parent_pid.ok_or("--parent-pid is required")?,
            poll_interval: Duration::from_millis(poll_ms.max(250)),
            stale_after: Duration::from_secs(stale_secs.max(3)),
        })
    }
}

/// Outcome of one watchdog recovery run.
#[derive(Debug, Default)]
pub struct RecoverySummary {
    pub executed: usize,
    pub succeeded: usize,
    pub failed: usize,
}

/// Execute all pending journal actions using the provided executor.
/// Shared between the watchdog main loop and tests.
pub async fn run_recovery(
    store: &dyn JournalStore,
    activity: &dyn banden_core::ActivityLog,
    executor: &dyn RestorationExecutor,
) -> RecoverySummary {
    let mut summary = RecoverySummary::default();
    let pending = store.pending().await;
    if pending.is_empty() {
        return summary;
    }
    tracing::warn!(
        count = pending.len(),
        "watchdog: pending restoration actions found"
    );
    activity
        .append(
            EventCategory::Recovery,
            format!(
                "Watchdog detected abnormal termination; {} restoration action(s) pending",
                pending.len()
            ),
            None,
        )
        .await;

    for mut entry in pending {
        summary.executed += 1;
        match executor.execute(&entry.action).await {
            Ok(()) => {
                entry.state = banden_core::JournalEntryState::Done;
                entry.updated_at = Utc::now();
                store.update(&entry).await;
                summary.succeeded += 1;
                tracing::info!(action = %entry.action.describe(), "watchdog: restored");
            }
            Err(e) => {
                entry.attempts += 1;
                entry.updated_at = Utc::now();
                entry.last_error = Some(e.clone());
                if entry.attempts >= banden_core::recovery::journal::MAX_ATTEMPTS {
                    entry.state = banden_core::JournalEntryState::Failed;
                }
                store.update(&entry).await;
                summary.failed += 1;
                tracing::error!(action = %entry.action.describe(), error = %e, "watchdog: restoration failed");
            }
        }
    }

    let verdict = if summary.failed == 0 {
        format!(
            "Watchdog recovery finished: {} action(s) restored",
            summary.succeeded
        )
    } else {
        format!(
            "Watchdog recovery finished with {} failure(s); manual review required",
            summary.failed
        )
    };
    activity
        .append(
            if summary.failed == 0 {
                EventCategory::Recovery
            } else {
                EventCategory::Error
            },
            verdict,
            None,
        )
        .await;
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use banden_core::ports::testing::{MemoryActivityLog, MemoryJournalStore};
    use banden_core::{JournalEntry, RestorationAction};

    #[test]
    fn decision_table() {
        let stale = Duration::from_secs(10);
        assert_eq!(
            evaluate(Some(true), Some(Duration::from_secs(2)), stale),
            Decision::Healthy
        );
        assert_eq!(
            evaluate(Some(true), Some(Duration::from_secs(11)), stale),
            Decision::HeartbeatStale
        );
        assert_eq!(evaluate(Some(true), None, stale), Decision::HeartbeatStale);
        assert_eq!(
            evaluate(Some(false), Some(Duration::from_secs(1)), stale),
            Decision::ParentDead
        );
        assert_eq!(
            evaluate(None, Some(Duration::from_secs(1)), stale),
            Decision::Healthy
        );
        assert_eq!(evaluate(None, None, stale), Decision::Unknown);
    }

    struct OkExecutor;
    #[async_trait::async_trait]
    impl RestorationExecutor for OkExecutor {
        async fn execute(&self, _: &RestorationAction) -> Result<(), String> {
            Ok(())
        }
    }

    struct FailExecutor;
    #[async_trait::async_trait]
    impl RestorationExecutor for FailExecutor {
        async fn execute(&self, _: &RestorationAction) -> Result<(), String> {
            Err("nope".into())
        }
    }

    #[tokio::test]
    async fn recovery_executes_pending_actions() {
        let store = MemoryJournalStore::new();
        let activity = MemoryActivityLog::new();
        let id = uuid::Uuid::new_v4();
        store
            .append(JournalEntry::new(
                id,
                RestorationAction::NoOp { reason: "x".into() },
                Utc::now(),
            ))
            .await;
        let summary = run_recovery(store.as_ref(), activity.as_ref(), &OkExecutor).await;
        assert_eq!(summary.executed, 1);
        assert_eq!(summary.succeeded, 1);
        assert!(store.pending().await.is_empty());
    }

    #[tokio::test]
    async fn failing_actions_stay_pending_until_max_attempts() {
        let store = MemoryJournalStore::new();
        let activity = MemoryActivityLog::new();
        let id = uuid::Uuid::new_v4();
        store
            .append(JournalEntry::new(
                id,
                RestorationAction::NoOp { reason: "x".into() },
                Utc::now(),
            ))
            .await;
        for _ in 0..3 {
            let _ = run_recovery(store.as_ref(), activity.as_ref(), &FailExecutor).await;
        }
        let entries = store.entries_for_session(id).await;
        assert_eq!(entries[0].state, banden_core::JournalEntryState::Failed);
        assert_eq!(entries[0].attempts, 3);
    }

    #[test]
    fn args_parse_and_validate() {
        let cfg = WatchdogConfig::parse_args(&[
            "--db".into(),
            "x.db".into(),
            "--heartbeat".into(),
            "hb.txt".into(),
            "--parent-pid".into(),
            "42".into(),
        ])
        .unwrap();
        assert_eq!(cfg.parent_pid, 42);
        assert!(WatchdogConfig::parse_args(&[]).is_err());
    }

    #[test]
    fn heartbeat_age_for_missing_file_is_none() {
        assert!(heartbeat_age(Path::new("Z:/definitely/missing/hb.txt")).is_none());
    }
}
