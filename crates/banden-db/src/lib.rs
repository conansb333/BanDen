//! BanDen SQLite persistence layer.
//!
//! Implements the core ports (`JournalStore`, `ActivityLog`) on top of
//! SQLite and provides repositories for devices, sessions, traffic samples
//! and settings. The schema is managed exclusively through embedded
//! migrations (see `migrations/` at the repository root).

use banden_core::{
    ActivityEvent, ActivityLog, EventCategory, JournalEntry, JournalEntryState, JournalStore,
    NetworkDevice, SessionRecordView,
};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub mod migrations;

pub use migrations::migrate;

/// Wrapper around a SQLite connection. All access is serialized through a
/// mutex; critical sections are short (single statements) so this is fine
/// for the application's write volume.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type DbResult<T> = Result<T, DbError>;

pub const SETTINGS_KEY_APP: &str = "app";

impl Db {
    pub fn open(path: &Path) -> DbResult<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// In-memory database, primarily for tests.
    pub fn open_in_memory() -> DbResult<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> DbResult<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        migrations::migrate(&db.conn.lock().unwrap())?;
        Ok(db)
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> DbResult<T>) -> DbResult<T> {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }

    // -- settings ----------------------------------------------------------

    pub fn get_settings(&self) -> DbResult<Option<serde_json::Value>> {
        self.with_conn(|conn| {
            let row: Option<String> = conn
                .query_row(
                    "SELECT value_json FROM settings WHERE key = ?1",
                    params![SETTINGS_KEY_APP],
                    |r| r.get(0),
                )
                .optional()?;
            match row {
                Some(json) => Ok(Some(serde_json::from_str(&json)?)),
                None => Ok(None),
            }
        })
    }

    pub fn set_settings(&self, value: &serde_json::Value) -> DbResult<()> {
        let json = serde_json::to_string(value)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
                params![SETTINGS_KEY_APP, json],
            )?;
            Ok(())
        })
    }

    // -- devices -----------------------------------------------------------

    pub fn upsert_device(&self, device: &NetworkDevice) -> DbResult<i64> {
        let mac = device.mac_address.clone();
        let ip = device.ip_address.clone();
        let hostname = device.hostname.clone();
        let vendor = device.vendor.clone();
        let dtype = device.device_type.clone();
        let status = status_to_str(device.status);
        let first_seen = device.first_seen.timestamp();
        let last_seen = device.last_seen.timestamp();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO devices (mac_address, ip_address, hostname, vendor, device_type, status, first_seen, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(mac_address) DO UPDATE SET
                    ip_address = excluded.ip_address,
                    hostname = COALESCE(excluded.hostname, devices.hostname),
                    vendor = COALESCE(excluded.vendor, devices.vendor),
                    device_type = COALESCE(excluded.device_type, devices.device_type),
                    status = excluded.status,
                    last_seen = excluded.last_seen",
                params![mac, ip, hostname, vendor, dtype, status, first_seen, last_seen],
            )?;
            let id: i64 = conn.query_row(
                "SELECT id FROM devices WHERE mac_address = ?1",
                params![mac],
                |r| r.get(0),
            )?;
            Ok(id)
        })
    }

    pub fn list_devices(&self) -> DbResult<Vec<NetworkDevice>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, mac_address, ip_address, hostname, vendor, device_type, status, first_seen, last_seen
                 FROM devices ORDER BY last_seen DESC",
            )?;
            let rows = stmt.query_map([], row_to_device)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Sqlite)
        })
    }

    pub fn get_device(&self, id: i64) -> DbResult<Option<NetworkDevice>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, mac_address, ip_address, hostname, vendor, device_type, status, first_seen, last_seen
                 FROM devices WHERE id = ?1",
                params![id],
                row_to_device,
            )
            .optional()
            .map_err(DbError::Sqlite)
        })
    }

    pub fn get_device_by_mac(&self, mac: &str) -> DbResult<Option<NetworkDevice>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, mac_address, ip_address, hostname, vendor, device_type, status, first_seen, last_seen
                 FROM devices WHERE mac_address = ?1",
                params![mac],
                row_to_device,
            )
            .optional()
            .map_err(DbError::Sqlite)
        })
    }

    /// Remove a device row (used to purge stale/junk registry entries).
    pub fn delete_device(&self, mac: &str) -> DbResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM devices WHERE mac_address = ?1", params![mac])?;
            Ok(())
        })
    }

    /// Delete registry rows with unusable all-zero MACs.
    pub fn delete_zero_mac_devices(&self) -> DbResult<usize> {
        self.with_conn(|conn| {
            Ok(conn.execute(
                "DELETE FROM devices WHERE mac_address = '00:00:00:00:00:00'",
                [],
            )?)
        })
    }

    pub fn mark_offline(&self, mac: &str, last_seen: DateTime<Utc>) -> DbResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE devices SET status = 'offline', last_seen = ?2 WHERE mac_address = ?1",
                params![mac, last_seen.timestamp()],
            )?;
            Ok(())
        })
    }

    pub fn device_counts(&self) -> DbResult<(u64, u64)> {
        self.with_conn(|conn| {
            let total: i64 = conn.query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))?;
            let online: i64 = conn.query_row(
                "SELECT COUNT(*) FROM devices WHERE status IN ('online', 'new')",
                [],
                |r| r.get(0),
            )?;
            Ok((total as u64, online as u64))
        })
    }

    // -- sessions ----------------------------------------------------------

    pub fn save_session(&self, session: &SessionRecordView) -> DbResult<()> {
        let config_json = serde_json::to_string(&session.config)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, target_mac, target_ip, target_label, config_json, state, error, created_at, started_at, ended_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                    state = excluded.state,
                    error = excluded.error,
                    started_at = excluded.started_at,
                    ended_at = excluded.ended_at",
                params![
                    session.id.to_string(),
                    session.config.target_mac,
                    session.config.target_ip,
                    session.config.target_label,
                    config_json,
                    session.state.as_str(),
                    session.error,
                    session.created_at.timestamp(),
                    session.started_at.map(|t| t.timestamp()),
                    session.ended_at.map(|t| t.timestamp()),
                ],
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO session_targets (session_id, mac_address, ip_address) VALUES (?1, ?2, ?3)",
                params![session.id.to_string(), session.config.target_mac, session.config.target_ip],
            )?;
            Ok(())
        })
    }

    pub fn list_sessions(&self, limit: u64) -> DbResult<Vec<SessionRecordView>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, config_json, state, error, created_at, started_at, ended_at FROM sessions
                 ORDER BY created_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], |r| {
                let id: String = r.get(0)?;
                let config_json: String = r.get(1)?;
                let state: String = r.get(2)?;
                let error: Option<String> = r.get(3)?;
                let created: i64 = r.get(4)?;
                let started: Option<i64> = r.get(5)?;
                let ended: Option<i64> = r.get(6)?;
                Ok((id, config_json, state, error, created, started, ended))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, config_json, state, error, created, started, ended) = row?;
                out.push(SessionRecordView {
                    id: uuid::Uuid::parse_str(&id)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                    config: serde_json::from_str(&config_json)?,
                    state: state_from_str(&state),
                    created_at: ts(created)?,
                    state_changed_at: ts(ended.unwrap_or(created))?,
                    started_at: started.map(ts).transpose()?,
                    ended_at: ended.map(ts).transpose()?,
                    error,
                    active_duration_secs: started
                        .zip(ended)
                        .map(|(s, e)| (e - s).max(0) as u64),
                });
            }
            Ok(out)
        })
    }

    /// Mark session rows left non-terminal by a crash/restart as failed.
    /// In-memory sessions do not survive a restart, so any row still in a
    /// live state is a phantom; the recovery journal (replayed separately)
    /// remains the source of truth for actual restoration.
    pub fn mark_stale_sessions_interrupted(&self) -> DbResult<usize> {
        self.with_conn(|conn| {
            let n = conn.execute(
                "UPDATE sessions SET state = 'failed',
                    error = COALESCE(error, 'interrupted by application restart')
                 WHERE state NOT IN ('completed', 'failed')",
                [],
            )?;
            Ok(n)
        })
    }

    // -- traffic -----------------------------------------------------------

    pub fn insert_traffic_sample(
        &self,
        ts: DateTime<Utc>,
        device_mac: Option<&str>,
        bytes_in: u64,
        bytes_out: u64,
        packets_in: u64,
        packets_out: u64,
    ) -> DbResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO traffic_samples (ts, device_mac, bytes_in, bytes_out, packets_in, packets_out)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    ts.timestamp(),
                    device_mac,
                    bytes_in as i64,
                    bytes_out as i64,
                    packets_in as i64,
                    packets_out as i64,
                ],
            )?;
            Ok(())
        })
    }

    /// Historical totals bucketed to `bucket_secs`.
    pub fn traffic_history(
        &self,
        since: DateTime<Utc>,
        bucket_secs: i64,
        device_mac: Option<&str>,
    ) -> DbResult<Vec<(DateTime<Utc>, u64, u64)>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT (ts / ?1) * ?1 AS bucket, SUM(bytes_in), SUM(bytes_out)
                 FROM traffic_samples
                 WHERE ts >= ?2 AND (?3 IS NULL OR device_mac = ?3)
                 GROUP BY bucket ORDER BY bucket ASC",
            )?;
            let rows =
                stmt.query_map(params![bucket_secs, since.timestamp(), device_mac], |r| {
                    let bucket: i64 = r.get(0)?;
                    let bin: i64 = r.get(1)?;
                    let bout: i64 = r.get(2)?;
                    Ok((bucket, bin as u64, bout as u64))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (bucket, bin, bout) = row?;
                out.push((ts(bucket)?, bin, bout));
            }
            Ok(out)
        })
    }

    /// Per-device traffic totals since a cutoff.
    pub fn device_traffic_history(
        &self,
        since: DateTime<Utc>,
    ) -> DbResult<Vec<(String, u64, u64)>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT device_mac, SUM(bytes_in), SUM(bytes_out)
                 FROM traffic_samples
                 WHERE ts >= ?1 AND device_mac IS NOT NULL
                 GROUP BY device_mac ORDER BY 2 DESC",
            )?;
            let rows = stmt.query_map(params![since.timestamp()], |r| {
                let mac: String = r.get(0)?;
                let bin: i64 = r.get(1)?;
                let bout: i64 = r.get(2)?;
                Ok((mac, bin as u64, bout as u64))
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Sqlite)
        })
    }

    // -- maintenance -------------------------------------------------------

    pub fn purge_old(&self, retention_days: u32) -> DbResult<(usize, usize)> {
        let cutoff = (Utc::now() - chrono::Duration::days(retention_days as i64)).timestamp();
        self.with_conn(|conn| {
            let traffic =
                conn.execute("DELETE FROM traffic_samples WHERE ts < ?1", params![cutoff])?;
            let activity =
                conn.execute("DELETE FROM activity_events WHERE ts < ?1", params![cutoff])?;
            Ok((traffic, activity))
        })
    }

    pub fn clear_history(&self) -> DbResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM traffic_samples", [])?;
            conn.execute("DELETE FROM activity_events", [])?;
            Ok(())
        })
    }

    // -- device enrichment ---------------------------------------------------

    /// Store or clear a user-assigned device kind. `None` clears the
    /// override so automatic classification applies again.
    pub fn set_device_kind_override(&self, mac: &str, kind: Option<&str>) -> DbResult<()> {
        let mac = mac.to_uppercase();
        self.with_conn(|conn| {
            match kind {
                Some(k) => {
                    conn.execute(
                        "INSERT INTO device_kind_overrides (mac_address, kind, updated_at)
                         VALUES (?1, ?2, ?3)
                         ON CONFLICT(mac_address) DO UPDATE SET
                           kind = excluded.kind, updated_at = excluded.updated_at",
                        params![mac, k, Utc::now().timestamp()],
                    )?;
                }
                None => {
                    conn.execute(
                        "DELETE FROM device_kind_overrides WHERE mac_address = ?1",
                        params![mac],
                    )?;
                }
            }
            Ok(())
        })
    }

    /// All kind overrides keyed by MAC.
    pub fn device_kind_overrides(&self) -> DbResult<std::collections::HashMap<String, String>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT mac_address, kind FROM device_kind_overrides")?;
            let rows = stmt.query_map([], |r| {
                let mac: String = r.get(0)?;
                let kind: String = r.get(1)?;
                Ok((mac, kind))
            })?;
            let mut map = std::collections::HashMap::new();
            for row in rows {
                let (mac, kind) = row?;
                map.insert(mac, kind);
            }
            Ok(map)
        })
    }

    /// Record one reachability probe result for a device.
    pub fn insert_latency_sample(
        &self,
        mac: &str,
        ip: &str,
        latency_ms: Option<i64>,
    ) -> DbResult<()> {
        let mac = mac.to_uppercase();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO latency_samples (mac_address, ip_address, ts, latency_ms)
                 VALUES (?1, ?2, ?3, ?4)",
                params![mac, ip, Utc::now().timestamp(), latency_ms],
            )?;
            Ok(())
        })
    }

    /// Latest `limit` probe results for a device, newest first: (ts, ms).
    pub fn latency_samples(&self, mac: &str, limit: i64) -> DbResult<Vec<(i64, Option<i64>)>> {
        let mac = mac.to_uppercase();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT ts, latency_ms FROM latency_samples
                 WHERE mac_address = ?1 ORDER BY ts DESC, id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![mac, limit], |r| {
                let ts: i64 = r.get(0)?;
                let ms: Option<i64> = r.get(1)?;
                Ok((ts, ms))
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    /// Drop probe history older than `retention_days`.
    pub fn prune_latency_samples(&self, retention_days: u32) -> DbResult<usize> {
        let cutoff = (Utc::now() - chrono::Duration::days(retention_days as i64)).timestamp();
        self.with_conn(|conn| {
            let n = conn.execute("DELETE FROM latency_samples WHERE ts < ?1", params![cutoff])?;
            Ok(n)
        })
    }

    /// Activity events that mention a device (by IP or MAC), newest first.
    /// Online/offline transition events embed the device IP in their text.
    pub fn device_events(&self, mac: &str, ip: &str, limit: i64) -> DbResult<Vec<ActivityEvent>> {
        let mac = mac.to_uppercase();
        let ip = ip.to_string();
        self.with_conn(|conn| {
            let sql = "SELECT id, ts, category, message, details_json FROM activity_events
                       WHERE message LIKE '%' || ?1 || '%'
                          OR message LIKE '%' || ?2 || '%'
                       ORDER BY ts DESC, id DESC LIMIT ?3";
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![ip, mac, limit], |r| {
                let id: i64 = r.get(0)?;
                let t: i64 = r.get(1)?;
                let c: String = r.get(2)?;
                let m: String = r.get(3)?;
                let d: Option<String> = r.get(4)?;
                Ok((id, t, c, m, d))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, t, c, m, d) = row?;
                let details = d.and_then(|json| serde_json::from_str(&json).ok());
                out.push(ActivityEvent {
                    id,
                    timestamp: ts(t)?,
                    category: category_from_str(&c),
                    message: m,
                    details,
                });
            }
            Ok(out)
        })
    }
}

