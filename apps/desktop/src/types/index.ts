/**
 * TypeScript mirrors of the Rust DTOs (banden-core `models.rs`).
 * Field names are snake_case to match the serde payloads exactly.
 */

export type ConnectionStatus = "disconnected" | "connected" | "degraded";
export type NetworkStateKind = "normal" | "modified" | "restoring" | "unknown";
export type DeviceStatus = "unknown" | "online" | "offline" | "new";
export type SessionState =
  | "preparing"
  | "active"
  | "stopping"
  | "restoring"
  | "verifying"
  | "completed"
  | "recovery_required"
  | "failed";
export type EventCategory = "INFO" | "WARNING" | "ERROR" | "RECOVERY" | "NETWORK" | "SESSION";
export type ThemePreference = "system" | "light" | "dark";

export interface InterfaceInfo {
  id: string;
  if_index: number | null;
  name: string;
  friendly_name: string | null;
  mac_address: string | null;
  ipv4: string | null;
  prefix_len: number | null;
  cidr: string | null;
  gateway: string | null;
  is_up: boolean;
  is_loopback: boolean;
  is_physical: boolean;
}

export interface NetworkSummary {
  interface_name: string | null;
  interface_id: string | null;
  ipv4: string | null;
  cidr: string | null;
  gateway: string | null;
  connection_status: ConnectionStatus;
}

export interface NetworkDevice {
  id: number;
  mac_address: string;
  ip_address: string;
  hostname: string | null;
  vendor: string | null;
  device_type: string | null;
  status: DeviceStatus;
  first_seen: string;
  last_seen: string;
}

/** Device row enriched with the effective kind (override or auto). */
export interface DeviceView extends NetworkDevice {
  kind: string | null;
  kind_source: "manual" | "auto" | "unknown";
}

export interface PingResult {
  latency_ms: number | null;
}

export interface LatencyPoint {
  /** Unix epoch seconds. */
  ts: number;
  latency_ms: number | null;
}

export interface ConnectivityReport {
  samples: LatencyPoint[];
  sample_count: number;
  ok_count: number;
  availability_pct: number;
  current_latency_ms: number | null;
}

export interface DeviceTraffic {
  mac_address: string | null;
  ip_address: string | null;
  label: string;
  bytes_in: number;
  bytes_out: number;
  packets_in: number;
  packets_out: number;
  download_rate_bps: number;
  upload_rate_bps: number;
}

export interface TrafficSample {
  timestamp: string;
  bytes_in: number;
  bytes_out: number;
  packets_in: number;
  packets_out: number;
  download_rate_bps: number;
  upload_rate_bps: number;
}

export interface ProtocolStat {
  protocol: string;
  bytes: number;
  packets: number;
}

export interface TrafficSnapshot {
  timestamp: string;
  total: DeviceTraffic;
  history: TrafficSample[];
  top_devices: DeviceTraffic[];
  protocols: ProtocolStat[];
}

export interface SessionConfig {
  target_mac: string;
  target_ip: string;
  target_label: string | null;
  download_limit_bps: number | null;
  upload_limit_bps: number | null;
  duration_secs: number | null;
  /** App IDs to block (mirrors the backend catalog). Empty = whole device. */
  blocked_apps: string[];
  /** Allowlist mode: ONLY these apps have internet; everything else is cut. */
  allowed_apps?: string[];
  /** Shaping priority: low | normal | high | max. Scales burst allowance. */
  priority?: string | null;
}

/** Mirrors the backend app catalog (crates/banden-net/apps.json). */
export const APP_CATALOG = [
  { id: "whatsapp", name: "WhatsApp", emoji: "💬" },
  { id: "facebook", name: "Facebook", emoji: "📘" },
  { id: "instagram", name: "Instagram", emoji: "📸" },
  { id: "youtube", name: "YouTube", emoji: "▶️" },
  { id: "tiktok", name: "TikTok", emoji: "🎵" },
  { id: "netflix", name: "Netflix", emoji: "🎬" },
  { id: "spotify", name: "Spotify", emoji: "🎧" },
  { id: "snapchat", name: "Snapchat", emoji: "👻" },
  { id: "telegram", name: "Telegram", emoji: "✈️" },
  { id: "x", name: "X (Twitter)", emoji: "🐦" },
  { id: "discord", name: "Discord", emoji: "🎮" },
  { id: "roblox", name: "Roblox", emoji: "🧱" },
] as const;

export interface SessionRecordView {
  id: string;
  config: SessionConfig;
  state: SessionState;
  created_at: string;
  state_changed_at: string;
  started_at: string | null;
  ended_at: string | null;
  error: string | null;
  active_duration_secs: number | null;
}

export interface ActivityEvent {
  id: number;
  timestamp: string;
  category: EventCategory;
  message: string;
  details: unknown | null;
}

export interface SystemWarning {
  code: string;
  message: string;
  timestamp: string;
}

export interface SystemStatus {
  network: NetworkSummary;
  network_state: NetworkStateKind;
  device_count: number;
  online_devices: number;
  active_sessions: number;
  capture_source: string;
  control_backend: string;
  watchdog_running: boolean;
  latency_ms: number | null;
  download_rate_bps: number;
  upload_rate_bps: number;
  warnings: SystemWarning[];
}

export interface GeneralSettings {
  start_with_windows: boolean;
  minimize_to_tray: boolean;
  theme: ThemePreference;
  notifications: boolean;
}

export interface NetworkSettings {
  default_interface: string | null;
  discovery_interval_secs: number;
  monitoring_enabled: boolean;
}

export interface SafetySettings {
  automatic_cleanup: boolean;
  recovery_watchdog: boolean;
  authorization_confirmation: boolean;
  restoration_verification: boolean;
}

export interface DataSettings {
  retention_days: number;
}

export interface AppSettings {
  general: GeneralSettings;
  network: NetworkSettings;
  safety: SafetySettings;
  data: DataSettings;
}

export interface EmergencyStopOutcome {
  stage: "completed" | "recovery_required";
  sessions_affected: string[];
  failures: string[];
  network_state: NetworkStateKind;
}

export interface TrafficHistoryPoint {
  ts: string;
  bytes_in: number;
  bytes_out: number;
}

export interface DeviceDiscoveryResult {
  discovered: number;
  devices: NetworkDevice[];
}

export interface DeviceTrafficReport {
  live: DeviceTraffic | null;
  bytes_last_24h: number;
}

/** Typed error payload returned by failed commands. */
export interface CommandError {
  code: string;
  message: string;
}

// ---------------------------------------------------------------------------
// Event payloads (versioned on the Rust side with a `v` field)
// ---------------------------------------------------------------------------

export interface DeviceDiscoveredEvent {
  v: number;
  device: NetworkDevice;
}

export interface DeviceUpdatedEvent {
  v: number;
  device?: NetworkDevice;
  mac?: string;
  status?: DeviceStatus;
}

export interface SessionStateChangedEvent {
  v: number;
  session_id: string;
  from: SessionState;
  to: SessionState;
  reason: string | null;
}

export interface NetworkStateChangedEvent {
  v: number;
  from?: NetworkStateKind;
  to?: NetworkStateKind;
  reason?: string;
  interface?: string;
}

export interface RecoveryProgressEvent {
  v: number;
  stage: string;
  detail: string | null;
  session_id?: string;
}

export interface SystemWarningEvent {
  v: number;
  warning: SystemWarning;
}

export interface EmergencyStopResultEvent {
  v: number;
  outcome?: EmergencyStopOutcome;
  error?: string;
}
