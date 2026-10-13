import { timeAgoToU, type Bucket } from "@neo/shared";

export type Scale = "log" | "fisheye";

/** Buckets (oldest → newest) → x = 1 − u (now at the right) with explicit gaps. */
export function bucketsToTable(bs: readonly Bucket[], spanS: number, scale: Scale): [number[], (number | null)[], (number | null)[], (number | null)[]] {
  const xs: number[] = [];
  const mean: (number | null)[] = [];
  const min: (number | null)[] = [];
  const max: (number | null)[] = [];
  let prev: Bucket | undefined;
  for (const b of bs) {
    if (prev && prev.tAgo - (b.tAgo + b.dur) > 1e-6) {
      xs.push(1 - timeAgoToU(prev.tAgo - 1e-3, spanS, scale));
      mean.push(null);
      min.push(null);
      max.push(null);
    }
    xs.push(1 - timeAgoToU(b.tAgo + b.dur / 2, spanS, scale));
    mean.push(b.mean);
    min.push(b.min);
    max.push(b.max);
    prev = b;
  }
  return [xs, mean, min, max];
}