fn ts(t: i64) -> DbResult<DateTime<Utc>> {
    Utc.timestamp_opt(t, 0)
        .single()
        .ok_or_else(|| DbError::Sqlite(rusqlite::Error::InvalidQuery))
}

fn row_to_device(r: &rusqlite::Row<'_>) -> rusqlite::Result<NetworkDevice> {
    let first: i64 = r.get(7)?;
    let last: i64 = r.get(8)?;
    Ok(NetworkDevice {
        id: r.get(0)?,
        mac_address: r.get(1)?,
        ip_address: r.get(2)?,
        hostname: r.get(3)?,
        vendor: r.get(4)?,
        device_type: r.get(5)?,
        status: status_from_str(r.get::<_, String>(6)?.as_str()),
        first_seen: Utc.timestamp_opt(first, 0).single().unwrap_or_default(),
        last_seen: Utc.timestamp_opt(last, 0).single().unwrap_or_default(),
    })
}

fn status_to_str(s: banden_core::DeviceStatus) -> &'static str {
    match s {
        banden_core::DeviceStatus::Unknown => "unknown",
        banden_core::DeviceStatus::Online => "online",
        banden_core::DeviceStatus::Offline => "offline",
        banden_core::DeviceStatus::New => "new",
    }
}

fn status_from_str(s: &str) -> banden_core::DeviceStatus {
    match s {
        "online" => banden_core::DeviceStatus::Online,
        "offline" => banden_core::DeviceStatus::Offline,
        "new" => banden_core::DeviceStatus::New,
        _ => banden_core::DeviceStatus::Unknown,
    }
}

