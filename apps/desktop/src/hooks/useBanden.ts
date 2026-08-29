/**
 * Query hooks: backend state lives in TanStack Query; events invalidate or
 * push updates into the cache / the UI store.
 */
import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import {
  onDeviceDiscovered,
  onDeviceUpdated,
  onEmergencyStopRequested,
  onEmergencyStopResult,
  onRecoveryProgress,
  onSessionStateChanged,
  onSystemWarning,
  onTrafficUpdate,
} from "@/lib/events";
import { useUi } from "@/stores/ui";

export const queryKeys = {
  interfaces: ["interfaces"] as const,
  network: ["network"] as const,
  devices: ["devices"] as const,
  traffic: ["traffic"] as const,
  trafficHistory: (windowSecs: number) => ["traffic-history", windowSecs] as const,
  deviceTraffic: (mac: string) => ["device-traffic", mac] as const,
  deviceConnectivity: (mac: string) => ["device-connectivity", mac] as const,
  deviceEvents: (mac: string, ip: string) => ["device-events", mac, ip] as const,
  sessions: ["sessions"] as const,
  activity: (filters?: { category?: string; search?: string }) => ["activity", filters ?? {}] as const,
  status: ["status"] as const,
  settings: ["settings"] as const,
  dataDir: ["data-dir"] as const,
};

export function useSystemStatus() {
  return useQuery({ queryKey: queryKeys.status, queryFn: api.getSystemStatus, refetchInterval: 5000 });
}

export function useDevices() {
  return useQuery({ queryKey: queryKeys.devices, queryFn: api.getDevices });
}

export function useSessions() {
  return useQuery({ queryKey: queryKeys.sessions, queryFn: api.getSessions, refetchInterval: 5000 });
}

export function useActivity(filters?: { limit?: number; category?: string; search?: string }) {
  return useQuery({
    queryKey: queryKeys.activity({ category: filters?.category, search: filters?.search }),
    queryFn: () => api.getActivity(filters),
  });
}

export function useSettings() {
  return useQuery({ queryKey: queryKeys.settings, queryFn: api.getSettings });
}

export function useTrafficHistory(windowSecs = 3600) {
  return useQuery({
    queryKey: queryKeys.trafficHistory(windowSecs),
    queryFn: () => api.getTrafficHistory(windowSecs),
    refetchInterval: 15000,
  });
}

export function useDeviceTraffic(mac: string | null) {
  return useQuery({
    queryKey: queryKeys.deviceTraffic(mac ?? ""),
    queryFn: () => api.getDeviceTraffic(mac as string),
    enabled: mac !== null,
  });
}

export function useDeviceConnectivity(mac: string | null) {
  return useQuery({
    queryKey: queryKeys.deviceConnectivity(mac ?? ""),
    queryFn: () => api.getDeviceConnectivity(mac as string),
    enabled: mac !== null,
  });
}

export function useDeviceEvents(mac: string | null, ip: string | null) {
  return useQuery({
    queryKey: queryKeys.deviceEvents(mac ?? "", ip ?? ""),
    queryFn: () => api.getDeviceEvents(mac as string, ip as string),
    enabled: mac !== null && ip !== null,
  });
}

/**
 * Global event wiring. Mount exactly once (in App).
 */
export function useBandenEvents() {
  const queryClient = useQueryClient();
  const pushTraffic = useUi((s) => s.pushTraffic);
  const emergencyStage = useUi((s) => s.emergencyStage);
  const beginEmergencyStop = useUi((s) => s.beginEmergencyStop);
  const finishEmergencyStop = useUi((s) => s.finishEmergencyStop);

  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [
      onTrafficUpdate((snapshot) => {
        pushTraffic(snapshot);
      }),
      onDeviceDiscovered(() => {
        queryClient.invalidateQueries({ queryKey: queryKeys.devices });
        queryClient.invalidateQueries({ queryKey: queryKeys.status });
      }),
      onDeviceUpdated(() => {
        queryClient.invalidateQueries({ queryKey: queryKeys.devices });
        queryClient.invalidateQueries({ queryKey: queryKeys.status });
      }),
      onSessionStateChanged(() => {
        queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
        queryClient.invalidateQueries({ queryKey: queryKeys.status });
      }),
      onSystemWarning(() => {
        queryClient.invalidateQueries({ queryKey: queryKeys.status });
      }),
      onRecoveryProgress((e) => {
        // The emergency-stop pipeline reports its stages here.
        if (
          ["cancelling", "stopping_controllers", "restoring_network_state", "verifying_network_state", "completed", "recovery_required"].includes(
            e.stage
          )
        ) {
          emergencyStage(e.stage, e.detail);
        }
        queryClient.invalidateQueries({ queryKey: queryKeys.status });
        queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
      }),
      onEmergencyStopRequested(() => {
        // Triggered via global hotkey / tray: the BACKEND already runs the
        // pipeline; the UI only opens the staged dialog (no second invoke).
        beginEmergencyStop();
      }),
      onEmergencyStopResult((e) => {
        // Outcome of the backend-initiated (hotkey/tray) emergency stop.
        if (e.outcome) {
          finishEmergencyStop(e.outcome.stage, e.outcome.failures);
        } else if (e.error) {
          finishEmergencyStop("recovery_required", [e.error]);
        }
      }),
    ];
    return () => {
      unlisteners.forEach((p) => p.then((un) => un()));
    };
  }, [queryClient, pushTraffic, emergencyStage, beginEmergencyStop, finishEmergencyStop]);
}
