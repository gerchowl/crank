export function mmss(totalS: number | null | undefined): string {
  if (totalS === null || totalS === undefined || !Number.isFinite(totalS)) return "—";
  const s = Math.max(0, Math.round(totalS));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

export function fmtDur(s: number): string {
  if (s >= 3600) return `${Math.floor(s / 3600)}h${s % 3600 ? `${Math.round((s % 3600) / 60)}m` : ""}`;
  if (s >= 60) return s % 60 ? `${Math.floor(s / 60)}m${Math.round(s % 60)}s` : `${s / 60}m`;
  return `${Math.round(s)}s`;
}

export function fmtTimeAgo(s: number): string {
  if (s === 0) return "now";
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${s / 60}m`;
  return `${s / 3600}h`;
}

/** Format a metric value with sensible precision for its unit. */
export function fmtValue(v: number | null, unit: string, id: string): string {
  if (v === null || !Number.isFinite(v)) return "—";
  if (unit === "s") return mmss(v);
  if (id === "if" || id === "dfa_a1") return v.toFixed(2);
  if (unit === "W/kg") return v.toFixed(1);
  if (unit === "km" || unit === "km/h") return v.toFixed(1);
  if (id === "decoupling" || id === "hr_drift" || id === "hr_vs_typical") return (v > 0 ? "+" : "") + v.toFixed(1);
  return Math.round(v).toString();
}
