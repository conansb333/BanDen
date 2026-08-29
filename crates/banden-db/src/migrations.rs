//! Embedded schema migrations.
//!
//! Migration SQL lives in the repository-root `migrations/` directory and is
//! embedded at compile time. The applied state is tracked with SQLite's
//! `user_version` pragma. The schema is never modified outside a migration.

use rusqlite::Connection;
use std::path::Path;

/// (name, sql) in application order. `include_str!` paths are relative to
/// this file: crates/banden-db/src -> ../../.. = repository root.
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_init",
        include_str!("../../../migrations/0001_init.sql"),
    ),
    (
        "0002_device_enrichment",
        include_str!("../../../migrations/0002_device_enrichment.sql"),
    ),
];

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, (name, sql)) in MIGRATIONS.iter().enumerate() {
        let target = (i + 1) as i64;
        if target > current {
            tracing::info!(migration = name, version = target, "applying migration");
            conn.execute_batch(sql)?;
        }
    }
    Ok(())
}

/// Validates migration ordering invariants without a live database; used by
/// tests to keep the list honest.
pub fn migration_count() -> usize {
    MIGRATIONS.len()
}

/// Open + migrate helper used by the app and the watchdog.
pub fn open_and_migrate(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    migrate(&conn)?;
    Ok(conn)
}
