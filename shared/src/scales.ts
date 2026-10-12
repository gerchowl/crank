/**
 * Time-ago axis scales shared by the history store (bucket edges) and the UI (x mapping),
 * so that buckets line up with the axis. u ∈ [0, 1], 0 = now, 1 = spanS ago.
 * A chart with "now at the right" draws at x = width * (1 - u).
 */
export type TimeScale = "log" | "fisheye";

/** Width (s) of the linear "focus" region of the fisheye scale. */
export function fisheyeFocusS(spanS: number): number {
  return Math.min(120, spanS / 4);
}

export function timeAgoToU(tAgo: number, spanS: number, scale: TimeScale): number {
  const a = Math.max(0, Math.min(tAgo, spanS));
  if (scale === "log") return Math.log1p(a) / Math.log1p(spanS);
  const f = fisheyeFocusS(spanS);
  if (a <= f) return (0.5 * a) / f;
  return 0.5 + (0.5 * Math.log(a / f)) / Math.log(spanS / f);
}

export function uToTimeAgo(u: number, spanS: number, scale: TimeScale): number {
  const x = Math.max(0, Math.min(1, u));
  if (scale === "log") return Math.expm1(x * Math.log1p(spanS));
  const f = fisheyeFocusS(spanS);
  if (x <= 0.5) return (x / 0.5) * f;
  return f * Math.pow(spanS / f, (x - 0.5) / 0.5);
}

/** n+1 ascending time-ago edges from 0 to spanS, uniform in u. */
export function timeAgoEdges(spanS: number, n: number, scale: TimeScale): number[] {
  const out: number[] = [];
  for (let k = 0; k <= n; k++) out.push(uToTimeAgo(k / n, spanS, scale));
  return out;
}

/** Suggested axis ticks (time-ago seconds) for a span. */
export function timeAgoTicks(spanS: number): number[] {
  const all = [0, 1, 5, 10, 30, 60, 120, 300, 600, 1200, 1800, 3600, 7200, 14400, 43200, 86400];
  return all.filter((t) => t <= spanS);
}
