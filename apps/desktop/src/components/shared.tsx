/** Small shared presentational components. */
import type { ReactNode } from "react";
import {
  Box,
  Camera,
  Cpu,
  Gamepad2,
  HelpCircle,
  Info,
  Laptop,
  Monitor,
  Network,
  Printer,
  Router,
  Server,
  ShieldCheck,
  SlidersHorizontal,
  Smartphone,
  Tv,
  XCircle,
  AlertTriangle,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { DeviceStatus, EventCategory, SessionState } from "@/types";

/** Canonical device kinds the UI knows how to iconify. */
export const DEVICE_KINDS = [
  { value: "smartphone", label: "Phone" },
  { value: "laptop", label: "Laptop" },
  { value: "desktop", label: "Computer" },
  { value: "router", label: "Router" },
  { value: "server", label: "Server" },
  { value: "nas", label: "NAS" },
  { value: "camera", label: "Camera" },
  { value: "console", label: "Game console" },
  { value: "media", label: "TV / media" },
  { value: "printer", label: "Printer" },
  { value: "virtual machine", label: "Virtual machine" },
  { value: "single-board computer", label: "Single-board" },
] as const;

/** Map any kind-ish string (auto or manual) to a lucide icon. */
export function DeviceKindIcon({
  kind,
  className,
}: {
  kind: string | null | undefined;
  className?: string;
}) {
  const k = (kind ?? "").toLowerCase();
  const cls = cn("h-4 w-4 shrink-0 text-muted-foreground", className);
  if (/phone|android|iphone|mobile/.test(k)) return <Smartphone className={cls} />;
  if (/laptop|notebook|macbook/.test(k)) return <Laptop className={cls} />;
  if (/desktop|computer|pc|workstation/.test(k)) return <Monitor className={cls} />;
  if (/router|gateway|access.point|ap\b/.test(k)) return <Router className={cls} />;
  if (/server/.test(k)) return <Server className={cls} />;
  if (/nas|storage|diskstation/.test(k)) return <Server className={cls} />;
  if (/camera/.test(k)) return <Camera className={cls} />;
  if (/console|playstation|playstation|xbox|nintendo|switch/.test(k))
    return <Gamepad2 className={cls} />;
  if (/media|tv|cast|roku|fire.?tv/.test(k)) return <Tv className={cls} />;
  if (/printer|epson/.test(k)) return <Printer className={cls} />;
  if (/virtual|vm/.test(k)) return <Box className={cls} />;
  if (/single.board|raspberry|iot|sbc/.test(k)) return <Cpu className={cls} />;
  return <HelpCircle className={cls} />;
}

/** Colored icon for an activity-event category. */
export function EventCategoryIcon({ category }: { category: EventCategory }) {
  const map: Record<
    EventCategory,
    { icon: ReactNode; cls: string }
  > = {
    INFO: { icon: <Info className="h-4 w-4" />, cls: "bg-sky-500/15 text-sky-400" },
    WARNING: {
      icon: <AlertTriangle className="h-4 w-4" />,
      cls: "bg-amber-500/15 text-amber-400",
    },
    ERROR: { icon: <XCircle className="h-4 w-4" />, cls: "bg-red-500/15 text-red-400" },
    RECOVERY: {
      icon: <ShieldCheck className="h-4 w-4" />,
      cls: "bg-emerald-500/15 text-emerald-400",
    },
    NETWORK: { icon: <Network className="h-4 w-4" />, cls: "bg-cyan-500/15 text-cyan-400" },
    SESSION: {
      icon: <SlidersHorizontal className="h-4 w-4" />,
      cls: "bg-violet-500/15 text-violet-400",
    },
  };
  const { icon, cls } = map[category] ?? map.INFO;
  return (
    <span className={cn("grid h-7 w-7 shrink-0 place-items-center rounded-full", cls)}>
      {icon}
    </span>
  );
}

export function DeviceStatusBadge({ status }: { status: DeviceStatus }) {
  const map: Record<DeviceStatus, { variant: "success" | "muted" | "warning" | "secondary"; label: string }> = {
    online: { variant: "success", label: "ONLINE" },
    new: { variant: "warning", label: "NEW" },
    offline: { variant: "muted", label: "OFFLINE" },
    unknown: { variant: "secondary", label: "UNKNOWN" },
  };
  const { variant, label } = map[status] ?? map.unknown;
  return <Badge variant={variant}>{label}</Badge>;
}

export function SessionStateBadge({ state }: { state: SessionState }) {
  const map: Record<SessionState, "success" | "warning" | "destructive" | "muted" | "secondary" | "default"> = {
    active: "success",
    preparing: "secondary",
    stopping: "warning",
    restoring: "warning",
    verifying: "warning",
    completed: "muted",
    recovery_required: "destructive",
    failed: "destructive",
  };
  return (
    <Badge variant={map[state] ?? "secondary"} className="uppercase">
      {state.replace(/_/g, " ")}
    </Badge>
  );
}

export function StatCard({
  title,
  value,
  sub,
  icon,
  loading,
  tone = "default",
}: {
  title: string;
  value: ReactNode;
  sub?: ReactNode;
  icon?: ReactNode;
  loading?: boolean;
  tone?: "default" | "success" | "warning" | "destructive" | "info" | "violet";
}) {
  const toneCls: Record<string, string> = {
    default: "bg-muted text-muted-foreground",
    success: "bg-emerald-500/15 text-emerald-400",
    warning: "bg-amber-500/15 text-amber-400",
    destructive: "bg-red-500/15 text-red-400",
    info: "bg-sky-500/15 text-sky-400",
    violet: "bg-violet-500/15 text-violet-400",
  };
  const valueCls: Record<string, string> = {
    default: "",
    success: "text-emerald-400",
    warning: "text-amber-400",
    destructive: "text-red-400",
    info: "text-sky-400",
    violet: "text-violet-400",
  };
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="text-sm font-medium text-muted-foreground">{title}</CardTitle>
        {icon && (
          <span className={cn("grid h-8 w-8 place-items-center rounded-md", toneCls[tone])}>
            {icon}
          </span>
        )}
      </CardHeader>
      <CardContent>
        {loading ? (
          <Skeleton className="h-8 w-24" />
        ) : (
          <div className={cn("text-2xl font-semibold tabular-nums", valueCls[tone])}>
            {value}
          </div>
        )}
        {sub && <div className="mt-1 text-xs text-muted-foreground">{sub}</div>}
      </CardContent>
    </Card>
  );
}

export function EmptyState({
  icon,
  title,
  description,
  action,
  className,
}: {
  icon?: ReactNode;
  title: string;
  description?: string;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-2 rounded-lg border border-dashed px-6 py-12 text-center",
        className
      )}
    >
      {icon && <div className="text-muted-foreground [&_svg]:h-8 [&_svg]:w-8">{icon}</div>}
      <div className="text-sm font-medium">{title}</div>
      {description && <div className="max-w-sm text-xs text-muted-foreground">{description}</div>}
      {action && <div className="mt-2">{action}</div>}
    </div>
  );
}
