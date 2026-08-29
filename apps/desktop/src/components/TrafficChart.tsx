/** Realtime traffic chart (Recharts) fed by the bounded live ring. */
import { Area, AreaChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { formatBps, formatTime } from "@/lib/format";
import type { TrafficSample } from "@/types";

export function TrafficChart({ samples }: { samples: TrafficSample[] }) {
  const data = samples.map((s) => ({
    t: formatTime(s.timestamp),
    down: +(s.download_rate_bps / 1_000_000).toFixed(3),
    up: +(s.upload_rate_bps / 1_000_000).toFixed(3),
    downBps: s.download_rate_bps,
    upBps: s.upload_rate_bps,
  }));

  return (
    <div className="h-64 w-full">
      <ResponsiveContainer width="100%" height="100%">
        <AreaChart data={data} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
          <defs>
            <linearGradient id="down" x1="0" y1="0" x2="0" y2="1">
              <stop offset="5%" stopColor="hsl(217, 42%, 58%)" stopOpacity={0.35} />
              <stop offset="95%" stopColor="hsl(217, 42%, 58%)" stopOpacity={0} />
            </linearGradient>
            <linearGradient id="up" x1="0" y1="0" x2="0" y2="1">
              <stop offset="5%" stopColor="hsl(150, 45%, 44%)" stopOpacity={0.3} />
              <stop offset="95%" stopColor="hsl(150, 45%, 44%)" stopOpacity={0} />
            </linearGradient>
          </defs>
          <XAxis dataKey="t" tick={{ fontSize: 10 }} tickLine={false} axisLine={false} minTickGap={48} />
          <YAxis
            tick={{ fontSize: 10 }}
            tickLine={false}
            axisLine={false}
            width={56}
            tickFormatter={(v: number) => `${v.toFixed(1)}M`}
          />
          <Tooltip
            contentStyle={{
              background: "hsl(var(--popover))",
              border: "1px solid hsl(var(--border))",
              borderRadius: 8,
              fontSize: 12,
            }}
            labelStyle={{ color: "hsl(var(--muted-foreground))" }}
            formatter={(value, name) => [formatBps(value as number), name === "down" ? "↓ Download" : "↑ Upload"]}
          />
          <Area type="monotone" dataKey="down" stroke="hsl(217, 42%, 58%)" strokeWidth={1.5} fill="url(#down)" />
          <Area type="monotone" dataKey="up" stroke="hsl(150, 45%, 44%)" strokeWidth={1.5} fill="url(#up)" />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}