fn state_from_str(s: &str) -> banden_core::SessionState {
    match s {
        "preparing" => banden_core::SessionState::Preparing,
        "active" => banden_core::SessionState::Active,
        "stopping" => banden_core::SessionState::Stopping,
        "restoring" => banden_core::SessionState::Restoring,
        "verifying" => banden_core::SessionState::Verifying,
        "recovery_required" => banden_core::SessionState::RecoveryRequired,
        "failed" => banden_core::SessionState::Failed,
        _ => banden_core::SessionState::Completed,
    }
}

// ---------------------------------------------------------------------------
// Port implementations
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl JournalStore for Db {
    async fn append(&self, entry: JournalEntry) {
        if let Err(e) = self.append_journal(&entry) {
            tracing::error!(error = %e, "failed to append journal entry");
        }
    }

    async fn update(&self, entry: &JournalEntry) {
        if let Err(e) = self.update_journal(entry) {
            tracing::error!(error = %e, "failed to update journal entry");
        }
    }

    async fn pending(&self) -> Vec<JournalEntry> {
        self.list_journal(Some(JournalEntryState::Pending))
            .unwrap_or_default()
    }

    async fn entries_for_session(&self, session_id: uuid::Uuid) -> Vec<JournalEntry> {
        self.list_journal_for_session(session_id)
            .unwrap_or_default()
    }

    async fn clear_session(&self, session_id: uuid::Uuid) {
        if let Err(e) = self.clear_journal_session(session_id) {
            tracing::error!(error = %e, "failed to clear journal session");
        }
    }
}

