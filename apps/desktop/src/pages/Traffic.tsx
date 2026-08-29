/** Traffic: overview, per-device and connection tabs. */
import { Area, AreaChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { Cable, Signal } from "lucide-react";
import { useTrafficHistory } from "@/hooks/useBanden";
import { formatBps, formatBytes } from "@/lib/format";
import { useUi } from "@/stores/ui";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { EmptyState } from "@/components/shared";
import { TrafficChart } from "@/components/TrafficChart";

export default function Traffic() {
  const live = useUi((s) => s.live);

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Traffic</h1>
        <p className="text-sm text-muted-foreground">
          Source: {live ? "live monitor" : "waiting for samples"}
        </p>
      </div>

      <Tabs defaultValue="overview">
        <TabsList>
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="devices">Devices</TabsTrigger>
          <TabsTrigger value="connections">Connections</TabsTrigger>
        </TabsList>

        <TabsContent value="overview" className="space-y-4">
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-medium text-muted-foreground">Realtime</CardTitle>
            </CardHeader>
            <CardContent>
              {live && live.history.length > 1 ? (
                <TrafficChart samples={live.history} />
              ) : (
                <EmptyState icon={<Signal />} title="Waiting for traffic samples…" />
              )}
            </CardContent>
          </Card>

          <div className="grid gap-4 lg:grid-cols-2">
            <ProtocolCard />
            <HistoricalCard />
          </div>
        </TabsContent>

        <TabsContent value="devices">
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-medium text-muted-foreground">Per-device throughput</CardTitle>
            </CardHeader>
            <CardContent>
              {(live?.top_devices ?? []).length === 0 ? (
                <EmptyState title="No per-device data" description="Per-device throughput needs the packet-capture feed. BanDen currently reports totals from the selected interface; per-device attribution arrives with capture integration." />
              ) : (
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Device</TableHead>
                      <TableHead>IP</TableHead>
                      <TableHead className="text-right">↓ rate</TableHead>
                      <TableHead className="text-right">↑ rate</TableHead>
                      <TableHead className="text-right">↓ total</TableHead>
                      <TableHead className="text-right">↑ total</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {(live?.top_devices ?? []).map((d) => (
                      <TableRow key={d.mac_address ?? d.label}>
                        <TableCell className="font-medium">{d.label}</TableCell>
                        <TableCell className="font-mono text-xs text-muted-foreground">{d.ip_address ?? "—"}</TableCell>
                        <TableCell className="text-right font-mono">{formatBps(d.download_rate_bps)}</TableCell>
                        <TableCell className="text-right font-mono">{formatBps(d.upload_rate_bps)}</TableCell>
                        <TableCell className="text-right font-mono">{formatBytes(d.bytes_in)}</TableCell>
                        <TableCell className="text-right font-mono">{formatBytes(d.bytes_out)}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              )}
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="connections">
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-medium text-muted-foreground">Connections / flows</CardTitle>
            </CardHeader>
            <CardContent>
              <EmptyState
                icon={<Cable />}
                title="Flow capture not active"
                description="Connection-level flows need the packet-capture feed wired into the traffic monitor. The control engine already uses Npcap during sessions; feeding it into this page is on the roadmap."
              />
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  );
}

function ProtocolCard() {
  const live = useUi((s) => s.live);
  const protocols = live?.protocols ?? [];
  const totalBytes = protocols.reduce((acc, p) => acc + p.bytes, 0) || 1;
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-medium text-muted-foreground">Protocols</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2">
        {protocols.length === 0 && <EmptyState title="No protocol data" />}
        {protocols.map((p) => (
          <div key={p.protocol} className="space-y-1">
            <div className="flex items-center justify-between text-sm">
              <span className="font-mono">{p.protocol}</span>
              <span className="text-muted-foreground">{formatBytes(p.bytes)}</span>
            </div>
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
              <div
                className="h-full rounded-full bg-primary"
                style={{ width: `${Math.max(2, (p.bytes / totalBytes) * 100)}%` }}
              />
            </div>
          </div>
        ))}
      </CardContent>
    </Card>
  );
}

function HistoricalCard() {
  const { data } = useTrafficHistory(3600);
  const chartData = (data ?? []).map((p) => ({
    t: new Date(p.ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
    down: +(p.bytes_in / 1_000_000).toFixed(2),
    up: +(p.bytes_out / 1_000_000).toFixed(2),
  }));
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-medium text-muted-foreground">Last hour (persisted)</CardTitle>
      </CardHeader>
      <CardContent>
        {chartData.length < 2 ? (
          <EmptyState title="Not enough history yet" description="Samples are persisted every 10 seconds." />
        ) : (
          <div className="h-48">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={chartData} margin={{ top: 4, right: 8, left: 0, bottom: 0 }}>
                <XAxis dataKey="t" tick={{ fontSize: 10 }} tickLine={false} axisLine={false} minTickGap={40} />
                <YAxis tick={{ fontSize: 10 }} tickLine={false} axisLine={false} width={48} tickFormatter={(v: number) => `${v}M`} />
                <Tooltip
                  contentStyle={{
                    background: "hsl(var(--popover))",
                    border: "1px solid hsl(var(--border))",
                    borderRadius: 8,
                    fontSize: 12,
                  }}
                  formatter={(value, name) => [`${value} MB`, name === "down" ? "↓ In" : "↑ Out"]}
                />
                <Area type="monotone" dataKey="down" stroke="hsl(217, 42%, 58%)" strokeWidth={1.5} fill="hsl(217, 42%, 58%)" fillOpacity={0.15} />
                <Area type="monotone" dataKey="up" stroke="hsl(150, 45%, 44%)" strokeWidth={1.5} fill="hsl(150, 45%, 44%)" fillOpacity={0.15} />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
