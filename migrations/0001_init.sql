-- BanDen initial schema.
-- Every schema change must ship as a new migration; manual edits are
-- forbidden. user_version tracks the applied migration count.

CREATE TABLE IF NOT EXISTS networks (
    id          INTEGER PRIMARY KEY,
    iface_id    TEXT NOT NULL,
    name        TEXT NOT NULL,
    cidr        TEXT,
    gateway     TEXT,
    first_seen  INTEGER NOT NULL,
    last_seen   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS devices (
    id           INTEGER PRIMARY KEY,
    mac_address  TEXT NOT NULL UNIQUE,
    ip_address   TEXT NOT NULL,
    hostname     TEXT,
    vendor       TEXT,
    device_type  TEXT,
    status       TEXT NOT NULL DEFAULT 'unknown',
    first_seen   INTEGER NOT NULL,
    last_seen    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_devices_last_seen ON devices(last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_devices_ip ON devices(ip_address);

CREATE TABLE IF NOT EXISTS sessions (
    id               TEXT PRIMARY KEY,
    target_mac       TEXT NOT NULL,
    target_ip        TEXT NOT NULL,
    target_label     TEXT,
    config_json      TEXT NOT NULL,
    state            TEXT NOT NULL,
    error            TEXT,
    created_at       INTEGER NOT NULL,
    started_at       INTEGER,
    ended_at         INTEGER
);

CREATE INDEX IF NOT EXISTS idx_sessions_created ON sessions(created_at DESC);

CREATE TABLE IF NOT EXISTS session_targets (
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    mac_address TEXT NOT NULL,
    ip_address  TEXT NOT NULL,
    PRIMARY KEY (session_id, mac_address)
);

CREATE TABLE IF NOT EXISTS traffic_samples (
    ts          INTEGER NOT NULL,
    device_mac  TEXT,
    bytes_in    INTEGER NOT NULL,
    bytes_out   INTEGER NOT NULL,
    packets_in  INTEGER NOT NULL,
    packets_out INTEGER NOT NULL,
    PRIMARY KEY (ts, device_mac)
);

CREATE INDEX IF NOT EXISTS idx_traffic_ts ON traffic_samples(ts DESC);

CREATE TABLE IF NOT EXISTS activity_events (
    id           INTEGER PRIMARY KEY,
    ts           INTEGER NOT NULL,
    category     TEXT NOT NULL,
    message      TEXT NOT NULL,
    details_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_activity_ts ON activity_events(ts DESC);
CREATE INDEX IF NOT EXISTS idx_activity_category ON activity_events(category);

CREATE TABLE IF NOT EXISTS settings (
    key        TEXT PRIMARY KEY,
    value_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS recovery_journal (
    id           INTEGER PRIMARY KEY,
    session_id   TEXT NOT NULL,
    action_json  TEXT NOT NULL,
    state        TEXT NOT NULL,
    attempts     INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    last_error   TEXT
);

CREATE INDEX IF NOT EXISTS idx_journal_session ON recovery_journal(session_id);
CREATE INDEX IF NOT EXISTS idx_journal_state ON recovery_journal(state);

PRAGMA user_version = 1;
