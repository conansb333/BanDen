/**
 * UI-only state (Zustand). Backend state lives in TanStack Query + events;
 * this store deliberately contains no duplicated domain data beyond the
 * high-frequency traffic ring buffer that would thrash the query cache.
 */
import { create } from "zustand";
import type { TrafficSnapshot } from "@/types";

const MAX_LIVE_SAMPLES = 180;

interface EmergencyStopProgress {
  open: boolean;
  stages: { stage: string; detail: string | null; done: boolean }[];
  outcome: "completed" | "recovery_required" | null;
  failures: string[];
}

interface UiState {
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;

  /** Bounded live traffic ring for the realtime chart. */
  live: TrafficSnapshot | null;
  pushTraffic: (snapshot: TrafficSnapshot) => void;

  emergency: EmergencyStopProgress;
  beginEmergencyStop: () => void;
  emergencyStage: (stage: string, detail: string | null) => void;
  finishEmergencyStop: (outcome: "completed" | "recovery_required", failures: string[]) => void;
  resetEmergencyStop: () => void;

  /** Toast suppression when notifications are off. */
  notificationsEnabled: boolean;
  setNotificationsEnabled: (v: boolean) => void;
}

const EMPTY_EMERGENCY: EmergencyStopProgress = {
  open: false,
  stages: [],
  outcome: null,
  failures: [],
};

export const useUi = create<UiState>((set) => ({
  paletteOpen: false,
  setPaletteOpen: (open) => set({ paletteOpen: open }),

  live: null,
  pushTraffic: (snapshot) =>
    set({
      live: {
        ...snapshot,
        history: snapshot.history.slice(-MAX_LIVE_SAMPLES),
      },
    }),

  emergency: EMPTY_EMERGENCY,
  beginEmergencyStop: () =>
    set({
      emergency: { open: true, stages: [], outcome: null, failures: [] },
    }),
  emergencyStage: (stage, detail) =>
    set((s) => ({
      emergency: {
        ...s.emergency,
        open: true,
        stages: [
          ...s.emergency.stages.map((x) => ({ ...x, done: true })),
          { stage, detail, done: false },
        ],
      },
    })),
  finishEmergencyStop: (outcome, failures) =>
    set((s) => ({
      emergency: {
        ...s.emergency,
        open: true,
        stages: s.emergency.stages.map((x) => ({ ...x, done: true })),
        outcome,
        failures,
      },
    })),
  resetEmergencyStop: () => set({ emergency: EMPTY_EMERGENCY }),

  notificationsEnabled: true,
  setNotificationsEnabled: (v) => set({ notificationsEnabled: v }),
}));
