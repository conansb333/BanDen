//! Domain models shared between the backend and the API surface.
//!
//! All structs here serialize in `snake_case` and are mirrored by the
//! TypeScript types in the frontend (`apps/desktop/src/types`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Interfaces & network
// ---------------------------------------------------------------------------

/// A host network adapter as seen by the discovery subsystem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct InterfaceInfo {
    /// Stable identifier (adapter GUID / alias).
    pub id: String,
    /// Numeric interface index used by IP Helper counter APIs.
    #[serde(default)]
    pub if_index: Option<u32>,
    pub name: String,
    pub friendly_name: Option<String>,
    pub mac_address: Option<String>,
    pub ipv4: Option<String>,
    pub prefix_len: Option<u8>,
    /// CIDR of the local subnet, e.g. `192.168.1.0/24`.
    pub cidr: Option<String>,
    pub gateway: Option<String>,
    pub is_up: bool,
    pub is_loopback: bool,
    pub is_physical: bool,
}

/// Aggregated view of the current network context.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct NetworkSummary {
    pub interface_name: Option<String>,
    pub interface_id: Option<String>,
    pub ipv4: Option<String>,
    pub cidr: Option<String>,
    pub gateway: Option<String>,
    pub connection_status: ConnectionStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connected,
    Degraded,
}

/// Global network-state classification tracked by the recovery manager.
/// `Unknown` is a real state: when we cannot prove the network is restored,
/// we must report that instead of assuming success.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkStateKind {
    #[default]
    Normal,
    Modified,
    Restoring,
    Unknown,
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct NetworkDevice {
    pub id: i64,
    pub mac_address: String,
    pub ip_address: String,
    pub hostname: Option<String>,
    pub vendor: Option<String>,
    pub device_type: Option<String>,
    pub status: DeviceStatus,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

impl NetworkDevice {
    /// Display label used across the UI.
    pub fn label(&self) -> String {
        self.hostname
            .clone()
            .unwrap_or_else(|| self.ip_address.clone())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    #[default]
    Unknown,
    Online,
    Offline,
    New,
}

// ---------------------------------------------------------------------------
// Traffic
// ---------------------------------------------------------------------------

/// Raw monotonic counters at a point in time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct TrafficCounters {
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub packets_in: u64,
    pub packets_out: u64,
    pub timestamp: DateTime<Utc>,
}

impl TrafficCounters {
    pub fn delta(&self, earlier: &TrafficCounters) -> TrafficCounters {
        let safe = |now: u64, before: u64| now.saturating_sub(before);
        TrafficCounters {
            bytes_in: safe(self.bytes_in, earlier.bytes_in),
            bytes_out: safe(self.bytes_out, earlier.bytes_out),
            packets_in: safe(self.packets_in, earlier.packets_in),
            packets_out: safe(self.packets_out, earlier.packets_out),
            timestamp: self.timestamp,
        }
    }
}

/// A computed traffic sample with rates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TrafficSample {
    pub timestamp: DateTime<Utc>,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub packets_in: u64,
    pub packets_out: u64,
    pub download_rate_bps: f64,
    pub upload_rate_bps: f64,
}

/// Per-device traffic view.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DeviceTraffic {
    pub mac_address: Option<String>,
    pub ip_address: Option<String>,
    pub label: String,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub packets_in: u64,
    pub packets_out: u64,
    pub download_rate_bps: f64,
    pub upload_rate_bps: f64,
}

/// Snapshot of aggregated traffic, emitted to the UI at a bounded rate.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct TrafficSnapshot {
    pub timestamp: DateTime<Utc>,
    pub total: DeviceTraffic,
    /// Most recent samples (bounded window) for the realtime chart.
    pub history: Vec<TrafficSample>,
    /// Top devices by combined throughput.
    pub top_devices: Vec<DeviceTraffic>,
    /// Aggregate protocol distribution (share of observed bytes).
    pub protocols: Vec<ProtocolStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct ProtocolStat {
    pub protocol: String,
    pub bytes: u64,
    pub packets: u64,
}

