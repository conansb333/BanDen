/**
 * Device detail drawer: Overview / Connectivity / History tabs, with the
 * identity and availability data presented on cards instead of floating
 * key/value rows.
 */
import { useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  ArrowDownRight,
  ArrowUpRight,
  Loader2,
  Radio,
  Signal,
} from "lucide-react";
import { toast } from "sonner";
import { api } from "@/lib/api";
import {
  queryKeys,
  useDeviceConnectivity,
  useDeviceEvents,
  useDeviceTraffic,
} from "@/hooks/useBanden";
import { formatBps, formatBytes, formatDateTime, relativeTime } from "@/lib/format";
import { DEVICE_KINDS, DeviceKindIcon, EventCategoryIcon } from "@/components/shared";
import type { ActivityEvent, DeviceView } from "@/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { DeviceStatusBadge } from "@/components/shared";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SessionCard } from "@/components/sessions/SessionCard";

/** Canonical kinds offered in the picker; "auto" clears the override. */
const KIND_CHOICES = [
  { value: "__auto", label: "Automatic (detect)" },
  ...DEVICE_KINDS,
];

function kindLabel(kind: string | null): string {
  if (!kind) return "Unknown";
  const hit = DEVICE_KINDS.find((k) => k.value === kind.toLowerCase());
  return hit ? hit.label : kind;
}

function Card({
  title,
  children,
  action,
}: {
  title: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="rounded-lg border bg-card">
      <div className="flex items-center justify-between px-4 pt-3 pb-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {title}
        </h3>
        {action}
      </div>
      <Separator />
      <div className="px-4 py-3">{children}</div>
    </div>
  );
}

function Row({ label, value, mono }: { label: string; value: ReactNode; mono?: boolean }) {
  return (
    <div className="flex items-center justify-between gap-4 py-1 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <span className={mono ? "text-right font-mono text-xs" : "text-right"}>{value}</span>
    </div>
  );
}

export function DeviceDrawer({
  device,
  sessions,
  onClose,
  onStartSession,
}: {
  device: DeviceView | null;
  sessions: import("@/types").SessionRecordView[];
  onClose: () => void;
  onStartSession: (d: DeviceView) => void;
}) {
  const [tab, setTab] = useState("overview");
  // Re-tab to Overview whenever a different device is opened.
  const [lastMac, setLastMac] = useState<string | null>(null);
  if (device && device.mac_address !== lastMac) {
    setLastMac(device.mac_address);
    setTab("overview");
  }

  return (
    <Sheet open={device !== null} onOpenChange={(v) => !v && onClose()}>
      <SheetContent className="w-full overflow-hidden sm:max-w-md">
        {device && (
          <ScrollArea className="h-full">
            <SheetHeader className="pb-0">
              <div className="flex items-center gap-2.5">
                <span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-primary/10">
                  <DeviceKindIcon kind={device.kind} className="h-5 w-5 text-primary" />
                </span>
                <div className="min-w-0">
                  <SheetTitle className="truncate">
                    {device.hostname ?? device.ip_address}
                  </SheetTitle>
                  <SheetDescription className="truncate">
                    {device.vendor ?? "Unknown vendor"} · {kindLabel(device.kind)}
                  </SheetDescription>
                </div>
              </div>
            </SheetHeader>

            <div className="px-3 pb-6 pt-1">
              <Tabs value={tab} onValueChange={setTab}>
                <TabsList className="grid w-full grid-cols-3">
                  <TabsTrigger value="overview">Overview</TabsTrigger>
                  <TabsTrigger value="connectivity">Connectivity</TabsTrigger>
                  <TabsTrigger value="history">History</TabsTrigger>
                </TabsList>

                <TabsContent value="overview" className="mt-3 space-y-3">
                  <OverviewTab device={device} sessions={sessions} onStartSession={onStartSession} />
                </TabsContent>
                <TabsContent value="connectivity" className="mt-3">
                  <ConnectivityTab device={device} />
                </TabsContent>
                <TabsContent value="history" className="mt-3">
                  <HistoryTab device={device} />
                </TabsContent>
              </Tabs>
            </div>
          </ScrollArea>
        )}
      </SheetContent>
    </Sheet>
  );
}

