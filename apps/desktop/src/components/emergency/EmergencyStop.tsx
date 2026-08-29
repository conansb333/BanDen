/**
 * Emergency Stop UI: the destructive trigger (with confirmation) and the
 * staged progress dialog that shows each recovery step explicitly.
 */
import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Check, Loader2, ShieldAlert } from "lucide-react";
import { api } from "@/lib/api";
import { useUi } from "@/stores/ui";
import { queryKeys } from "@/hooks/useBanden";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { cn } from "@/lib/utils";

const STAGE_LABELS: Record<string, string> = {
  cancelling: "Cancelling active operations",
  stopping_controllers: "Stopping control engines",
  restoring_network_state: "Restoring network state",
  verifying_network_state: "Verifying network state",
  completed: "Completed",
  recovery_required: "Recovery required",
};

export function EmergencyStopButton({ className }: { className?: string }) {
  const [confirmOpen, setConfirmOpen] = React.useState(false);
  const beginEmergencyStop = useUi((s) => s.beginEmergencyStop);
  const finishEmergencyStop = useUi((s) => s.finishEmergencyStop);

  return (
    <>
      <Button
        variant="destructive"
        size="sm"
        className={cn("gap-1.5 font-semibold", className)}
        onClick={() => setConfirmOpen(true)}
      >
        <ShieldAlert className="h-4 w-4" />
        Emergency Stop
      </Button>
      <Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2 text-destructive">
              <AlertTriangle className="h-5 w-5" />
              Emergency Stop
            </DialogTitle>
            <DialogDescription>
              This immediately stops every active control session, restores the
              network state captured before each session, and verifies the
              result. The full pipeline runs even if some steps fail.
            </DialogDescription>
          </DialogHeader>
          <Alert variant="destructive">
            <AlertTitle>Confirm authorization</AlertTitle>
            <AlertDescription>
              You are performing an emergency stop on this machine&apos;s network
              control sessions.
            </AlertDescription>
          </Alert>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={async () => {
                setConfirmOpen(false);
                beginEmergencyStop();
                try {
                  const outcome = await api.emergencyStop();
                  finishEmergencyStop(outcome.stage, outcome.failures);
                } catch (e) {
                  // Stage events still arrive; surface the failure honestly.
                  finishEmergencyStop("recovery_required", [
                    (e as { message?: string }).message ?? "Emergency stop command failed",
                  ]);
                }
              }}
            >
              <ShieldAlert className="h-4 w-4" />
              Execute Emergency Stop
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

export function EmergencyStopProgressDialog() {
  const emergency = useUi((s) => s.emergency);
  const reset = useUi((s) => s.resetEmergencyStop);
  const queryClient = useQueryClient();
  const done = emergency.outcome !== null;

  return (
    <Dialog
      open={emergency.open}
      onOpenChange={(open) => {
        if (!open && emergency.outcome !== null) {
          reset();
          queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
          queryClient.invalidateQueries({ queryKey: queryKeys.status });
        }
      }}
    >
      <DialogContent className="max-w-md" onPointerDownOutside={(e) => e.preventDefault()}>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldAlert className="h-5 w-5 text-destructive" />
            Emergency Stop
          </DialogTitle>
          <DialogDescription>Recovery pipeline progress</DialogDescription>
        </DialogHeader>

        <div className="space-y-2 py-2">
          {emergency.stages.length === 0 && !done && (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              Preparing…
            </div>
          )}
          {emergency.stages.map((s, i) => (
            <div key={`${s.stage}-${i}`} className="flex items-center gap-2 text-sm">
              {s.done ? (
                <Check className="h-4 w-4 text-success" />
              ) : (
                <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
              )}
              <span className={cn(s.done ? "text-foreground" : "text-muted-foreground")}>
                {STAGE_LABELS[s.stage] ?? s.stage}
              </span>
              {s.detail && <span className="ml-auto text-xs text-muted-foreground font-mono">{s.detail}</span>}
            </div>
          ))}
        </div>

        {emergency.outcome === "completed" && (
          <Alert className="border-success/50">
            <Check className="h-4 w-4 text-success" />
            <AlertTitle>Network state verified</AlertTitle>
            <AlertDescription>
              All sessions stopped, restoration actions executed and verified.
            </AlertDescription>
          </Alert>
        )}
        {emergency.outcome === "recovery_required" && (
          <Alert variant="destructive">
            <AlertTriangle className="h-4 w-4" />
            <AlertTitle>Recovery required</AlertTitle>
            <AlertDescription>
              {emergency.failures.length > 0
                ? emergency.failures.join(" · ")
                : "Network state could not be fully verified. Check the Activity page."}
            </AlertDescription>
          </Alert>
        )}

        <DialogFooter>
          <Button variant="outline" disabled={!done} onClick={() => {
            reset();
            queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
            queryClient.invalidateQueries({ queryKey: queryKeys.status });
          }}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
