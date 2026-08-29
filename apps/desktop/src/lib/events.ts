/**
 * Typed Tauri event subscriptions. Event names match the Rust constants
 * in banden-core `events.rs` exactly.
 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  DeviceDiscoveredEvent,
  DeviceUpdatedEvent,
  EmergencyStopResultEvent,
  NetworkStateChangedEvent,
  RecoveryProgressEvent,
  SessionStateChangedEvent,
  SystemWarningEvent,
  TrafficSnapshot,
} from "@/types";

export const Events = {
  DeviceDiscovered: "device_discovered",
  DeviceUpdated: "device_updated",
  DeviceRemoved: "device_removed",
  TrafficUpdate: "traffic_update",
  SessionCreated: "session_created",
  SessionStateChanged: "session_state_changed",
  SessionCompleted: "session_completed",
  RecoveryStarted: "recovery_started",
  RecoveryProgress: "recovery_progress",
  RecoveryCompleted: "recovery_completed",
  RecoveryFailed: "recovery_failed",
  NetworkStateChanged: "network_state_changed",
  SystemWarning: "system_warning",
  EmergencyStopRequested: "emergency_stop_requested",
  EmergencyStopResult: "emergency_stop_result",
} as const;

export function onDeviceDiscovered(cb: (e: DeviceDiscoveredEvent) => void): Promise<UnlistenFn> {
  return listen<DeviceDiscoveredEvent>(Events.DeviceDiscovered, (ev) => cb(ev.payload));
}

export function onDeviceUpdated(cb: (e: DeviceUpdatedEvent) => void): Promise<UnlistenFn> {
  return listen<DeviceUpdatedEvent>(Events.DeviceUpdated, (ev) => cb(ev.payload));
}

export function onTrafficUpdate(cb: (s: TrafficSnapshot) => void): Promise<UnlistenFn> {
  return listen<TrafficSnapshot>(Events.TrafficUpdate, (ev) => cb(ev.payload));
}

export function onSessionStateChanged(cb: (e: SessionStateChangedEvent) => void): Promise<UnlistenFn> {
  return listen<SessionStateChangedEvent>(Events.SessionStateChanged, (ev) => cb(ev.payload));
}

export function onNetworkStateChanged(cb: (e: NetworkStateChangedEvent) => void): Promise<UnlistenFn> {
  return listen<NetworkStateChangedEvent>(Events.NetworkStateChanged, (ev) => cb(ev.payload));
}

export function onRecoveryProgress(cb: (e: RecoveryProgressEvent) => void): Promise<UnlistenFn> {
  return listen<RecoveryProgressEvent>(Events.RecoveryProgress, (ev) => cb(ev.payload));
}

export function onSystemWarning(cb: (e: SystemWarningEvent) => void): Promise<UnlistenFn> {
  return listen<SystemWarningEvent>(Events.SystemWarning, (ev) => cb(ev.payload));
}

export function onEmergencyStopRequested(cb: () => void): Promise<UnlistenFn> {
  return listen(Events.EmergencyStopRequested, () => cb());
}

export function onEmergencyStopResult(cb: (e: EmergencyStopResultEvent) => void): Promise<UnlistenFn> {
  return listen<EmergencyStopResultEvent>(Events.EmergencyStopResult, (ev) => cb(ev.payload));
}
