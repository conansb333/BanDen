/** Global command palette (Ctrl+K). */
import { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import {
  Activity,
  Waypoints,
  Gauge,
  Network,
  Play,
  Settings as SettingsIcon,
  ShieldAlert,
  Signal,
  SlidersHorizontal,
  Square,
} from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys, useSessions } from "@/hooks/useBanden";
import { useUi } from "@/stores/ui";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command";

export function CommandPalette() {
  const open = useUi((s) => s.paletteOpen);
  const setOpen = useUi((s) => s.setPaletteOpen);
  const beginEmergencyStop = useUi((s) => s.beginEmergencyStop);
  const finishEmergencyStop = useUi((s) => s.finishEmergencyStop);
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { data: sessions } = useSessions();
  const activeSession = sessions?.find((s) => s.state === "active");

  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setOpen(!open);
      }
    };
    document.addEventListener("keydown", down);
    return () => document.removeEventListener("keydown", down);
  }, [open, setOpen]);

  const go = (path: string) => {
    setOpen(false);
    navigate(path);
  };

  return (
    <CommandDialog open={open} onOpenChange={setOpen}>
      <CommandInput placeholder="Type a command or search…" />
      <CommandList>
        <CommandEmpty>No results found.</CommandEmpty>
        <CommandGroup heading="Navigation">
          <CommandItem onSelect={() => go("/")}>
            <Gauge /> Open Dashboard
          </CommandItem>
          <CommandItem onSelect={() => go("/devices")}>
            <Network /> Open Devices
          </CommandItem>
          <CommandItem onSelect={() => go("/map")}>
            <Waypoints /> Open Network Map
          </CommandItem>
          <CommandItem onSelect={() => go("/traffic")}>
            <Signal /> Open Traffic
          </CommandItem>
          <CommandItem onSelect={() => go("/controls")}>
            <SlidersHorizontal /> Open Controls
          </CommandItem>
          <CommandItem onSelect={() => go("/activity")}>
            <Activity /> Open Activity
          </CommandItem>
          <CommandItem onSelect={() => go("/settings")}>
            <SettingsIcon /> Open Settings
          </CommandItem>
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="Actions">
          <CommandItem
            onSelect={async () => {
              setOpen(false);
              await api.discoverDevices();
              queryClient.invalidateQueries({ queryKey: queryKeys.devices });
              queryClient.invalidateQueries({ queryKey: queryKeys.status });
            }}
          >
            <Network /> Discover devices
          </CommandItem>
          <CommandItem
            onSelect={async () => {
              setOpen(false);
              await api.startMonitoring();
              queryClient.invalidateQueries({ queryKey: queryKeys.status });
            }}
          >
            <Play /> Start monitoring
          </CommandItem>
          {activeSession && (
            <CommandItem
              onSelect={async () => {
                setOpen(false);
                await api.stopSession(activeSession.id).catch(() => undefined);
                queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
              }}
            >
              <Square /> Stop active session ({activeSession.config.target_label ?? activeSession.config.target_ip})
            </CommandItem>
          )}
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="Safety">
          <CommandItem
            className="text-destructive"
            onSelect={() => {
              setOpen(false);
              beginEmergencyStop();
              void api
                .emergencyStop()
                .then((outcome) => finishEmergencyStop(outcome.stage, outcome.failures))
                .catch((e) =>
                  finishEmergencyStop("recovery_required", [
                    (e as { message?: string }).message ?? "Emergency stop command failed",
                  ])
                );
            }}
          >
            <ShieldAlert /> Emergency Stop
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
