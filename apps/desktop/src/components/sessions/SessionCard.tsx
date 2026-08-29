/** Active session card with state machine badge and stop action. */
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { api } from "@/lib/api";
import { queryKeys } from "@/hooks/useBanden";
import { formatDuration } from "@/lib/format";
import { APP_CATALOG, type SessionRecordView } from "@/types";
import { AppIcon } from "@/components/apps/AppIcon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { SessionStateBadge } from "@/components/shared";

export function SessionCard({ session }: { session: SessionRecordView }) {
  const [busy, setBusy] = useState(false);
  const queryClient = useQueryClient();
  const label = session.config.target_label ?? session.config.target_ip;
  const live = ["active", "preparing", "stopping", "restoring", "verifying", "recovery_required"].includes(
    session.state
  );

  const stop = async () => {
    setBusy(true);
    try {
      await api.stopSession(session.id);
      toast.success(`Session for ${label} stopped`);
    } catch (e) {
      toast.error((e as { message?: string }).message ?? "Unable to stop session");
    } finally {
      setBusy(false);
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
      queryClient.invalidateQueries({ queryKey: queryKeys.status });
    }
  };

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-3">
        <div className="min-w-0">
          <CardTitle className="truncate text-base">{label}</CardTitle>
          <div className="mt-0.5 font-mono text-xs text-muted-foreground">
            {session.config.target_ip} · {session.config.target_mac}
          </div>
        </div>
        <SessionStateBadge state={session.state} />
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 text-sm">
          <div className="text-muted-foreground">Download limit</div>
          <div className="font-mono">
            {session.config.download_limit_bps
              ? `${(session.config.download_limit_bps / 1_000_000).toFixed(0)} Mbps`
              : "unlimited"}
          </div>
          <div className="text-muted-foreground">Upload limit</div>
          <div className="font-mono">
            {session.config.upload_limit_bps
              ? `${(session.config.upload_limit_bps / 1_000_000).toFixed(0)} Mbps`
              : "unlimited"}
          </div>
          <div className="text-muted-foreground">Elapsed</div>
          <div className="font-mono">{formatDuration(session.active_duration_secs ?? 0)}</div>
          {session.config.duration_secs && (
            <>
              <div className="text-muted-foreground">Max duration</div>
              <div className="font-mono">{formatDuration(session.config.duration_secs)}</div>
            </>
          )}
        </div>
        {session.error && (
          <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {session.error}
          </div>
        )}
        {session.config.blocked_apps.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {session.config.blocked_apps.map((id) => {
              const app = APP_CATALOG.find((a) => a.id === id);
              return (
                <Badge key={id} variant="destructive" className="gap-1 text-[10px]">
                  {app ? (
                    <>
                      <AppIcon id={app.id} name={app.name} />
                      {app.name}
                    </>
                  ) : (
                    id
                  )}
                </Badge>
              );
            })}
          </div>
        )}
        <div className="flex items-center justify-between">
          <Badge variant="outline" className="font-mono text-[10px]">
            {session.id.slice(0, 8)}
          </Badge>
          {live && (
            <Button size="sm" variant="outline" onClick={stop} disabled={busy}>
              {busy ? "Stopping…" : "Stop"}
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