/// A connection/flow record, when a flow-capable capture source is active.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FlowInfo {
    pub source_ip: String,
    pub source_port: u16,
    pub dest_ip: String,
    pub dest_port: u16,
    pub protocol: String,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// User-supplied parameters for a traffic-control session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct SessionConfig {
    pub target_mac: String,
    pub target_ip: String,
    pub target_label: Option<String>,
    /// Download limit in bits per second (None = unlimited).
    pub download_limit_bps: Option<u64>,
    /// Upload limit in bits per second (None = unlimited).
    pub upload_limit_bps: Option<u64>,
    /// Optional maximum duration; the session self-stops afterwards.
    pub duration_secs: Option<u64>,
    /// App IDs to block (catalog references, e.g. "whatsapp"). Empty =
    /// the whole device is controlled; non-empty = only these apps'
    /// traffic is dropped while everything else flows normally.
    #[serde(default)]
    pub blocked_apps: Vec<String>,
    /// App IDs to ALLOW in allowlist mode: only these apps' traffic passes
    /// and everything else on the device is cut. Non-empty = allowlist
    /// mode (blocked_apps must be empty). Default-deny, so it also kills
    /// apps that dodge DNS/SNI blocking via hardcoded IPs or encrypted DNS.
    #[serde(default)]
    pub allowed_apps: Vec<String>,
    /// Optional shaping priority ("low"|"normal"|"high"|"max"). Scales the
    /// token-bucket burst allowance; None = normal.
    #[serde(default)]
    pub priority: Option<String>,
}

impl SessionConfig {
    pub fn validate(&self) -> Result<(), super::CoreError> {
        if !is_mac(&self.target_mac) {
            return Err(super::CoreError::InvalidConfig(
                "target MAC address is malformed".into(),
            ));
        }
        if self.target_ip.parse::<std::net::IpAddr>().is_err() {
            return Err(super::CoreError::InvalidConfig(
                "target IP address is malformed".into(),
            ));
        }
        // NOTE: a limit of 0 is legal and means "hard blackhole" - the
        // shaper's token bucket drops everything at rate 0. That is how the
        // UI's rate sliders express "Blocked".
        Ok(())
    }
}

fn is_mac(s: &str) -> bool {
    let bytes = s.split(':').collect::<Vec<_>>();
    bytes.len() == 6
        && bytes
            .iter()
            .all(|b| b.len() == 2 && u8::from_str_radix(b, 16).is_ok())
}

/// Read-only view of a session as exposed through the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionRecordView {
    pub id: uuid::Uuid,
    pub config: SessionConfig,
    pub state: super::session::machine::SessionState,
    pub created_at: DateTime<Utc>,
    pub state_changed_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// Elapsed active seconds, when the session is/was active.
    pub active_duration_secs: Option<u64>,
}

// ---------------------------------------------------------------------------
// Activity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ActivityEvent {
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    pub category: EventCategory,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    Info,
    Warning,
    Error,
    Recovery,
    Network,
    Session,
}

impl EventCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventCategory::Info => "INFO",
            EventCategory::Warning => "WARNING",
            EventCategory::Error => "ERROR",
            EventCategory::Recovery => "RECOVERY",
            EventCategory::Network => "NETWORK",
            EventCategory::Session => "SESSION",
        }
    }
}

// ---------------------------------------------------------------------------
// System status & settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct SystemStatus {
    pub network: NetworkSummary,
    pub network_state: NetworkStateKind,
    pub device_count: u64,
    pub online_devices: u64,
    pub active_sessions: u64,
    pub capture_source: String,
    /// Name of the active control backend (`simulation` or `arp-cut`).
    pub control_backend: String,
    pub watchdog_running: bool,
    pub latency_ms: Option<u64>,
    pub download_rate_bps: f64,
    pub upload_rate_bps: f64,
    pub warnings: Vec<SystemWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct SystemWarning {
    pub code: String,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub struct AppSettings {
    #[serde(default)]
    pub general: GeneralSettings,
    #[serde(default)]
    pub network: NetworkSettings,
    #[serde(default)]
    pub safety: SafetySettings,
    #[serde(default)]
    pub data: DataSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GeneralSettings {
    pub start_with_windows: bool,
    pub minimize_to_tray: bool,
    pub theme: ThemePreference,
    pub notifications: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            minimize_to_tray: true,
            theme: ThemePreference::System,
            notifications: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct NetworkSettings {
    /// Preferred interface id; `None` = automatic selection.
    pub default_interface: Option<String>,
    pub discovery_interval_secs: u64,
    pub monitoring_enabled: bool,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            default_interface: None,
            discovery_interval_secs: 60,
            monitoring_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct SafetySettings {
    /// Restore network state automatically on normal shutdown paths.
    pub automatic_cleanup: bool,
    /// Run the independent watchdog process.
    pub recovery_watchdog: bool,
    /// Require an explicit authorization confirmation before any session.
    pub authorization_confirmation: bool,
    /// Verify restoration after every stop, not only emergencies.
    pub restoration_verification: bool,
}

impl Default for SafetySettings {
    fn default() -> Self {
        Self {
            automatic_cleanup: true,
            recovery_watchdog: true,
            authorization_confirmation: true,
            restoration_verification: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct DataSettings {
    pub retention_days: u32,
}

impl Default for DataSettings {
    fn default() -> Self {
        Self { retention_days: 30 }
    }
}
