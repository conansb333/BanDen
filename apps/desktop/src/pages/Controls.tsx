/** Controls: active sessions, network state, emergency stop. */
import { Plus } from "lucide-react";
import { useSessions, useSystemStatus } from "@/hooks/useBanden";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { EmptyState } from "@/components/shared";
import { SessionCard } from "@/components/sessions/SessionCard";
import { StartSessionDialog } from "@/components/sessions/StartSessionDialog";
import { useDevices } from "@/hooks/useBanden";
import { useState } from "react";

const LIVE_STATES = ["active", "preparing", "stopping", "restoring", "verifying", "recovery_required"];

export default function Controls() {
  const { data: sessions } = useSessions();
  const { data: status } = useSystemStatus();
  const { data: devices } = useDevices();
  const [startOpen, setStartOpen] = useState(false);
  const [target, setTarget] = useState<import("@/types").NetworkDevice | null>(null);

  const live = (sessions ?? []).filter((s) => LIVE_STATES.includes(s.state));
  const history = (sessions ?? []).filter((s) => !LIVE_STATES.includes(s.state)).slice(0, 6);
  // The gateway itself is never a valid target (the backend refuses it
  // anyway); presenting it invited exactly the wrong click.
  //
  // A power-saving phone stops answering discovery probes minutes after
  // its screen locks while still being on the WiFi, so the registry
  // flips it "offline". Excluding those made the session picker empty
  // and pushed users toward wrong rows - keep any device seen in the
  // last 15 minutes selectable (the backend re-verifies at session
  // start and refuses unreachable targets).
  const gatewayIp = status?.network.gateway ?? null;
  const RECENT_MS = 15 * 60 * 1000;
  const targets = (devices ?? []).filter(
    (d) =>
      d.ip_address !== gatewayIp &&
      (d.status === "online" ||
        d.status === "new" ||
        (d.status === "offline" && Date.now() - Date.parse(d.last_seen) < RECENT_MS)),
  );

  const state = status?.network_state ?? "normal";
  const stateVariant = state === "normal" ? "success" : state === "unknown" ? "destructive" : "warning";

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">Controls</h1>
          <p className="text-sm text-muted-foreground">
            Session-based traffic control with guaranteed restoration
          </p>
        </div>
        <Button
          size="sm"
          className="gap-2"
          disabled={targets.length === 0}
          onClick={() => {
            setTarget(null); // dialog defaults to its first candidate
            setStartOpen(true);
          }}
        >
          <Plus className="h-4 w-4" />
          New session
        </Button>
      </div>

      {/* Network state banner */}
      <Card>
        <CardContent className="flex flex-wrap items-center gap-3 py-4">
          <span className="text-sm text-muted-foreground">Network state:</span>
          <Badge variant={stateVariant as "success" | "warning" | "destructive"} className="uppercase">
            {state}
          </Badge>
          <span className="text-sm text-muted-foreground">·</span>
          <span className="text-sm text-muted-foreground">
            Watchdog: {status?.watchdog_running ? "running" : "off"}
          </span>
        </CardContent>
      </Card>

      {state === "unknown" && (
        <Alert variant="destructive">
          <AlertTitle>Network state unknown</AlertTitle>
          <AlertDescription>
            A previous restoration could not be verified. Review the Activity page before starting
            new sessions.
          </AlertDescription>
        </Alert>
      )}

      {/* Live sessions */}
      <section className="space-y-3">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">
          Active sessions
        </h2>
        {live.length === 0 ? (
          <EmptyState
            title="No active sessions"
            description="Start a session from the Devices page or the button above. Every session captures state, registers a restoration journal entry and verifies restoration when it stops."
          />
        ) : (
          <div className="grid gap-4 lg:grid-cols-2">
            {live.map((s) => (
              <SessionCard key={s.id} session={s} />
            ))}
          </div>
        )}
      </section>

      {/* History */}
      {history.length > 0 && (
        <section className="space-y-3">
          <h2 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">
            Recent history
          </h2>
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-xs font-mono text-muted-foreground">
                target · state · started · ended
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-1.5 text-sm">
              {history.map((s) => (
                <div key={s.id} className="flex items-center gap-3 font-mono text-xs">
                  <span className="w-40 truncate font-sans font-medium">
                    {s.config.target_label ?? s.config.target_ip}
                  </span>
                  <span
                    className={
                      s.state === "completed" ? "text-muted-foreground" : "text-destructive"
                    }
                  >
                    {s.state}
                  </span>
                  <span className="ml-auto text-muted-foreground">
                    {s.started_at ? new Date(s.started_at).toLocaleTimeString() : "—"} →{" "}
                    {s.ended_at ? new Date(s.ended_at).toLocaleTimeString() : "—"}
                  </span>
                </div>
              ))}
            </CardContent>
          </Card>
        </section>
      )}

      <StartSessionDialog
        open={startOpen}
        onOpenChange={setStartOpen}
        devices={targets}
        initialTarget={target}
      />
    </div>
  );
}
