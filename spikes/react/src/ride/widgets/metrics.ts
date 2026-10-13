import { zoneIndex, type MetricId, type MetricInfo } from "@neo/shared";
import { backend } from "../../backend";
import type { ChartTheme } from "../../theme/theme";

const byId = new Map(backend.metrics.map((m) => [m.id, m]));
export function info(id: MetricId): MetricInfo {
  return byId.get(id) ?? { id, label: id, unit: "", sourceHz: 1 };
}

/** Fixed series identity (VIEWS §0): power → slot 1, HR → 2, cadence → 3, DFA-α1 → 4, torque → 5. */
export function seriesColor(t: ChartTheme, id: MetricId): string {
  if (id.startsWith("power") || id === "np" || id === "w_per_kg" || id === "pct_ftp" || id === "kj") return t.series.power;
  if (id.startsWith("hr") || id === "rr" || id === "rmssd") return t.series.hr;
  if (id.includes("cadence")) return t.series.cadence;
  if (id === "dfa_a1") return t.series.dfa;
  if (id === "target_power") return t.target;
  return t.ink2;
}

export interface ZoneHit { idx: number; count: number; name: string }
export function zoneOf(id: MetricId, v: number | null): ZoneHit | null {
  const zs = info(id).zones;
  if (!zs || v === null) return null;
  const idx = zoneIndex(zs, v);
  const z = zs[idx];
  return z ? { idx, count: zs.length, name: z.name } : null;
}

