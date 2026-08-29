/**
 * Typed wrappers around Tauri commands. All backend access goes through
 * this module — pages and components never call `invoke` directly.
 */
import { invoke } from "@tauri-apps/api/core";
import type {
  ActivityEvent,
  AppSettings,
  ConnectivityReport,
  DeviceDiscoveryResult,
  DeviceTrafficReport,
  DeviceView,
  EmergencyStopOutcome,
  InterfaceInfo,
  NetworkSummary,
  PingResult,
  SessionConfig,
  SessionRecordView,
  SystemStatus,
  TrafficHistoryPoint,
  TrafficSnapshot,
} from "@/types";

export const api = {
  // Network
  getNetworkInterfaces: () => invoke<InterfaceInfo[]>("get_network_interfaces"),
  getNetworkInfo: () => invoke<NetworkSummary>("get_network_info"),

  // Devices
  discoverDevices: () => invoke<DeviceDiscoveryResult>("discover_devices"),
  getDevices: () => invoke<DeviceView[]>("get_devices"),
  getDevice: (deviceId: number) => invoke<DeviceView | null>("get_device", { deviceId }),
  setDeviceKind: (mac: string, ip: string, kind: string | null) =>
    invoke<void>("set_device_kind", { mac, ip, kind }),
  pingDevice: (mac: string, ip: string) =>
    invoke<PingResult>("ping_device", { mac, ip }),
  getDeviceConnectivity: (mac: string) =>
    invoke<ConnectivityReport>("get_device_connectivity", { mac }),
  getDeviceEvents: (mac: string, ip: string, limit?: number) =>
    invoke<ActivityEvent[]>("get_device_events", { mac, ip, limit }),

  // Traffic
  getTrafficStats: () => invoke<TrafficSnapshot>("get_traffic_stats"),
  getTrafficHistory: (windowSecs?: number, bucketSecs?: number) =>
    invoke<TrafficHistoryPoint[]>("get_traffic_history", { windowSecs, bucketSecs }),
  getDeviceTraffic: (mac: string) => invoke<DeviceTrafficReport>("get_device_traffic", { mac }),

  // Sessions
  getSessions: () => invoke<SessionRecordView[]>("get_sessions"),
  getSession: (sessionId: string) => invoke<SessionRecordView>("get_session", { sessionId }),
  startSession: (config: SessionConfig) => invoke<SessionRecordView>("start_session", { config }),
  stopSession: (sessionId: string, reason?: "user_requested" | "duration_elapsed") =>
    invoke<SessionRecordView>("stop_session", { sessionId, reason }),

  // Emergency stop
  emergencyStop: () => invoke<EmergencyStopOutcome>("emergency_stop"),

  // Activity & system
  getActivity: (opts?: { limit?: number; category?: string; search?: string }) =>
    invoke<ActivityEvent[]>("get_activity", {
      limit: opts?.limit,
      category: opts?.category,
      search: opts?.search,
    }),
  getSystemStatus: () => invoke<SystemStatus>("get_system_status"),

  // Monitoring
  startMonitoring: () => invoke<void>("start_monitoring"),
  stopMonitoring: () => invoke<void>("stop_monitoring"),

  // Settings
  getSettings: () => invoke<AppSettings>("get_settings"),
  updateSettings: (settings: AppSettings) => invoke<AppSettings>("update_settings", { settings }),

  // Data
  clearHistory: () => invoke<void>("clear_history"),
  purgeOldData: () => invoke<[number, number]>("purge_old_data"),
  getDataDir: () => invoke<string>("get_data_dir"),
};
