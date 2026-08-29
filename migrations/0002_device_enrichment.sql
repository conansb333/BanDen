-- Device enrichment: user-assigned kind overrides + per-device latency probes.

-- User-picked device kind ("phone", "laptop", ...). Absent row = automatic
-- classification from hostname/OUI heuristics.
CREATE TABLE IF NOT EXISTS device_kind_overrides (
    mac_address TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- Reachability probe history per device (manual "Ping now" + periodic
-- background sampling). latency_ms NULL = no answer within the timeout.
CREATE TABLE IF NOT EXISTS latency_samples (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    mac_address TEXT NOT NULL,
    ip_address  TEXT NOT NULL,
    ts          INTEGER NOT NULL,
    latency_ms  INTEGER
);

CREATE INDEX IF NOT EXISTS idx_latency_samples_mac_ts
    ON latency_samples(mac_address, ts DESC);
