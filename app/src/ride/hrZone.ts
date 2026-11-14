// The heart-rate zone of a live HR value (DESIGN §18.3 "Heart-rate zone", #586). Two models, both from cited tables:
//
// - With an LTHR: Joe Friel's seven bike zones, as % of LTHR (J. Friel, "Joe Friel's Quick Guide to Setting Zones",
//   TrainingPeaks, 2012, from his 2009 TrainingBible blog; https://www.trainingpeaks.com/learn/articles/joe-friel-s-quick-guide-to-setting-zones/):
//   Z1 "Less than 81%", Z2 "81% to 89%", Z3 "90% to 93%", Z4 "94% to 99%", Z5a "100% to 102%", Z5b "103% to 106%",
//   Z5c "More than 106%".
// - Otherwise, with an HR max: five zones as % of HR max (Polar, "Heart Rate Zones",
//   https://www.polar.com/blog/running-heart-rate-zones-basics/): Z1 50–60 %, Z2 60–70 %, Z3 70–80 %, Z4 80–90 %,
//   Z5 90–100 %.
//
// Both tables are written in whole percents, so the percent is rounded to a whole number first (half up) and the
// table then applies as written: Friel's ranges leave no gap between them on whole percents. Polar's adjacent zones
// share their edge (60, 70, 80, 90); the edge belongs to the higher zone ("Z2 from 60 %"). Two choices of ours, each
// in DESIGN: below 50 % of HR max is still Z1 (Friel's Z1 has no floor either, and a blank would read as no data), and
// above 100 % is still Z5 (the HR max is an estimate the rider can exceed).
//
// This copy of the tables is canonical for now; they move to crank-domain before Review shows HR time in zone (#645).

/** A zone: its label ("Z5a") and the model it is read in. */
export interface HrZone {
  /** The zone's name as the tables write it: "Z1" … "Z5c". */
  label: string;
  /** The model it is read in: Friel's % of LTHR, or % of HR max. */
  model: "lthr" | "hrMax";
}

/** The athlete's HR references, bpm; `null` when not known. */
export interface HrRefs {
  /** Lactate-threshold heart rate. */
  lthrBpm: number | null;
  /** Maximum heart rate. */
  hrMaxBpm: number | null;
}

/** Friel's bike zones: each zone's lowest whole percent of LTHR, highest first. */
const FRIEL: readonly (readonly [number, string])[] = [
  [107, "Z5c"],
  [103, "Z5b"],
  [100, "Z5a"],
  [94, "Z4"],
  [90, "Z3"],
  [81, "Z2"],
];
/** The five zones of % HR max: each zone's lowest whole percent, highest first. */
const PCT_MAX: readonly (readonly [number, string])[] = [
  [90, "Z5"],
  [80, "Z4"],
  [70, "Z3"],
  [60, "Z2"],
];

const known = (v: number | null): v is number =>
  v !== null && Number.isFinite(v) && v > 0;

function pick(
  hr: number,
  ref: number,
  table: readonly (readonly [number, string])[],
): string {
  const pct = Math.round((100 * hr) / ref);
  return table.find(([lo]) => pct >= lo)?.[1] ?? "Z1";
}

/** Friel's bike zone of `hr` for this LTHR (both bpm). */
export const frielZone = (hr: number, lthrBpm: number): string =>
  pick(hr, lthrBpm, FRIEL);

/** The five-zone % HR max zone of `hr` for this HR max (both bpm). */
export const pctMaxZone = (hr: number, hrMaxBpm: number): string =>
  pick(hr, hrMaxBpm, PCT_MAX);

/**
 * The zone of a live HR: Friel's with an LTHR, else % of HR max, else none. No HR (`null`, lost, not finite) is no
 * zone too: the readout says why (§5.2 "HR lost").
 */
export function hrZone(hr: number | null, refs: HrRefs): HrZone | null {
  if (hr === null || !Number.isFinite(hr) || hr <= 0) return null;
  if (known(refs.lthrBpm))
    return { label: frielZone(hr, refs.lthrBpm), model: "lthr" };
  if (known(refs.hrMaxBpm))
    return { label: pctMaxZone(hr, refs.hrMaxBpm), model: "hrMax" };
  return null;
}

/** The readout's words: the zone, then the model it is read in ("Z2 · LTHR", "Z3 · HRmax"). */
export const zoneText = (z: HrZone): string =>
  `${z.label} · ${z.model === "lthr" ? "LTHR" : "HRmax"}`;
