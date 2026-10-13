import type { MetricId, PowerTarget, StepEnd, Zone } from "@neo/shared";
import { formatDuration } from "@neo/shared";

/** m:ss or h:mm:ss */
export function clock(totalS: number): string {
  const s = Math.max(0, Math.round(totalS));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const mm = h ? String(m).padStart(2, "0") : String(m);
  return `${h ? `${h}:` : ""}${mm}:${String(sec).padStart(2, "0")}`;
}

const TIME_METRICS: ReadonlySet<MetricId> = new Set(["elapsed", "remaining", "block_remaining"]);

export function formatValue(metric: MetricId, v: number | null): string {
  if (v === null || !Number.isFinite(v)) return "—";
  if (TIME_METRICS.has(metric)) return clock(v);
  switch (metric) {
    case "if":
    case "dfa_a1":
      return v.toFixed(2);
    case "w_per_kg":
    case "distance":
      return v.toFixed(1);
    case "speed":
    case "decoupling":
    case "hr_drift":
    case "hr_vs_typical":
      return (v > 0 && metric !== "speed" ? "+" : "") + v.toFixed(1);
    default:
      return Math.round(v).toString();
  }
}

/** 0-based zone index for a value (values above the last zone map to the last). */
export function zoneIdx(zones: readonly Zone[], v: number): number {
  for (let i = 0; i < zones.length; i++) {
    const z = zones[i];
    if (z && v < z.max) return i;
  }
  return zones.length - 1;
}

export function formatTarget(t: PowerTarget): string {
  switch (t.kind) {
    case "pct_ftp":
      return `${t.value}%`;
    case "watts":
      return `${t.value} W`;
    case "range":
      return `${t.lo}–${t.hi}%`;
    case "ramp":
      return `ramp ${t.from}→${t.to}%`;
    case "hr":
      return `${t.bpm} bpm`;
    case "free":
      return "free";
  }
}

export function formatEnd(e: StepEnd): string {
  switch (e.kind) {
    case "time":
      return formatDuration(e.s);
    case "kj":
      return `${e.kj} kJ`;
    case "lap":
      return "lap";
    case "hr_recovery":
      return `HR<${e.targetBpm} ${formatDuration(e.minS)}..${formatDuration(e.maxS)}`;
  }
}

export function fmtTimeAgo(s: number): string {
  if (s === 0) return "now";
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.round(s / 60)}m`;
  return `${Math.round(s / 360) / 10}h`;
}