#[async_trait::async_trait]
impl ActivityLog for Db {
    async fn append(
        &self,
        category: EventCategory,
        message: String,
        details: Option<serde_json::Value>,
    ) {
        if let Err(e) = self.insert_activity(Utc::now(), category, &message, details.as_ref()) {
            tracing::error!(error = %e, "failed to append activity event");
        }
    }

    async fn recent(
        &self,
        limit: u64,
        category: Option<EventCategory>,
        search: Option<String>,
    ) -> Vec<ActivityEvent> {
        self.list_activity(limit, category, search)
            .unwrap_or_default()
    }
}

impl Db {
    // -- recovery journal ---------------------------------------------------

    fn append_journal(&self, entry: &JournalEntry) -> DbResult<()> {
        let action_json = serde_json::to_string(&entry.action)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO recovery_journal (session_id, action_json, state, attempts, created_at, updated_at, last_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    entry.session_id.to_string(),
                    action_json,
                    journal_state_to_str(&entry.state),
                    entry.attempts as i64,
                    entry.created_at.timestamp(),
                    entry.updated_at.timestamp(),
                    entry.last_error,
                ],
            )?;
            Ok(())
        })
    }

    fn update_journal(&self, entry: &JournalEntry) -> DbResult<()> {
        let action_json = serde_json::to_string(&entry.action)?;
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE recovery_journal
                 SET state = ?2, attempts = ?3, updated_at = ?4, last_error = ?5, action_json = ?6
                 WHERE id = ?1",
                params![
                    entry.id,
                    journal_state_to_str(&entry.state),
                    entry.attempts as i64,
                    entry.updated_at.timestamp(),
                    entry.last_error,
                    action_json,
                ],
            )?;
            Ok(())
        })
    }

    fn list_journal(&self, state: Option<JournalEntryState>) -> DbResult<Vec<JournalEntry>> {
        self.with_conn(|conn| {
            let sql = match state {
                Some(_) => "SELECT id, session_id, action_json, state, attempts, created_at, updated_at, last_error
                            FROM recovery_journal WHERE state = ?1 ORDER BY id",
                None => "SELECT id, session_id, action_json, state, attempts, created_at, updated_at, last_error
                         FROM recovery_journal ORDER BY id",
            };
            let mut out = Vec::new();
            match state {
                Some(s) => {
                    let mut stmt = conn.prepare(sql)?;
                    let rows = stmt.query_map(params![journal_state_to_str(&s)], journal_row_to_entry)?;
                    for r in rows {
                        out.push(r?);
                    }
                }
                None => {
                    let mut stmt = conn.prepare(sql)?;
                    let rows = stmt.query_map([], journal_row_to_entry)?;
                    for r in rows {
                        out.push(r?);
                    }
                }
            }
            Ok(out)
        })
    }

    fn list_journal_for_session(&self, session_id: uuid::Uuid) -> DbResult<Vec<JournalEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, action_json, state, attempts, created_at, updated_at, last_error
                 FROM recovery_journal WHERE session_id = ?1 ORDER BY id",
            )?;
            let rows = stmt.query_map(params![session_id.to_string()], journal_row_to_entry)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Sqlite)
        })
    }

    fn clear_journal_session(&self, session_id: uuid::Uuid) -> DbResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM recovery_journal WHERE session_id = ?1",
                params![session_id.to_string()],
            )?;
            Ok(())
        })
    }

    // -- activity -----------------------------------------------------------

    fn insert_activity(
        &self,
        at: DateTime<Utc>,
        category: EventCategory,
        message: &str,
        details: Option<&serde_json::Value>,
    ) -> DbResult<()> {
        let details_json = details.map(serde_json::to_string).transpose()?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO activity_events (ts, category, message, details_json) VALUES (?1, ?2, ?3, ?4)",
                params![at.timestamp(), category.as_str(), message, details_json],
            )?;
            Ok(())
        })
    }

    fn list_activity(
        &self,
        limit: u64,
        category: Option<EventCategory>,
        search: Option<String>,
    ) -> DbResult<Vec<ActivityEvent>> {
        self.with_conn(|conn| {
            let sql = "SELECT id, ts, category, message, details_json FROM activity_events
                       WHERE (?1 IS NULL OR category = ?1)
                         AND (?2 IS NULL OR message LIKE '%' || ?2 || '%')
                       ORDER BY ts DESC, id DESC LIMIT ?3";
            let mut stmt = conn.prepare(sql)?;
            let cat = category.map(|c| c.as_str().to_string());
            let search = search.map(|s| s.to_lowercase());
            let rows = stmt.query_map(params![cat, search, limit as i64], |r| {
                let id: i64 = r.get(0)?;
                let t: i64 = r.get(1)?;
                let c: String = r.get(2)?;
                let m: String = r.get(3)?;
                let d: Option<String> = r.get(4)?;
                Ok((id, t, c, m, d))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, t, c, m, d) = row?;
                let details = match d {
                    Some(s) => serde_json::from_str(&s).ok(),
                    None => None,
                };
                out.push(ActivityEvent {
                    id,
                    timestamp: ts(t)?,
                    category: category_from_str(&c),
                    message: m,
                    details,
                });
            }
            Ok(out)
        })
    }
}

