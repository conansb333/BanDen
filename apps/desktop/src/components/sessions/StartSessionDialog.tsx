/**
 * Start-session dialog: target picker, scope, and limits presented with
 * sliders + presets instead of bare number typing.
 *
 * Semantics kept honest:
 * - Priority scales the shaper's burst allowance (not the average rate).
 * - The slider max position means "unlimited"; an exact Mbps value typed
 *   below the slider overrides it.
 */
import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Router, Timer } from "lucide-react";
import { toast } from "sonner";
import { api } from "@/lib/api";
import { queryKeys } from "@/hooks/useBanden";
import { AppIcon } from "@/components/apps/AppIcon";
import { APP_CATALOG, type NetworkDevice, type SessionConfig } from "@/types";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Slider } from "@/components/ui/slider";
import { cn } from "@/lib/utils";

function bpsFromMbps(mbps: number): number {
  return Math.round(mbps * 1_000_000);
}

const SLIDER_MAX = 200; // Mbps; the max position means "unlimited"

const DURATION_PRESETS = [
  { minutes: 30, label: "30 minutes" },
  { minutes: 60, label: "1 hour" },
  { minutes: 240, label: "4 hours" },
] as const;
const DURATION_SLIDER_MIN = 15; // minutes
const DURATION_SLIDER_MAX = 24 * 60; // 24 hours

const PRIORITIES = [
  { value: "low", label: "Low" },
  { value: "normal", label: "Normal" },
  { value: "high", label: "High" },
  { value: "max", label: "Max" },
] as const;

const SPEED_PRESETS = [
  { label: "Minimal — 2 ↓ / 1 ↑ Mbps", down: 2, up: 1 },
  { label: "Browsing — 25 ↓ / 10 ↑ Mbps", down: 25, up: 10 },
  { label: "Streaming — 50 ↓ / 20 ↑ Mbps", down: 50, up: 20 },
  { label: "Gaming — 80 ↓ / 40 ↑ Mbps", down: 80, up: 40 },
] as const;

function durationLabel(mins: number): string {
  if (mins < 60) return `${mins} minutes`;
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return m === 0 ? `${h} hour${h > 1 ? "s" : ""}` : `${h}h ${m}m`;
}

/** One rate control: slider (max = unlimited) + exact Mbps input. */
function RateSlider({
  id,
  label,
  input,
  onInput,
}: {
  id: string;
  label: string;
  input: string;
  onInput: (v: string) => void;
}) {
  const unlimited = input.trim() === "";
  const blocked = input.trim() === "0";
  const numeric = unlimited ? SLIDER_MAX : Math.min(SLIDER_MAX, Number(input));
  const state = unlimited
    ? { text: "Unlimited", cls: "text-emerald-400" }
    : blocked
      ? { text: "Blocked (full cut)", cls: "text-red-400" }
      : { text: `${input} Mbps`, cls: "text-foreground" };
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <Label htmlFor={id}>{label}</Label>
        <span className={cn("text-sm font-medium", state.cls)}>{state.text}</span>
      </div>
      <Slider
        id={id}
        min={0}
        max={SLIDER_MAX}
        step={1}
        value={[numeric]}
        onValueChange={(v) => {
          const val = v[0] ?? SLIDER_MAX;
          onInput(val >= SLIDER_MAX ? "" : String(val));
        }}
      />
      <Input
        inputMode="decimal"
        value={input}
        onChange={(e) => onInput(e.target.value.replace(/[^0-9.]/g, ""))}
        placeholder="Exact value in Mbps (empty = unlimited, 0 = blocked)"
      />
    </div>
  );
}

