import type { Filter, MetricId, Widget, Zone } from "@neo/shared";
import { metricInfo } from "../backend";
import type { ThemeTokens } from "../theme/theme";

/** Fixed series identity (VIEWS §0): never cycled, never by rank. */
export function seriesColor(t: ThemeTokens, metric: MetricId): string {
  if (metric.startsWith("power") || metric === "np" || metric === "w_per_kg" || metric === "pct_ftp") return t.power;
  if (metric === "hr" || metric === "hr_pct_max" || metric === "hr_zone") return t.hr;
  if (metric === "cadence") return t.cadence;
  if (metric === "dfa_a1") return t.dfa;
  return t.ink2;
}

export function zonesOf(metric: MetricId): Zone[] | undefined {
  return metricInfo(metric).zones;
}

/** Zone names are "Z4 Threshold" for power; return the short label. */
export function zoneLabel(z: Zone | undefined, idx: number): string {
  if (!z) return `Z${idx + 1}`;
  const m = /^Z\d/.exec(z.name);
  return m ? m[0] : z.name;
}

export const FILTER_PRESETS: { name: string; filter: Filter[] }[] = [
  { name: "raw", filter: [] },
  { name: "EMA 2s", filter: [{ kind: "ema", tauS: 2 }] },
  { name: "EMA 5s", filter: [{ kind: "ema", tauS: 5 }] },
  { name: "SMA 10s", filter: [{ kind: "sma", windowS: 10 }] },
];

export function filterLabel(f: Filter[] | undefined): string {
  if (!f || f.length === 0) return "raw";
  return f
    .map((x) => {
      switch (x.kind) {
        case "ema":
          return `EMA ${x.tauS}s`;
        case "sma":
          return `SMA ${x.windowS}s`;
        case "median":
          return `median ${x.n}`;
        case "one_euro":
          return `1€ ${x.minCutoff}/${x.beta}`;
        case "hold":
          return "hold";
      }
    })
    .join(" → ");
}

export interface WidgetProps {
  widget: Widget;
  editing: boolean;
}
