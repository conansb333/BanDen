/** Activity: event feed with icons, severity filters and search. */
import { useMemo, useState } from "react";
import { Search } from "lucide-react";
import { useActivity } from "@/hooks/useBanden";
import { formatDateTime, relativeTime } from "@/lib/format";
import type { EventCategory } from "@/types";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { EmptyState, EventCategoryIcon } from "@/components/shared";

const CATEGORY_VARIANTS: Record<EventCategory, "secondary" | "warning" | "destructive" | "success" | "default" | "muted"> = {
  INFO: "secondary",
  WARNING: "warning",
  ERROR: "destructive",
  RECOVERY: "success",
  NETWORK: "default",
  SESSION: "muted",
};

const CATEGORY_LABELS: Record<EventCategory, string> = {
  INFO: "Info",
  WARNING: "Warning",
  ERROR: "Error",
  RECOVERY: "Recovery",
  NETWORK: "Network",
  SESSION: "Session",
};

/** Pull a short human headline out of the event message. */
function headline(message: string): { title: string; detail: string | null } {
  const idx = message.indexOf(": ");
  if (idx > 0 && idx < 40) {
    return { title: message.slice(0, idx), detail: message.slice(idx + 2) };
  }
  return { title: message, detail: null };
}

export default function Activity() {
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState<string>("all");
  const { data, isLoading, refetch } = useActivity({
    limit: 300,
    category: category === "all" ? undefined : category,
    search: search.trim() || undefined,
  });

  const events = useMemo(() => data ?? [], [data]);

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Activity</h1>
        <p className="text-sm text-muted-foreground">
          Device transitions, sessions, network changes and recovery — newest first
        </p>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <div className="relative w-72">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search events…"
            className="pl-8"
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              refetch();
            }}
          />
        </div>
        <Select value={category} onValueChange={setCategory}>
          <SelectTrigger className="w-40">
            <SelectValue placeholder="Category" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All severities</SelectItem>
            <SelectItem value="INFO">Info</SelectItem>
            <SelectItem value="WARNING">Warning</SelectItem>
            <SelectItem value="ERROR">Error</SelectItem>
            <SelectItem value="RECOVERY">Recovery</SelectItem>
            <SelectItem value="NETWORK">Network</SelectItem>
            <SelectItem value="SESSION">Session</SelectItem>
          </SelectContent>
        </Select>
        <span className="ml-auto text-xs text-muted-foreground">{events.length} events</span>
      </div>

      <div className="rounded-lg border bg-card">
        {isLoading ? (
          <div className="p-8 text-center text-sm text-muted-foreground">Loading…</div>
        ) : events.length === 0 ? (
          <EmptyState className="m-4" title="No matching events" />
        ) : (
          <ScrollArea className="h-[calc(100vh-260px)]">
            <div className="divide-y">
              {events.map((e) => {
                const { title, detail } = headline(e.message);
                return (
                  <div key={e.id} className="flex items-start gap-3 px-4 py-3">
                    <EventCategoryIcon category={e.category} />
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-medium leading-tight">{title}</span>
                        <Badge
                          variant={CATEGORY_VARIANTS[e.category] ?? "secondary"}
                          className="px-1.5 py-0 text-[10px] uppercase"
                        >
                          {CATEGORY_LABELS[e.category] ?? e.category}
                        </Badge>
                      </div>
                      {detail && <div className="mt-0.5 text-sm text-muted-foreground">{detail}</div>}
                      <div className="mt-1 flex items-center gap-2 text-xs text-muted-foreground">
                        <span>{formatDateTime(e.timestamp)}</span>
                        <span>·</span>
                        <span>{relativeTime(e.timestamp)}</span>
                      </div>
                      {e.details != null && (
                        <details className="mt-1 text-xs text-muted-foreground">
                          <summary className="cursor-pointer select-none">details</summary>
                          <pre className="mt-1 max-w-md overflow-auto rounded bg-muted p-2 text-[10px]">
                            {JSON.stringify(e.details, null, 2)}
                          </pre>
                        </details>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </ScrollArea>
        )}
      </div>
    </div>
  );
}
