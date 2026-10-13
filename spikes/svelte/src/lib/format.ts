import { zoneIndex, type MetricId, type MetricInfo, type PowerTarget, type StepEnd, type Zone } from "@neo/shared";
import { backend } from "./backend";

export const metricInfo = new Map<MetricId, MetricInfo>(backend.metrics.map((m) => [m.id, m]));

export function label(m: MetricId): string {
  return metricInfo.get(m)?.label ?? m;
}

export function unit(m: MetricId): string {
  return metricInfo.get(m)?.unit ?? "";
}

const TIME_METRICS = new Set<MetricId>(["elapsed", "remaining", "block_remaining"]);

export function isTimeMetric(m: MetricId): boolean {
  return TIME_METRICS.has(m);
}

/** m:ss or h:mm:ss; negative values get a sign. */
export function clock(s: number): string {
  const sign = s < 0 ? "−" : "";
  const a = Math.round(Math.abs(s));
  const h = Math.floor(a / 3600);
  const m = Math.floor((a % 3600) / 60);
  const sec = a % 60;
  const ss = String(sec).padStart(2, "0");
  return h > 0 ? `${sign}${h}:${String(m).padStart(2, "0")}:${ss}` : `${sign}${m}:${ss}`;
}

/** Compact duration for axis labels and the editor: 30s, 4m, 1m30s, 1h5m. */
export function dur(s: number): string {
  const a = Math.round(s);
  if (a < 60) return `${a}s`;
  const h = Math.floor(a / 3600);
  const m = Math.floor((a % 3600) / 60);
  const sec = a % 60;
  return `${h ? `${h}h` : ""}${m ? `${m}m` : ""}${sec ? `${sec}s` : ""}`;
}

export function fmtValue(m: MetricId, v: number | null): string {
  if (v === null || !Number.isFinite(v)) return "—";
  if (isTimeMetric(m)) return clock(v);
  switch (m) {
    case "if":
    case "w_per_kg":
    case "dfa_a1":
      return v.toFixed(2);
    case "distance":
    case "decoupling":
    case "hr_drift":
    case "speed":
      return v.toFixed(1);
    default:
      return Math.round(v).toString();
  }
}

export function zonesFor(m: MetricId): Zone[] | undefined {
  return metricInfo.get(m)?.zones;
}

/** 0-based zone index and zone count, or null if the metric has no zones. */
export function zoneOf(m: MetricId, v: number | null): { index: number; n: number; name: string } | null {
  const zs = zonesFor(m);
  if (!zs || v === null) return null;
  const i = zoneIndex(zs, v);
  return { index: i, n: zs.length, name: zs[i]?.name.split(" ")[0] ?? "" };
}

export function targetText(t: PowerTarget): string {
  switch (t.kind) {
    case "pct_ftp":
      return `${t.value}%`;
    case "watts":
      return `${t.value} W`;
    case "range":
      return `${t.lo}–${t.hi}%`;
    case "ramp":
      return `${t.from}→${t.to}%`;
    case "hr":
      return `${t.bpm} bpm`;
    case "free":
      return "free";
  }
}

export function endText(e: StepEnd): string {
  switch (e.kind) {
    case "time":
      return dur(e.s);
    case "kj":
      return `${e.kj} kJ`;
    case "lap":
      return "lap";
    case "hr_recovery":
      return `HR<${e.targetBpm} ${dur(e.minS)}–${dur(e.maxS)}`;
  }
}

export const PHASE_LABEL: Record<string, string> = {
  warmup: "Warm-up",
  work: "Work",
  recovery: "Recovery",
  cooldown: "Cool-down",
  free: "Free",
};