function OverviewTab({
  device,
  sessions,
  onStartSession,
}: {
  device: DeviceView;
  sessions: import("@/types").SessionRecordView[];
  onStartSession: (d: DeviceView) => void;
}) {
  const queryClient = useQueryClient();
  const { data: traffic } = useDeviceTraffic(device.mac_address);
  const [savingKind, setSavingKind] = useState(false);
  const setKind = async (value: string) => {
    setSavingKind(true);
    try {
      const kind = value === "__auto" ? null : value;
      await api.setDeviceKind(device.mac_address, device.ip_address, kind);
      toast.success(
        kind
          ? `Device type set to ${kindLabel(kind)}`
          : "Device type reset to automatic",
      );
      queryClient.invalidateQueries({ queryKey: queryKeys.devices });
    } catch (e) {
      toast.error((e as { message?: string }).message ?? "Unable to set device type");
    } finally {
      setSavingKind(false);
    }
  };

  return (
    <>
      <Card
        title="Identity"
        action={
          device.kind_source === "manual" ? (
            <Badge variant="default" className="text-[10px] uppercase">manual</Badge>
          ) : device.kind_source === "auto" ? (
            <Badge variant="secondary" className="text-[10px] uppercase">auto</Badge>
          ) : undefined
        }
      >
        <Row label="Hostname" value={device.hostname ?? "—"} />
        <Row label="IP address" value={device.ip_address} mono />
        <Row label="MAC address" value={device.mac_address} mono />
        <Row label="Vendor" value={device.vendor ?? "—"} />
        <div className="mt-2 flex items-center justify-between gap-3 pt-1">
          <span className="text-sm text-muted-foreground">Device type</span>
          <div className="flex items-center gap-2">
            {savingKind && <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />}
            <Select
              value={device.kind ?? "__auto"}
              onValueChange={setKind}
            >
              <SelectTrigger className="h-8 w-40">
                <SelectValue placeholder="Pick a type…" />
              </SelectTrigger>
              <SelectContent>
                {KIND_CHOICES.map((k) => (
                  <SelectItem key={k.value} value={k.value}>
                    {k.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      </Card>

      <Card title="Status & availability">
        <Row
          label="Status"
          value={<DeviceStatusBadge status={device.status} />}
        />
        <Row label="First seen" value={formatDateTime(device.first_seen)} />
        <Row label="Last seen" value={relativeTime(device.last_seen)} />
      </Card>

      <Card title="Traffic">
        {traffic?.live ? (
          <>
            <Row label="Download rate" value={formatBps(traffic.live.download_rate_bps)} mono />
            <Row label="Upload rate" value={formatBps(traffic.live.upload_rate_bps)} mono />
            <Row label="Total (24h)" value={formatBytes(traffic.bytes_last_24h)} mono />
          </>
        ) : (
          <p className="text-sm text-muted-foreground">No live per-device data.</p>
        )}
      </Card>

      <Card title="Sessions for this device">
        {sessions.length === 0 && (
          <p className="text-sm text-muted-foreground">No sessions yet.</p>
        )}
        <div className="space-y-2">
          {sessions.map((s) => (
            <SessionCard key={s.id} session={s} />
          ))}
        </div>
      </Card>

      <Button className="w-full" onClick={() => onStartSession(device)}>
        Start control session
      </Button>
    </>
  );
}

function ConnectivityTab({ device }: { device: DeviceView }) {
  const { data, isLoading, refetch, isRefetching } = useDeviceConnectivity(device.mac_address);
  const [pinging, setPinging] = useState(false);
  const [lastPing, setLastPing] = useState<string | null>(null);
  const queryClient = useQueryClient();

  const pingNow = async () => {
    setPinging(true);
    try {
      const r = await api.pingDevice(device.mac_address, device.ip_address);
      setLastPing(r.latency_ms !== null ? `${r.latency_ms} ms` : "no answer (timeout)");
      if (r.latency_ms !== null) {
        toast.success(`Ping reply in ${r.latency_ms} ms`);
      } else {
        toast.warning(`${device.ip_address} did not answer within 1 s`);
      }
      queryClient.invalidateQueries({ queryKey: queryKeys.deviceConnectivity(device.mac_address) });
    } catch (e) {
      toast.error((e as { message?: string }).message ?? "Ping failed");
    } finally {
      setPinging(false);
    }
  };

  const recent = (data?.samples ?? []).slice(0, 12);

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <Button size="sm" className="gap-2" onClick={pingNow} disabled={pinging}>
          {pinging ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Signal className="h-4 w-4" />
          )}
          Ping now
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() => refetch()}
          disabled={isRefetching}
          className="gap-2"
        >
          <Radio className={`h-4 w-4 ${isRefetching ? "animate-pulse" : ""}`} />
          Refresh
        </Button>
        {lastPing && (
          <span className="ml-auto font-mono text-xs text-muted-foreground">{lastPing}</span>
        )}
      </div>

      <div className="grid grid-cols-3 gap-2">
        <MiniStat title="Samples" value={String(data?.sample_count ?? 0)} loading={isLoading} />
        <MiniStat
          title="Availability"
          value={
            data && data.sample_count > 0 ? `${Math.round(data.availability_pct)}%` : "—"
          }
          loading={isLoading}
          tone={
            data && data.sample_count > 0
              ? data.availability_pct >= 80
                ? "text-emerald-400"
                : data.availability_pct >= 50
                  ? "text-amber-400"
                  : "text-red-400"
              : undefined
          }
        />
        <MiniStat
          title="Latency"
          value={data?.current_latency_ms != null ? `${data.current_latency_ms} ms` : "—"}
          loading={isLoading}
        />
      </div>

      <Card title="Recent probe results">
        {recent.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No probe history yet. BanDen samples this device about once a
            minute while it is on the network, or use Ping now.
          </p>
        ) : (
          <div className="space-y-1">
            {recent.map((p) => (
              <div
                key={p.ts}
                className="flex items-center justify-between rounded px-1.5 py-1 text-xs hover:bg-muted/50"
              >
                <span className="font-mono text-muted-foreground">
                  {new Date(p.ts * 1000).toLocaleString([], {
                    month: "short",
                    day: "numeric",
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
                </span>
                <span
                  className={
                    p.latency_ms != null
                      ? "font-mono text-emerald-400"
                      : "font-mono text-red-400"
                  }
                >
                  {p.latency_ms != null ? `${p.latency_ms} ms` : "timeout"}
                </span>
              </div>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}

function MiniStat({
  title,
  value,
  loading,
  tone,
}: {
  title: string;
  value: string;
  loading?: boolean;
  tone?: string;
}) {
  return (
    <div className="rounded-lg border bg-card px-3 py-2">
      <div className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </div>
      <div className={`mt-0.5 text-lg font-semibold tabular-nums ${tone ?? ""}`}>
        {loading ? "…" : value}
      </div>
    </div>
  );
}

function HistoryTab({ device }: { device: DeviceView }) {
  const { data: events, isLoading } = useDeviceEvents(device.mac_address, device.ip_address);

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-8 text-sm text-muted-foreground">
        <Loader2 className="mr-2 h-4 w-4 animate-spin" /> Loading history…
      </div>
    );
  }

  const list = events ?? [];
  if (list.length === 0) {
    return (
      <p className="py-6 text-center text-sm text-muted-foreground">
        No recorded events for this device yet. Online/offline transitions and
        control sessions will appear here.
      </p>
    );
  }

  const transition = (e: ActivityEvent) => {
    if (/back online/i.test(e.message)) {
      return { icon: <ArrowUpRight className="h-4 w-4 text-emerald-400" /> };
    }
    if (/stopped responding|offline/i.test(e.message)) {
      return { icon: <ArrowDownRight className="h-4 w-4 text-amber-400" /> };
    }
    return null;
  };

  return (
    <div className="space-y-0.5">
      {list.map((e) => {
        const t = transition(e);
        return (
          <div key={e.id} className="flex items-start gap-2.5 rounded-md px-1.5 py-2 hover:bg-muted/50">
            {t ? (
              <span className="mt-0.5">{t.icon}</span>
            ) : (
              <EventCategoryIcon category={e.category} />
            )}
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm">{e.message}</div>
              <div className="text-xs text-muted-foreground">
                {formatDateTime(e.timestamp)} · {relativeTime(e.timestamp)}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}
