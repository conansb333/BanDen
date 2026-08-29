/** Dashboard: network overview, live traffic, top devices, recent activity. */
import { Link } from "react-router-dom";
import { useMemo } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  Network,
  RefreshCw,
  ShieldAlert,
  Signal,
  Timer,
  Wifi,
  WifiOff,
} from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys, useActivity, useDevices, useSystemStatus } from "@/hooks/useBanden";
import { formatBps, relativeTime } from "@/lib/format";
import { useUi } from "@/stores/ui";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { TrafficChart } from "@/components/TrafficChart";
import {
  DeviceKindIcon,
  DeviceStatusBadge,
  EmptyState,
  EventCategoryIcon,
  StatCard,
} from "@/components/shared";

export default function Dashboard() {
  const { data: status, isLoading } = useSystemStatus();
  const { data: devices } = useDevices();
  const { data: activity } = useActivity({ limit: 8 });
  const live = useUi((s) => s.live);
  const queryClient = useQueryClient();

  const samples = live?.history ?? [];
  const topDevices = (live?.top_devices ?? []).slice(0, 5);

  const offlineCount = Math.max(
    0,
    (status?.device_count ?? 0) - (status?.online_devices ?? 0),
  );

  const recentlySeen = useMemo(() => {
    return [...(devices ?? [])]
      .sort((a, b) => b.last_seen.localeCompare(a.last_seen))
      .slice(0, 8);
  }, [devices]);

  const refresh = async () => {
    await api.discoverDevices().catch(() => undefined);
    queryClient.invalidateQueries({ queryKey: queryKeys.devices });
    queryClient.invalidateQueries({ queryKey: queryKeys.status });
  };

  return (
    <div className="space-y-6">
      {/* Network header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">
            {status?.network.interface_name ?? "No network"}
          </h1>
          <div className="mt-1 flex items-center gap-2 text-sm text-muted-foreground">
            <span className="font-mono">{status?.network.ipv4 ?? "—"}</span>
            {status?.network.cidr && (
              <>
                <span>·</span>
                <span className="font-mono">{status.network.cidr}</span>
              </>
            )}
            <Badge variant={status?.network.connection_status === "connected" ? "success" : "muted"} className="uppercase">
              {status?.network.connection_status ?? "unknown"}
            </Badge>
          </div>
        </div>
        <Button
          variant="outline"
          size="sm"
          className="gap-2"
          onClick={refresh}
        >
          <RefreshCw className="h-4 w-4" />
          Discover devices
        </Button>
      </div>

      {/* Warnings */}
      {(status?.warnings ?? []).length > 0 && (
        <div className="space-y-2">
          {status!.warnings.map((w) => (
            <div
              key={w.code}
              className="flex items-center gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-sm text-warning"
            >
              <ShieldAlert className="h-4 w-4 shrink-0" />
              <span className="flex-1">{w.message}</span>
              <span className="font-mono text-xs opacity-70">{relativeTime(w.timestamp)}</span>
            </div>
          ))}
        </div>
      )}

      {/* Stat cards */}
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <StatCard
          title="Devices"
          value={status?.device_count ?? 0}
          sub={`${status?.online_devices ?? 0} online · ${offlineCount} offline`}
          icon={<Network className="h-4 w-4" />}
          tone="violet"
          loading={isLoading}
        />
        <StatCard
          title="Online"
          value={status?.online_devices ?? 0}
          sub="responding now"
          icon={<Wifi className="h-4 w-4" />}
          tone="success"
          loading={isLoading}
        />
        <StatCard
          title="Offline"
          value={offlineCount}
          sub="not responding"
          icon={<WifiOff className="h-4 w-4" />}
          tone="warning"
          loading={isLoading}
        />
        <StatCard
          title="Avg latency"
          value={
            status?.latency_ms !== null && status?.latency_ms !== undefined
              ? `${status.latency_ms} ms`
              : "—"
          }
          sub={status?.network.gateway ? `gateway ${status.network.gateway}` : "no gateway"}
          icon={<Timer className="h-4 w-4" />}
          tone="info"
          loading={isLoading}
        />
      </div>

      {/* Live traffic */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm font-medium text-muted-foreground">Traffic (realtime)</CardTitle>
        </CardHeader>
        <CardContent>
          {samples.length > 1 ? (
            <TrafficChart samples={samples} />
          ) : (
            <EmptyState
              title="Waiting for traffic samples…"
              description="The monitor samples the active interface once per second. If this stays empty, check the selected interface in Settings."
            />
          )}
        </CardContent>
      </Card>

      <div className="grid gap-4 lg:grid-cols-2">
        {/* Top devices */}
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">Top devices</CardTitle>
          </CardHeader>
          <CardContent className="space-y-1">
            {topDevices.length === 0 && (
              <EmptyState title="No per-device data" description="Run a discovery so per-device traffic can be attributed to known devices." />
            )}
            {topDevices.map((d) => (
              <div key={d.mac_address ?? d.label} className="flex items-center justify-between rounded-md px-2 py-1.5 text-sm hover:bg-muted/50">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate">{d.label}</span>
                  {d.ip_address && <span className="font-mono text-xs text-muted-foreground">{d.ip_address}</span>}
                </div>
                <span className="font-mono text-sm">
                  ↓ {formatBps(d.download_rate_bps)}
                </span>
              </div>
            ))}
            <Link to="/traffic" className="block px-2 pt-1 text-xs text-muted-foreground hover:text-foreground">
              View all traffic →
            </Link>
          </CardContent>
        </Card>

        {/* Recent activity */}
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">Recent events</CardTitle>
          </CardHeader>
          <CardContent className="space-y-1">
            {(activity ?? []).length === 0 && <EmptyState title="No activity yet" />}
            {(activity ?? []).map((e) => (
              <div key={e.id} className="flex items-center gap-3 rounded-md px-2 py-1.5 text-sm hover:bg-muted/50">
                <EventCategoryIcon category={e.category} />
                <span className="min-w-0 flex-1 truncate">{e.message}</span>
                <span className="shrink-0 text-xs text-muted-foreground">
                  {relativeTime(e.timestamp)}
                </span>
              </div>
            ))}
            <Link to="/activity" className="block px-2 pt-1 text-xs text-muted-foreground hover:text-foreground">
              View all activity →
            </Link>
          </CardContent>
        </Card>
      </div>

      {/* Recently seen devices */}
      <Card>
        <CardHeader className="pb-2">
          <div className="flex items-center justify-between">
            <CardTitle className="text-sm font-medium text-muted-foreground">
              Recently seen devices
            </CardTitle>
            <Link
              to="/devices"
              className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
            >
              All devices →
            </Link>
          </div>
        </CardHeader>
        <CardContent>
          {recentlySeen.length === 0 ? (
            <EmptyState
              icon={<Signal className="h-8 w-8" />}
              title="No devices seen yet"
              action={
                <Button size="sm" variant="outline" onClick={refresh}>
                  Discover now
                </Button>
              }
            />
          ) : (
            <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
              {recentlySeen.map((d) => (
                <Link
                  key={d.mac_address}
                  to="/devices"
                  className="flex items-center gap-2.5 rounded-lg border bg-card px-3 py-2.5 transition-colors hover:border-primary/40 hover:bg-accent/40"
                >
                  <span className="grid h-8 w-8 shrink-0 place-items-center rounded-md bg-primary/10">
                    <DeviceKindIcon kind={d.kind} className="h-4 w-4 text-primary" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium">
                      {d.hostname ?? d.ip_address}
                    </span>
                    <span className="block truncate font-mono text-xs text-muted-foreground">
                      {d.ip_address}
                    </span>
                  </span>
                  <DeviceStatusBadge status={d.status} />
                </Link>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

    </div>
  );
}