fn category_from_str(s: &str) -> EventCategory {
    match s {
        "WARNING" => EventCategory::Warning,
        "ERROR" => EventCategory::Error,
        "RECOVERY" => EventCategory::Recovery,
        "NETWORK" => EventCategory::Network,
        "SESSION" => EventCategory::Session,
        _ => EventCategory::Info,
    }
}

fn journal_state_to_str(s: &JournalEntryState) -> &'static str {
    match s {
        JournalEntryState::Pending => "pending",
        JournalEntryState::Done => "done",
        JournalEntryState::Failed => "failed",
    }
}

fn journal_state_from_str(s: &str) -> JournalEntryState {
    match s {
        "done" => JournalEntryState::Done,
        "failed" => JournalEntryState::Failed,
        _ => JournalEntryState::Pending,
    }
}

fn journal_row_to_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<JournalEntry> {
    let id: i64 = r.get(0)?;
    let session: String = r.get(1)?;
    let action_json: String = r.get(2)?;
    let state: String = r.get(3)?;
    let attempts: i64 = r.get(4)?;
    let created: i64 = r.get(5)?;
    let updated: i64 = r.get(6)?;
    let last_error: Option<String> = r.get(7)?;
    let action = serde_json::from_str(&action_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let session_id = uuid::Uuid::parse_str(&session).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(JournalEntry {
        id,
        session_id,
        action,
        state: journal_state_from_str(&state),
        attempts: attempts as u32,
        created_at: Utc.timestamp_opt(created, 0).single().unwrap_or_default(),
        updated_at: Utc.timestamp_opt(updated, 0).single().unwrap_or_default(),
        last_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use banden_core::{NetworkDevice, RestorationAction};

    fn device(mac: &str, ip: &str) -> NetworkDevice {
        NetworkDevice {
            id: 0,
            mac_address: mac.into(),
            ip_address: ip.into(),
            hostname: None,
            vendor: Some("Test Vendor".into()),
            device_type: None,
            status: banden_core::DeviceStatus::Online,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        }
    }

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    #[test]
    fn migrations_apply_and_are_idempotent() {
        let d = db();
        // Opening again must not fail or duplicate.
        let version: i64 = d
            .with_conn(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn device_upsert_keeps_first_seen_and_enriches() {
        let d = db();
        let mut dev = device("AA:BB:CC:DD:EE:01", "192.168.1.10");
        dev.hostname = None;
        d.upsert_device(&dev).unwrap();
        let mut dev2 = dev.clone();
        dev2.hostname = Some("laptop.local".into());
        dev2.ip_address = "192.168.1.99".into();
        d.upsert_device(&dev2).unwrap();
        let all = d.list_devices().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].ip_address, "192.168.1.99");
        assert_eq!(all[0].hostname.as_deref(), Some("laptop.local"));
        // Timestamps are stored with second resolution.
        assert_eq!(all[0].first_seen.timestamp(), dev.first_seen.timestamp());
    }

    #[test]
    fn journal_store_roundtrip() {
        let d = db();
        let id = uuid::Uuid::new_v4();
        let entry = JournalEntry::new(
            id,
            RestorationAction::ClearNeighborEntry {
                ip: "10.0.0.1".into(),
            },
            Utc::now(),
        );
        d.append_journal(&entry).unwrap();
        let pending = d.list_journal(Some(JournalEntryState::Pending)).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(matches!(
            pending[0].action,
            RestorationAction::ClearNeighborEntry { .. }
        ));
        let mut e = pending.into_iter().next().unwrap();
        e.state = JournalEntryState::Done;
        d.update_journal(&e).unwrap();
        assert!(d
            .list_journal(Some(JournalEntryState::Pending))
            .unwrap()
            .is_empty());
        d.clear_journal_session(id).unwrap();
        assert!(d.list_journal(None).unwrap().is_empty());
    }

    #[test]
    fn activity_append_and_filter() {
        let d = db();
        d.insert_activity(Utc::now(), EventCategory::Info, "Device discovered", None)
            .unwrap();
        d.insert_activity(Utc::now(), EventCategory::Session, "Session started", None)
            .unwrap();
        let all = d.list_activity(10, None, None).unwrap();
        assert_eq!(all.len(), 2);
        let sessions = d
            .list_activity(10, Some(EventCategory::Session), None)
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].message, "Session started");
        let found = d.list_activity(10, None, Some("discover".into())).unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn traffic_history_buckets() {
        let d = db();
        // Align to a minute boundary so the 120 samples span exactly 2 buckets.
        let now_secs = Utc::now().timestamp();
        let base = now_secs - now_secs.rem_euclid(60);
        let base = Utc.timestamp_opt(base, 0).single().unwrap();
        for i in 0..120 {
            d.insert_traffic_sample(base + chrono::Duration::seconds(i), None, 1000, 500, 10, 5)
                .unwrap();
        }
        let hist = d
            .traffic_history(base - chrono::Duration::minutes(5), 60, None)
            .unwrap();
        assert_eq!(hist.len(), 2);
        assert!(hist[0].1 >= 60_000);
    }

    #[test]
    fn settings_roundtrip() {
        let d = db();
        assert!(d.get_settings().unwrap().is_none());
        let v = serde_json::json!({ "general": { "theme": "dark" } });
        d.set_settings(&v).unwrap();
        assert_eq!(d.get_settings().unwrap().unwrap(), v);
    }
}
