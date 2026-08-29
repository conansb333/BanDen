/**
 * Dependency-free range slider for the app. Uses the native control with
 * accent styling so the track and thumb are always visible.
 */
import { cn } from "@/lib/utils";

export function Slider({
  id,
  min,
  max,
  step = 1,
  value,
  onValueChange,
  className,
  disabled,
}: {
  id?: string;
  min: number;
  max: number;
  step?: number;
  value: number[];
  onValueChange: (value: number[]) => void;
  className?: string;
  disabled?: boolean;
}) {
  return (
    <input
      id={id}
      type="range"
      min={min}
      max={max}
      step={step}
      value={value[0] ?? min}
      disabled={disabled}
      onChange={(e) => onValueChange([Number(e.target.value)])}
      className={cn("h-2 w-full cursor-pointer accent-primary", className)}
    />
  );
}
