/** Devices: searchable/sortable table + detail drawer + session start. */
import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowUpDown, Network, RefreshCw, Search } from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys, useDevices, useSessions, useSystemStatus } from "@/hooks/useBanden";
import { formatDateTime, relativeTime } from "@/lib/format";
import type { DeviceView } from "@/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { DeviceKindIcon, DeviceStatusBadge, EmptyState } from "@/components/shared";
import { DeviceDrawer } from "@/components/devices/DeviceDrawer";
import { StartSessionDialog } from "@/components/sessions/StartSessionDialog";

type SortKey = "label" | "ip" | "last_seen";

export default function Devices() {
  const { data: devices, isLoading, isFetching } = useDevices();
  const { data: sessions } = useSessions();
  const { data: status } = useSystemStatus();
  const gatewayIp = status?.network.gateway ?? null;
  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [sortKey, setSortKey] = useState<SortKey>("last_seen");
  const [selected, setSelected] = useState<DeviceView | null>(null);
  const [startOpen, setStartOpen] = useState(false);
  const [startDevice, setStartDevice] = useState<DeviceView | null>(null);
  const queryClient = useQueryClient();

  const filtered = useMemo(() => {
    let list = [...(devices ?? [])];
    const q = search.trim().toLowerCase();
    if (q) {
      list = list.filter((d) =>
        [d.hostname ?? "", d.ip_address, d.mac_address, d.vendor ?? ""]
          .join(" ")
          .toLowerCase()
          .includes(q)
      );
    }
    if (statusFilter !== "all") {
      list = list.filter((d) => d.status === statusFilter);
    }
    list.sort((a, b) => {
      switch (sortKey) {
        case "label":
          return (a.hostname ?? a.ip_address).localeCompare(b.hostname ?? b.ip_address);
        case "ip":
          return a.ip_address.localeCompare(b.ip_address, undefined, { numeric: true });
        default:
          return b.last_seen.localeCompare(a.last_seen);
      }
    });
    return list;
  }, [devices, search, statusFilter, sortKey]);

  const deviceSessions = (sessions ?? []).filter(
    (s) => selected && s.config.target_mac === selected.mac_address
  );

  const refresh = async () => {
    await api.discoverDevices().catch(() => undefined);
    queryClient.invalidateQueries({ queryKey: queryKeys.devices });
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">Devices</h1>
          <p className="text-sm text-muted-foreground">{filtered.length} of {devices?.length ?? 0} shown</p>
        </div>
        <Button variant="outline" size="sm" className="gap-2" onClick={refresh} disabled={isFetching}>
          <RefreshCw className={`h-4 w-4 ${isFetching ? "animate-spin" : ""}`} />
          {isFetching ? "Discovering…" : "Discover"}
        </Button>
      </div>

      {/* Filters */}
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative w-72">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search hostname, IP, MAC, vendor…"
            className="pl-8"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <Select value={statusFilter} onValueChange={setStatusFilter}>
          <SelectTrigger className="w-36">
            <SelectValue placeholder="Status" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All statuses</SelectItem>
            <SelectItem value="online">Online</SelectItem>
            <SelectItem value="new">New</SelectItem>
            <SelectItem value="offline">Offline</SelectItem>
            <SelectItem value="unknown">Unknown</SelectItem>
          </SelectContent>
        </Select>
        <Select value={sortKey} onValueChange={(v) => setSortKey(v as SortKey)}>
          <SelectTrigger className="w-44">
            <SelectValue placeholder="Sort" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="last_seen">Sort: last seen</SelectItem>
            <SelectItem value="ip">Sort: IP address</SelectItem>
            <SelectItem value="label">Sort: name</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {/* Table */}
      <div className="rounded-lg border bg-card">
        {isLoading ? (
          <div className="p-8 text-center text-sm text-muted-foreground">Loading devices…</div>
        ) : filtered.length === 0 ? (
          <EmptyState
            className="m-4"
            icon={<Network />}
            title="No devices found"
            description="Run a discovery to scan your LAN. BanDen reads the ARP table and probes the local subnet."
            action={
              <Button size="sm" variant="outline" onClick={refresh}>
                Discover now
              </Button>
            }
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-24">Status</TableHead>
                <TableHead>
                  <button className="inline-flex items-center gap-1" onClick={() => setSortKey("label")}>
                    Hostname <ArrowUpDown className="h-3 w-3 opacity-50" />
                  </button>
                </TableHead>
                <TableHead>
                  <button className="inline-flex items-center gap-1" onClick={() => setSortKey("ip")}>
                    IPv4 <ArrowUpDown className="h-3 w-3 opacity-50" />
                  </button>
                </TableHead>
                <TableHead>MAC</TableHead>
                <TableHead>Vendor</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>First seen</TableHead>
                <TableHead>
                  <button className="inline-flex items-center gap-1" onClick={() => setSortKey("last_seen")}>
                    Last seen <ArrowUpDown className="h-3 w-3 opacity-50" />
                  </button>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.map((d) => (
                <TableRow
                  key={d.id}
                  className="cursor-pointer"
                  onClick={() => setSelected(d)}
                >
                  <TableCell>
                    <DeviceStatusBadge status={d.status} />
                  </TableCell>
                  <TableCell className="font-medium">
                    <span className="flex items-center gap-2">
                      <DeviceKindIcon kind={d.kind} />
                      <span className="truncate">
                        {d.hostname ?? d.ip_address}
                      </span>
                    </span>
                    {d.ip_address === gatewayIp && (
                      <span className="ml-2 rounded bg-primary/15 px-1.5 py-0.5 align-middle font-mono text-[10px] text-primary">
                        GATEWAY
                      </span>
                    )}
                  </TableCell>
                  <TableCell className="font-mono">{d.ip_address}</TableCell>
                  <TableCell className="font-mono text-xs text-muted-foreground">{d.mac_address}</TableCell>
                  <TableCell>{d.vendor ?? "—"}</TableCell>
                  <TableCell className="text-muted-foreground">{d.kind ?? "—"}</TableCell>
                  <TableCell className="text-muted-foreground">{formatDateTime(d.first_seen)}</TableCell>
                  <TableCell className="text-muted-foreground">{relativeTime(d.last_seen)}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </div>

      {/* Detail drawer */}
      <DeviceDrawer
        device={selected}
        sessions={deviceSessions}
        onClose={() => setSelected(null)}
        onStartSession={(d) => {
          setStartDevice(d);
          setStartOpen(true);
        }}
      />
      <StartSessionDialog
        open={startOpen}
        onOpenChange={setStartOpen}
        devices={(devices ?? []).filter((d) => d.ip_address !== gatewayIp)}
        initialTarget={startDevice}
      />
    </div>
  );
}
