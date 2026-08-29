/**
 * Network Map: inferred star topology (Internet -> gateway -> devices).
 *
 * Topology honesty: only the gateway relationship is real network data.
 * Devices are laid out as a star under the detected default gateway;
 * physical wiring (switches, APs) is intentionally not invented.
 */
import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Globe, RefreshCw, Router, Wifi, WifiOff } from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys, useDevices, useSystemStatus } from "@/hooks/useBanden";
import { DeviceKindIcon, EmptyState } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";

const W = 1000;
const NODE_H = 58;
const NODE_W = 168;
const PER_ROW = 5;

export default function NetworkMap() {
  const { data: devices, isLoading, isFetching } = useDevices();
  const { data: status } = useSystemStatus();
  const gatewayIp = status?.network.gateway ?? null;
  const [onlineOnly, setOnlineOnly] = useState(false);
  const queryClient = useQueryClient();

  const refresh = async () => {
    await api.discoverDevices().catch(() => undefined);
    queryClient.invalidateQueries({ queryKey: queryKeys.devices });
    queryClient.invalidateQueries({ queryKey: queryKeys.status });
  };

  const visible = useMemo(() => {
    let list = devices ?? [];
    if (onlineOnly) {
      list = list.filter((d) => d.status === "online" || d.status === "new");
    }
    return list.slice(0, 25);
  }, [devices, onlineOnly]);

  const hiddenCount = (devices?.length ?? 0) - visible.length;
  const gateway = (devices ?? []).find((d) => d.ip_address === gatewayIp) ?? null;
  const leaves = visible.filter((d) => d.ip_address !== gatewayIp);

  const rows = Math.ceil(leaves.length / PER_ROW);
  const height = Math.max(430, 300 + rows * 120);
  const rowY = (r: number) => 268 + r * 118;
  const colX = (c: number, cols: number) =>
    W / 2 + (c - (cols - 1) / 2) * (NODE_W + 28);

  const gw = { x: W / 2 - NODE_W / 2, y: 148 };

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">Network Map</h1>
          <p className="text-sm text-muted-foreground">
            Inferred star topology from the detected gateway
          </p>
        </div>
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2">
            <Switch id="online-only" checked={onlineOnly} onCheckedChange={setOnlineOnly} />
            <Label htmlFor="online-only" className="text-sm">
              Show online only
            </Label>
          </div>
          <Button variant="outline" size="sm" className="gap-2" onClick={refresh} disabled={isFetching}>
            <RefreshCw className={`h-4 w-4 ${isFetching ? "animate-spin" : ""}`} />
            Scan network
          </Button>
        </div>
      </div>

      <div className="overflow-x-auto rounded-lg border bg-card">
        {isLoading ? (
          <div className="p-10 text-center text-sm text-muted-foreground">Mapping the network…</div>
        ) : leaves.length === 0 ? (
          <EmptyState
            className="m-6"
            icon={<Wifi className="h-8 w-8" />}
            title={onlineOnly ? "No online devices" : "No devices mapped yet"}
            description={
              onlineOnly
                ? "Everything we know about is currently offline. Turn off 'Show online only' to see all devices."
                : "Run a discovery to scan your LAN and populate the map."
            }
            action={
              <Button size="sm" variant="outline" onClick={refresh}>
                Scan network
              </Button>
            }
          />
        ) : (
          <div className="min-w-[900px] p-2">
            <svg viewBox={`0 0 ${W} ${height}`} width="100%" style={{ display: "block" }}>
              {/* dotted canvas */}
              <defs>
                <pattern id="dots" width="22" height="22" patternUnits="userSpaceOnUse">
                  <circle cx="1.5" cy="1.5" r="1.2" fill="currentColor" className="text-muted-foreground/25" />
                </pattern>
              </defs>
              <rect width={W} height={height} fill="url(#dots)" rx="10" />

              {/* uplink: internet -> gateway */}
              <line
                x1={W / 2}
                y1={64}
                x2={W / 2}
                y2={gw.y}
                stroke="currentColor"
                className="text-emerald-500/60"
                strokeWidth={1.6}
              />

              {/* gateway -> leaves */}
              {leaves.map((d, i) => {
                const r = Math.floor(i / PER_ROW);
                const cols = Math.min(PER_ROW, leaves.length - r * PER_ROW);
                const c = i % PER_ROW;
                const x = colX(c, cols) - NODE_W / 2;
                const y = rowY(r);
                const online = d.status === "online" || d.status === "new";
                return (
                  <line
                    key={d.mac_address}
                    x1={W / 2}
                    y1={gw.y + NODE_H}
                    x2={x + NODE_W / 2}
                    y2={y}
                    stroke="currentColor"
                    strokeWidth={online ? 1.5 : 1}
                    strokeDasharray={online ? undefined : "5 5"}
                    className={online ? "text-emerald-500/45" : "text-muted-foreground/25"}
                  />
                );
              })}

              {/* internet node */}
              <g>
                <rect
                  x={W / 2 - NODE_W / 2}
                  y={18}
                  width={NODE_W}
                  height={46}
                  rx="10"
                  className="fill-card stroke-border"
                  strokeWidth={1}
                />
                <foreignObject x={W / 2 - NODE_W / 2 + 10} y={28} width={30} height={26}>
                  <Globe className="h-5 w-5 text-sky-400" />
                </foreignObject>
                <text
                  x={W / 2 - NODE_W / 2 + 46}
                  y={47}
                  className="fill-foreground text-[13px] font-semibold"
                >
                  Internet
                </text>
              </g>

              {/* gateway node */}
              <g>
                <rect
                  x={gw.x}
                  y={gw.y}
                  width={NODE_W}
                  height={NODE_H}
                  rx="10"
                  className="fill-card stroke-primary/60"
                  strokeWidth={1.4}
                />
                <foreignObject x={gw.x + 10} y={gw.y + 16} width={30} height={26}>
                  <Router className="h-5 w-5 text-primary" />
                </foreignObject>
                <text x={gw.x + 46} y={gw.y + 24} className="fill-foreground text-[13px] font-semibold">
                  {gateway?.hostname ?? "Gateway"}
                </text>
                <text x={gw.x + 46} y={gw.y + 42} className="fill-muted-foreground text-[11px] font-mono">
                  {gatewayIp ?? "unknown"}
                </text>
                <circle cx={gw.x + NODE_W - 14} cy={gw.y + 14} r={4} className="fill-emerald-500" />
              </g>

              {/* device nodes */}
              {leaves.map((d, i) => {
                const r = Math.floor(i / PER_ROW);
                const cols = Math.min(PER_ROW, leaves.length - r * PER_ROW);
                const c = i % PER_ROW;
                const x = colX(c, cols) - NODE_W / 2;
                const y = rowY(r);
                const online = d.status === "online" || d.status === "new";
                const label = d.hostname ?? d.ip_address;
                return (
                  <g key={d.mac_address} opacity={online ? 1 : 0.62}>
                    <rect
                      x={x}
                      y={y}
                      width={NODE_W}
                      height={NODE_H}
                      rx="10"
                      className="fill-card stroke-border"
                      strokeWidth={1}
                    />
                    <foreignObject x={x + 9} y={y + 16} width={28} height={26}>
                      <DeviceKindIcon kind={d.kind} className="h-5 w-5" />
                    </foreignObject>
                    <text x={x + 44} y={y + 24} className="fill-foreground text-[12.5px] font-medium">
                      {label.length > 16 ? `${label.slice(0, 15)}…` : label}
                    </text>
                    <text x={x + 44} y={y + 42} className="fill-muted-foreground text-[11px] font-mono">
                      {d.ip_address}
                    </text>
                    <circle
                      cx={x + NODE_W - 14}
                      cy={y + 14}
                      r={4}
                      className={online ? "fill-emerald-500" : "fill-muted-foreground/40"}
                    />
                  </g>
                );
              })}
            </svg>

            <div className="flex flex-wrap items-center gap-4 border-t px-4 py-3 text-xs text-muted-foreground">
              <span className="flex items-center gap-1.5">
                <span className="inline-block h-2 w-2 rounded-full bg-emerald-500" /> online
              </span>
              <span className="flex items-center gap-1.5">
                <span className="inline-block h-2 w-2 rounded-full bg-muted-foreground/40" /> offline
              </span>
              <span className="flex items-center gap-1.5">
                <WifiOff className="h-3.5 w-3.5" /> dashed = no recent answer
              </span>
              {hiddenCount > 0 && <span>…and {hiddenCount} more devices</span>}
              <span className="ml-auto">
                <span className="font-medium text-foreground">Topology honesty:</span> only the
                gateway relationship is known from network data — lines are the detected default
                gateway to each device as an inferred star layout.
              </span>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