export function StartSessionDialog({
  open,
  onOpenChange,
  /** Candidate targets; the gateway is excluded by the caller. */
  devices,
  /** Preselected device (e.g. the one whose drawer opened the dialog). */
  initialTarget,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  devices: NetworkDevice[];
  initialTarget: NetworkDevice | null;
}) {
  const [targetId, setTargetId] = useState<string>("");
  const [scope, setScope] = useState<"device" | "apps" | "allow">("device");
  const [blockedApps, setBlockedApps] = useState<string[]>([]);
  const [allowedApps, setAllowedApps] = useState<string[]>([]);
  const [step, setStep] = useState<"configure" | "confirm">("configure");
  const [downMbps, setDownMbps] = useState("");
  const [upMbps, setUpMbps] = useState("");
  const [durationMin, setDurationMin] = useState(30);
  const [priority, setPriority] = useState<string>("normal");
  const [busy, setBusy] = useState(false);
  const queryClient = useQueryClient();

  const device = devices.find((d) => String(d.id) === targetId) ?? null;
  const limitsSet = downMbps.trim() !== "" || upMbps.trim() !== "";

  // Reset the dialog ONLY on the closed -> open transition. The devices
  // prop gets a new array identity on every react-query refetch, so
  // including it in the deps would reset the scope/target mid-editing
  // (the "tabs flip back" bug).
  const wasOpen = useRef(false);
  useEffect(() => {
    if (open && !wasOpen.current) {
      setStep("configure");
      setScope("device");
      setBlockedApps([]);
      setAllowedApps([]);
      setDownMbps("");
      setUpMbps("");
      setPriority("normal");
      setDurationMin(30);
      setTargetId(
        initialTarget
          ? String(initialTarget.id)
          : devices[0]
            ? String(devices[0].id)
            : "",
      );
    }
    wasOpen.current = open;
    // deliberately not depending on devices/initialTarget: re-renders from
    // background refetches must not reset user selections.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const close = () => {
    setStep("configure");
    onOpenChange(false);
  };

  const buildConfig = (): SessionConfig => ({
    target_mac: device?.mac_address ?? "",
    target_ip: device?.ip_address ?? "",
    target_label: device ? (device.hostname ?? device.ip_address) : null,
    download_limit_bps: downMbps.trim() ? bpsFromMbps(Number(downMbps)) : null,
    upload_limit_bps: upMbps.trim() ? bpsFromMbps(Number(upMbps)) : null,
    duration_secs: durationMin * 60,
    blocked_apps: scope === "apps" ? blockedApps : [],
    allowed_apps: scope === "allow" ? allowedApps : [],
    priority: limitsSet ? priority : "normal",
  });

  const start = async () => {
    if (!device) return;
    setBusy(true);
    try {
      await api.startSession(buildConfig());
      toast.success(
        limitsSet
          ? `Rate limit started for ${device.hostname ?? device.ip_address}`
          : `Cut started for ${device.hostname ?? device.ip_address}`,
      );
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
      queryClient.invalidateQueries({ queryKey: queryKeys.status });
      close();
    } catch (e) {
      toast.error((e as { message?: string }).message ?? "Unable to start session");
    } finally {
      setBusy(false);
    }
  };

  const applyPreset = (label: string) => {
    const preset = SPEED_PRESETS.find((p) => p.label === label);
    if (preset) {
      setDownMbps(String(preset.down));
      setUpMbps(String(preset.up));
    }
  };

  return (
    <Dialog open={open} onOpenChange={(v) => (v ? setStep("configure") : close())}>
      <DialogContent className="max-h-[92vh] max-w-lg overflow-y-auto">
        <DialogHeader>
          <DialogTitle>New control session</DialogTitle>
          <DialogDescription>
            {device
              ? `Target: ${device.hostname ?? device.ip_address} (${device.ip_address} · ${device.mac_address})`
              : "No controllable device found."}
          </DialogDescription>
        </DialogHeader>

        {step === "configure" && (
          <>
            <div className="space-y-2">
              <Label>Target device</Label>
              <Select value={targetId} onValueChange={setTargetId}>
                <SelectTrigger>
                  <SelectValue placeholder="Pick a device…" />
                </SelectTrigger>
                <SelectContent>
                  {devices.map((d) => (
                    <SelectItem key={d.id} value={String(d.id)}>
                      {d.hostname ?? d.ip_address} ({d.ip_address})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Router className="h-3 w-3" />
                The router/gateway cannot be selected - controlling it would
                take the whole network down.
              </p>
            </div>

            <Tabs
              value={scope}
              onValueChange={(v) => setScope(v as "device" | "apps" | "allow")}
            >
              <TabsList className="grid w-full grid-cols-3">
                <TabsTrigger value="device">Whole device</TabsTrigger>
                <TabsTrigger value="apps">Selected apps</TabsTrigger>
                <TabsTrigger value="allow">Allowed apps</TabsTrigger>
              </TabsList>
              <TabsContent value="device" className="mt-3">
                <p className="text-xs text-muted-foreground">
                  The whole device loses internet (or gets throttled) for
                  the duration of the session.
                </p>
              </TabsContent>
              <TabsContent value="apps" className="mt-3">
                <div className="rounded-md border p-3">
                  <Label className="mb-2 block">Block these apps</Label>
                  <div className="grid grid-cols-2 gap-x-4 gap-y-2">
                    {APP_CATALOG.map((app) => (
                      <label
                        key={app.id}
                        className="flex cursor-pointer items-center gap-2 text-sm"
                      >
                        <input
                          type="checkbox"
                          className="h-4 w-4 accent-primary"
                          checked={blockedApps.includes(app.id)}
                          onChange={(e) =>
                            setBlockedApps((cur) =>
                              e.target.checked
                                ? [...cur, app.id]
                                : cur.filter((id) => id !== app.id),
                            )
                          }
                        />
                        <span>
                          <AppIcon id={app.id} name={app.name} /> {app.name}
                        </span>
                      </label>
                    ))}
                  </div>
                  <p className="mt-2 text-xs text-muted-foreground">
                    Blocked apps lose their internet completely; everything
                    else on the device keeps working.
                  </p>
                </div>
              </TabsContent>
              <TabsContent value="allow" className="mt-3">
                <div className="rounded-md border p-3">
                  <Label className="mb-2 block">Only these apps keep internet</Label>
                  <div className="grid grid-cols-2 gap-x-4 gap-y-2">
                    {APP_CATALOG.map((app) => (
                      <label
                        key={app.id}
                        className="flex cursor-pointer items-center gap-2 text-sm"
                      >
                        <input
                          type="checkbox"
                          className="h-4 w-4 accent-primary"
                          checked={allowedApps.includes(app.id)}
                          onChange={(e) =>
                            setAllowedApps((cur) =>
                              e.target.checked
                                ? [...cur, app.id]
                                : cur.filter((id) => id !== app.id),
                            )
                          }
                        />
                        <span>
                          <AppIcon id={app.id} name={app.name} /> {app.name}
                        </span>
                      </label>
                    ))}
                  </div>
                  <p className="mt-2 text-xs text-muted-foreground">
                    Allowlist mode: only the selected apps have internet -
                    everything else on the device is cut. Strongest per-app
                    option (also stops apps that dodge DNS blocking), but
                    unselected apps won&apos;t work either.
                  </p>
                </div>
              </TabsContent>
            </Tabs>

            <div className="space-y-4">
              <RateSlider id="down" label="Download" input={downMbps} onInput={setDownMbps} />
              <RateSlider id="up" label="Upload" input={upMbps} onInput={setUpMbps} />

              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-2">
                  <Label>Speed profile</Label>
                  <Select onValueChange={applyPreset}>
                    <SelectTrigger>
                      <SelectValue placeholder="Choose preset…" />
                    </SelectTrigger>
                    <SelectContent>
                      {SPEED_PRESETS.map((p) => (
                        <SelectItem key={p.label} value={p.label}>
                          {p.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label>Mode</Label>
                  <div className="flex h-9 items-center rounded-md border px-3 text-sm">
                    {limitsSet && !(downMbps.trim() === "0" && upMbps.trim() === "0") ? "Rate limit (shaper)" : "Full cut"}
                  </div>
                </div>
              </div>

              <div className="space-y-2">
                <Label>Priority</Label>
                <div className="grid grid-cols-4 gap-2">
                  {PRIORITIES.map((p) => (
                    <button
                      key={p.value}
                      type="button"
                      onClick={() => setPriority(p.value)}
                      className={cn(
                        "rounded-md border px-2 py-1.5 text-sm transition-colors",
                        priority === p.value
                          ? "border-primary bg-primary/10 text-primary"
                          : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                      )}
                    >
                      {p.label}
                    </button>
                  ))}
                </div>
                <p className="text-xs text-muted-foreground">
                  Priority scales how much the device may burst above its
                  limit - the average rate stays exactly as set.
                </p>
              </div>

              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <Label className="flex items-center gap-1.5">
                    <Timer className="h-3.5 w-3.5" /> Duration
                  </Label>
                  <span className="text-sm font-medium">{durationLabel(durationMin)}</span>
                </div>
                <div className="grid grid-cols-3 gap-2">
                  {DURATION_PRESETS.map((p) => (
                    <button
                      key={p.minutes}
                      type="button"
                      onClick={() => setDurationMin(p.minutes)}
                      className={cn(
                        "flex items-center justify-center gap-1.5 rounded-md border px-2 py-1.5 text-sm transition-colors",
                        durationMin === p.minutes
                          ? "border-primary bg-primary/10 text-primary"
                          : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                      )}
                    >
                      <Timer className="h-3.5 w-3.5" /> {p.label}
                    </button>
                  ))}
                </div>
                <Slider
                  min={DURATION_SLIDER_MIN}
                  max={DURATION_SLIDER_MAX}
                  step={15}
                  value={[durationMin]}
                  onValueChange={(v) => setDurationMin(v[0] ?? 30)}
                />
                <div className="flex justify-between text-[10px] text-muted-foreground">
                  <span>15 min</span>
                  <span>24 hours</span>
                </div>
              </div>

              {!limitsSet && (
                <p className="text-xs text-muted-foreground">
                  No limits set: the device will be fully cut from the network
                  until the session stops. Set a limit above to keep it online
                  but throttled instead.
                </p>
              )}
            </div>

            <DialogFooter>
              <Button variant="outline" onClick={close}>
                Cancel
              </Button>
              <Button onClick={() => setStep("confirm")} disabled={!device}>
                Continue
              </Button>
            </DialogFooter>
          </>
        )}

        {step === "confirm" && (
          <>
            <Alert variant="destructive">
              <AlertTriangle className="h-4 w-4" />
              <AlertTitle>Confirm authorization</AlertTitle>
              <AlertDescription>
                {limitsSet
                  ? "REAL CONTROL - rate limit: the device stays online, but its traffic is routed through this PC and throttled to the limits above. On some routers this may briefly disturb the network. Ensure you are authorized to manage this network and this device."
                  : "REAL CONTROL - full cut: the device will lose internet connectivity until the session stops. Stopping sends corrective ARP replies so it recovers within seconds."}
                {scope === "apps" &&
                  " Note: if the device has mobile data enabled, blocked apps may switch to cellular and keep working - WiFi-side control cannot block the SIM. Turn off mobile data on the target for a guaranteed block."}
                {scope === "device" &&
                  " Note: if the device has mobile data enabled it may switch to cellular - WiFi-side control cannot block the SIM. Turn off mobile data on the target for a guaranteed block."}
              </AlertDescription>
            </Alert>
            <ul className="space-y-1 text-sm text-muted-foreground">
              <li>
                Target:{" "}
                <span className="text-foreground font-medium">
                  {device?.hostname ?? device?.ip_address}
                </span>
              </li>
              <li>
                Scope:{" "}
                <span className="text-foreground font-medium">
                  {scope === "allow"
                    ? allowedApps.length > 0
                      ? `Allow ${allowedApps.length} selected app(s) - cut everything else`
                      : "allowlist mode (no apps selected - everything cut)"
                    : scope === "apps"
                      ? blockedApps.length > 0
                        ? `Block ${blockedApps.length} selected app(s)`
                        : "whole device (no apps selected)"
                      : "whole device"}
                </span>
              </li>
              <li>
                Download:{" "}
                <span className="text-foreground font-medium">
                  {downMbps === "0" ? "blocked" : `${downMbps || "unlimited"}${downMbps ? " Mbps" : ""}`}
                </span>
              </li>
              <li>
                Upload:{" "}
                <span className="text-foreground font-medium">
                  {upMbps === "0" ? "blocked" : `${upMbps || "unlimited"}${upMbps ? " Mbps" : ""}`}
                </span>
              </li>
              {limitsSet && (
                <li>
                  Priority:{" "}
                  <span className="text-foreground font-medium capitalize">{priority}</span>
                </li>
              )}
              <li>
                Duration:{" "}
                <span className="text-foreground font-medium">
                  {durationLabel(durationMin)}
                </span>
              </li>
            </ul>
            <DialogFooter>
              <Button variant="outline" onClick={() => setStep("configure")}>
                Back
              </Button>
              <Button onClick={start} disabled={busy}>
                {busy ? "Starting…" : "Confirm & Start"}
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
