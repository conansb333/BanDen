/** Application shell: sidebar navigation + top status bar. */
import appIcon from "@/assets/banden-icon.png";
import { NavLink, Outlet } from "react-router-dom";
import {
  Activity,
  Gauge,
  Network,
  Settings as SettingsIcon,
  Signal,
  SlidersHorizontal,
  Waypoints,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useSystemStatus } from "@/hooks/useBanden";
import { EmergencyStopButton } from "@/components/emergency/EmergencyStop";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { CommandPalette } from "@/components/CommandPalette";
import { formatBps } from "@/lib/format";
import { useUi } from "@/stores/ui";

const NAV = [
  { to: "/", label: "Dashboard", icon: Gauge },
  { to: "/devices", label: "Devices", icon: Network },
  { to: "/map", label: "Network Map", icon: Waypoints },
  { to: "/traffic", label: "Traffic", icon: Signal },
  { to: "/controls", label: "Controls", icon: SlidersHorizontal },
  { to: "/activity", label: "Activity", icon: Activity },
  { to: "/settings", label: "Settings", icon: SettingsIcon },
];

export function NetworkStateBadge() {
  const { data: status } = useSystemStatus();
  const state = status?.network_state ?? "normal";
  const variant =
    state === "normal" ? "success" : state === "unknown" ? "destructive" : "warning";
  return (
    <Badge variant={variant as "success" | "warning" | "destructive"} className="uppercase">
      net: {state}
    </Badge>
  );
}

export function AppShell() {
  const setPaletteOpen = useUi((s) => s.setPaletteOpen);
  const { data: status } = useSystemStatus();

  return (
    <div className="flex h-screen overflow-hidden">
      {/* Sidebar */}
      <aside className="flex w-56 shrink-0 flex-col border-r bg-card">
        <div className="flex h-14 items-center gap-2 border-b px-4">
          <img src={appIcon} alt="BanDen" className="h-7 w-7 rounded-md" />
          <div className="text-sm font-semibold tracking-tight">BanDen</div>
        </div>
        <nav className="flex-1 space-y-1 p-2">
          {NAV.map(({ to, label, icon: Icon }) => (
            <NavLink
              key={to}
              to={to}
              className={({ isActive }) =>
                cn(
                  "flex items-center gap-2.5 rounded-md px-3 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground",
                  isActive && "bg-accent text-accent-foreground"
                )
              }
            >
              <Icon className="h-4 w-4" />
              {label}
            </NavLink>
          ))}
        </nav>
        <div className="border-t p-3 text-xs text-muted-foreground">
          <div className="truncate">
            {status?.network.interface_name ?? "no interface"} · {status?.network.ipv4 ?? "—"}
          </div>
          <div className="mt-1 truncate font-mono">
            {status?.network.cidr ?? ""}
          </div>
        </div>
      </aside>

      {/* Main column */}
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center gap-3 border-b bg-card px-4">
          <Button
            variant="outline"
            size="sm"
            className="h-9 w-96 justify-start gap-2 font-normal text-muted-foreground"
            onClick={() => setPaletteOpen(true)}
          >
            <span className="text-sm">Search devices, commands, pages…</span>
            <kbd className="pointer-events-none rounded border bg-muted px-1.5 font-mono text-[10px]">
              Ctrl K
            </kbd>
          </Button>
          <div className="ml-auto flex items-center gap-3">
            {status && (
              <div className="hidden items-center gap-4 text-xs text-muted-foreground lg:flex">
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="font-mono">↓ {formatBps(status.download_rate_bps)}</span>
                  </TooltipTrigger>
                  <TooltipContent>Download rate</TooltipContent>
                </Tooltip>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="font-mono">↑ {formatBps(status.upload_rate_bps)}</span>
                  </TooltipTrigger>
                  <TooltipContent>Upload rate</TooltipContent>
                </Tooltip>
                {status.latency_ms !== null && (
                  <span className="font-mono">{status.latency_ms} ms</span>
                )}
              </div>
            )}
            <NetworkStateBadge />
            <EmergencyStopButton />
          </div>
        </header>
        <main className="min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto max-w-7xl p-6">
            <Outlet />
          </div>
        </main>
      </div>

      <CommandPalette />
    </div>
  );
}
