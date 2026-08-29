/** Settings: general, network, safety, data, about. */
import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { api } from "@/lib/api";
import { queryKeys, useSettings, useSystemStatus } from "@/hooks/useBanden";
import type { AppSettings as AppSettingsType, InterfaceInfo } from "@/types";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

export default function Settings() {
  const { data: saved } = useSettings();
  const { data: status } = useSystemStatus();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState<AppSettingsType | null>(null);
  const [saving, setSaving] = useState(false);
  const [dataDir, setDataDir] = useState("");
  const { data: interfaces } = useQuery({
    queryKey: queryKeys.interfaces,
    queryFn: api.getNetworkInterfaces,
  });
  const usable: InterfaceInfo[] = (interfaces ?? []).filter((i) => !i.is_loopback && i.is_up);

  useEffect(() => {
    if (saved && !draft) setDraft(structuredClone(saved));
  }, [saved, draft]);

  useEffect(() => {
    api.getDataDir().then(setDataDir).catch(() => undefined);
  }, []);

  if (!draft) {
    return <div className="p-8 text-sm text-muted-foreground">Loading settings…</div>;
  }

  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);

  const save = async () => {
    setSaving(true);
    try {
      await api.updateSettings(draft);
      queryClient.invalidateQueries({ queryKey: queryKeys.settings });
      toast.success("Settings saved");
    } catch (e) {
      toast.error((e as { message?: string }).message ?? "Unable to save settings");
    } finally {
      setSaving(false);
    }
  };

  const setGeneral = (patch: Partial<AppSettingsType["general"]>) =>
    setDraft({ ...draft, general: { ...draft.general, ...patch } });
  const setNetwork = (patch: Partial<AppSettingsType["network"]>) =>
    setDraft({ ...draft, network: { ...draft.network, ...patch } });
  const setSafety = (patch: Partial<AppSettingsType["safety"]>) =>
    setDraft({ ...draft, safety: { ...draft.safety, ...patch } });
  const setData = (patch: Partial<AppSettingsType["data"]>) =>
    setDraft({ ...draft, data: { ...draft.data, ...patch } });

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold tracking-tight">Settings</h1>
        <div className="flex items-center gap-2">
          {dirty && <span className="text-xs text-warning">unsaved changes</span>}
          <Button size="sm" onClick={save} disabled={!dirty || saving}>
            {saving ? "Saving…" : "Save changes"}
          </Button>
        </div>
      </div>

      <Tabs defaultValue="general">
        <TabsList>
          <TabsTrigger value="general">General</TabsTrigger>
          <TabsTrigger value="network">Network</TabsTrigger>
          <TabsTrigger value="safety">Safety</TabsTrigger>
          <TabsTrigger value="data">Data</TabsTrigger>
          <TabsTrigger value="about">About</TabsTrigger>
        </TabsList>

        <TabsContent value="general" className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>General</CardTitle>
              <CardDescription>Application behaviour</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <Toggle
                label="Start with Windows"
                description="Launch BanDen automatically at sign-in."
                checked={draft.general.start_with_windows}
                onChange={(v) => setGeneral({ start_with_windows: v })}
              />
              <Separator />
              <Toggle
                label="Minimize to tray"
                description="Closing the window keeps BanDen running in the system tray."
                checked={draft.general.minimize_to_tray}
                onChange={(v) => setGeneral({ minimize_to_tray: v })}
              />
              <Separator />
              <Toggle
                label="Notifications"
                description="Show toast notifications for important events."
                checked={draft.general.notifications}
                onChange={(v) => setGeneral({ notifications: v })}
              />
              <Separator />
              <div className="flex items-center justify-between gap-4">
                <div>
                  <Label>Theme</Label>
                  <p className="text-xs text-muted-foreground">Appearance preference.</p>
                </div>
                <Select
                  value={draft.general.theme}
                  onValueChange={(v) => {
                    setGeneral({ theme: v as AppSettingsType["general"]["theme"] });
                    applyTheme(v as AppSettingsType["general"]["theme"]);
                  }}
                >
                  <SelectTrigger className="w-36">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="system">System</SelectItem>
                    <SelectItem value="light">Light</SelectItem>
                    <SelectItem value="dark">Dark</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="network" className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Network</CardTitle>
              <CardDescription>Discovery and monitoring preferences</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="flex items-center justify-between gap-4">
                <div>
                  <Label>Default network interface</Label>
                  <p className="text-xs text-muted-foreground">
                    Automatic selection picks the first up physical adapter with a gateway.
                  </p>
                </div>
                <Select
                  value={draft.network.default_interface ?? "auto"}
                  onValueChange={(v) =>
                    setNetwork({ default_interface: v === "auto" ? null : v })
                  }
                >
                  <SelectTrigger className="w-56">
                    <SelectValue placeholder="Automatic" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="auto">Automatic</SelectItem>
                    {usable.map((i) => (
                      <SelectItem key={i.id} value={i.id}>
                        {i.friendly_name ?? i.name}
                        {i.ipv4 ? ` (${i.ipv4})` : ""}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <Separator />
              <div className="flex items-center justify-between gap-4">
                <div>
                  <Label>Discovery interval (seconds)</Label>
                  <p className="text-xs text-muted-foreground">
                    How often to re-scan the LAN in the background.
                  </p>
                </div>
                <Input
                  className="w-24 text-right"
                  inputMode="numeric"
                  value={String(draft.network.discovery_interval_secs)}
                  onChange={(e) =>
                    setNetwork({ discovery_interval_secs: Math.max(5, Number(e.target.value) || 5) })
                  }
                />
              </div>
              <Separator />
              <Toggle
                label="Monitoring enabled"
                description="Periodic discovery and traffic sampling."
                checked={draft.network.monitoring_enabled}
                onChange={(v) => setNetwork({ monitoring_enabled: v })}
              />
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="safety" className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Safety</CardTitle>
              <CardDescription>
                These settings govern restoration and must not be disabled casually.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <Toggle
                label="Automatic cleanup"
                description="Restore network state during normal application shutdown."
                checked={draft.safety.automatic_cleanup}
                onChange={(v) => setSafety({ automatic_cleanup: v })}
              />
              <Separator />
              <Toggle
                label="Recovery watchdog"
                description="Run the independent watchdog process that restores state if BanDen dies."
                checked={draft.safety.recovery_watchdog}
                onChange={(v) => setSafety({ recovery_watchdog: v })}
              />
              <Separator />
              <Toggle
                label="Authorization confirmation"
                description="Require an explicit confirmation before starting control sessions."
                checked={draft.safety.authorization_confirmation}
                onChange={(v) => setSafety({ authorization_confirmation: v })}
              />
              <Separator />
              <Toggle
                label="Restoration verification"
                description="Verify network state after every session stop, not only emergencies."
                checked={draft.safety.restoration_verification}
                onChange={(v) => setSafety({ restoration_verification: v })}
              />
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="data" className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Data</CardTitle>
              <CardDescription>Local database and retention</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="flex items-center justify-between gap-4">
                <div>
                  <Label>Database location</Label>
                  <p className="font-mono text-xs text-muted-foreground">{dataDir || "…"}</p>
                </div>
              </div>
              <Separator />
              <div className="flex items-center justify-between gap-4">
                <div>
                  <Label>Retention period (days)</Label>
                  <p className="text-xs text-muted-foreground">
                    Traffic samples and activity older than this are purged.
                  </p>
                </div>
                <Input
                  className="w-24 text-right"
                  inputMode="numeric"
                  value={String(draft.data.retention_days)}
                  onChange={(e) => setData({ retention_days: Math.max(1, Number(e.target.value) || 1) })}
                />
              </div>
              <Separator />
              <div className="flex items-center justify-between gap-4">
                <div>
                  <Label>Clear history</Label>
                  <p className="text-xs text-muted-foreground">
                    Deletes all stored traffic samples and activity events.
                  </p>
                </div>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={async () => {
                    await api.clearHistory().catch(() => undefined);
                    toast.success("History cleared");
                    queryClient.invalidateQueries({ queryKey: queryKeys.activity() });
                  }}
                >
                  Clear now
                </Button>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="about" className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>About BanDen</CardTitle>
              <CardDescription>Windows network traffic management &amp; analysis</CardDescription>
            </CardHeader>
            <CardContent className="space-y-2 text-sm">
              <Row label="Version" value="0.1.0" mono />
              <Row label="Capture source" value={status?.capture_source ?? "—"} />
              <Row label="Watchdog" value={status?.watchdog_running ? "running" : "not running"} />
              <Row label="License" value="MIT" />
              <Row label="Repository" value="github.com/banden-app/banden" mono />
              <p className="pt-4 text-xs text-muted-foreground">
                BanDen is intended for authorized network administration on personal
                networks, home labs and controlled environments. Misuse on networks
                you are not authorized to manage is prohibited by the license terms.
              </p>
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  );
}

function Toggle({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div>
        <Label>{label}</Label>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
      <Switch checked={checked} onCheckedChange={onChange} />
    </div>
  );
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-center justify-between gap-4">
      <span className="text-muted-foreground">{label}</span>
      <span className={mono ? "font-mono" : undefined}>{value}</span>
    </div>
  );
}

function applyTheme(theme: "system" | "light" | "dark") {
  const dark =
    theme === "dark" ||
    (theme === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.classList.toggle("dark", dark);
  localStorage.setItem("banden.theme", JSON.stringify(theme));
}
