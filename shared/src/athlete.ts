import type { MetricId, MetricInfo, Zone } from "./contract";

export interface Athlete {
  ftp: number;
  weightKg: number;
  maxHr: number;
  restHr: number;
}

export const DEFAULT_ATHLETE: Athlete = { ftp: 280, weightKg: 72, maxHr: 190, restHr: 50 };

/**
 * Physiological parameters of the simulated athlete. Derived from the athlete record; the
 * "typical" values are what the backend believes is normal for this athlete (used for
 * hr_vs_typical, the HR-recovery prior and compile-time estimates).
 */
export interface Physio {
  cp: number;            // critical power ≈ FTP
  wPrime: number;        // J
  /** steady-state HR at 0 W while pedalling */
  hrAtZero: number;
  /** steady-state HR at FTP */
  hrAtFtp: number;
  /** usual mono-exponential HR recovery time constant (s), fresh legs */
  usualTauS: number;
}

export function physioFor(a: Athlete): Physio {
  return {
    cp: a.ftp,
    wPrime: 20000,
    hrAtZero: a.restHr + 12,
    hrAtFtp: Math.round(a.maxHr * 0.9),
    usualTauS: 50,
  };
}

/** Steady-state HR for a sustained power, with a soft cap below max HR. */
export function hrSteady(powerW: number, a: Athlete, p: Physio = physioFor(a)): number {
  const raw = p.hrAtZero + (p.hrAtFtp - p.hrAtZero) * (Math.max(0, powerW) / a.ftp);
  const knee = a.maxHr - 8;
  if (raw <= knee) return raw;
  return knee + 8 * (1 - Math.exp(-(raw - knee) / 8));
}

/** Inverse of hrSteady (below the soft cap). */
export function powerForHr(bpm: number, a: Athlete, p: Physio = physioFor(a)): number {
  return Math.max(0, ((bpm - p.hrAtZero) / (p.hrAtFtp - p.hrAtZero)) * a.ftp);
}

// ---------- zones ----------

export const POWER_ZONE_DEFS: readonly [string, number, number, string][] = [
  ["Z1 Recovery", 0, 0.55, "#8a94a6"],
  ["Z2 Endurance", 0.55, 0.75, "#3b82f6"],
  ["Z3 Tempo", 0.75, 0.9, "#22c55e"],
  ["Z4 Threshold", 0.9, 1.05, "#eab308"],
  ["Z5 VO2max", 1.05, 1.2, "#f97316"],
  ["Z6 Anaerobic", 1.2, 1.5, "#ef4444"],
  ["Z7 Neuromuscular", 1.5, 8, "#a855f7"],
];

export const HR_ZONE_DEFS: readonly [string, number, number, string][] = [
  ["Z1", 0, 0.6, "#8a94a6"],
  ["Z2", 0.6, 0.7, "#3b82f6"],
  ["Z3", 0.7, 0.8, "#22c55e"],
  ["Z4", 0.8, 0.9, "#f97316"],
  ["Z5", 0.9, 1.0, "#ef4444"],
];

/** Power zones in watts (Coggan 7-zone). */
export function powerZones(ftp: number): Zone[] {
  return POWER_ZONE_DEFS.map(([name, lo, hi, color]) => ({ name, min: Math.round(lo * ftp), max: Math.round(hi * ftp), color }));
}

/** Power zones in % FTP. */
export function pctZones(): Zone[] {
  return POWER_ZONE_DEFS.map(([name, lo, hi, color]) => ({ name, min: lo * 100, max: hi * 100, color }));
}

/** HR zones in bpm (% of max HR). */
export function hrZones(maxHr: number): Zone[] {
  return HR_ZONE_DEFS.map(([name, lo, hi, color]) => ({ name, min: Math.round(lo * maxHr), max: Math.round(hi * maxHr), color }));
}

/** 0-based index of the zone containing v (values above the last zone map to the last). */
export function zoneIndex(zones: Zone[], v: number): number {
  for (let i = 0; i < zones.length; i++) {
    if (v < (zones[i] as Zone).max) return i;
  }
  return zones.length - 1;
}

function indexZones(defs: readonly [string, number, number, string][]): Zone[] {
  return defs.map(([name, , , color], i) => ({ name, min: i + 1, max: i + 2, color }));
}

// ---------- metric catalog ----------

export function metricCatalog(a: Athlete): MetricInfo[] {
  const pz = powerZones(a.ftp);
  const hz = hrZones(a.maxHr);
  const wbalZones: Zone[] = [
    { name: "empty", min: 0, max: 25, color: "#ef4444" },
    { name: "low", min: 25, max: 50, color: "#f97316" },
    { name: "ok", min: 50, max: 101, color: "#22c55e" },
  ];
  const m = (id: MetricId, label: string, unit: string, sourceHz: number, zones?: Zone[]): MetricInfo =>
    zones ? { id, label, unit, sourceHz, zones } : { id, label, unit, sourceHz };
  return [
    m("power", "Power", "W", 4, pz),
    m("power_3s", "Power 3s", "W", 4, pz),
    m("power_10s", "Power 10s", "W", 4, pz),
    m("power_30s", "Power 30s", "W", 4, pz),
    m("power_lap", "Lap power", "W", 4, pz),
    m("np", "Normalized power", "W", 1, pz),
    m("if", "Intensity factor", "", 1),
    m("tss", "TSS", "", 1),
    m("kj", "Work", "kJ", 4),
    m("w_per_kg", "W/kg", "W/kg", 4),
    m("pct_ftp", "% FTP", "%", 4, pctZones()),
    m("target_power", "Target", "W", 4, pz),
    m("delta_target", "Δ target", "W", 4),
    m("power_zone", "Power zone", "", 4, indexZones(POWER_ZONE_DEFS)),
    m("wbal_pct", "W'bal", "%", 4, wbalZones),
    m("hr", "Heart rate", "bpm", 1, hz),
    m("hr_zone", "HR zone", "", 1, indexZones(HR_ZONE_DEFS)),
    m("hr_pct_max", "% max HR", "%", 1, HR_ZONE_DEFS.map(([name, lo, hi, color]) => ({ name, min: lo * 100, max: hi * 100, color }))),
    m("hr_vs_typical", "HR vs typical", "bpm", 1),
    m("hr_drift", "HR drift", "bpm", 1),
    m("decoupling", "Pa:HR decoupling", "%", 1),
    m("rr", "RR interval", "ms", 2),
    m("rmssd", "RMSSD", "ms", 1),
    m("dfa_a1", "DFA α1", "", 1),
    m("cadence", "Cadence", "rpm", 4),
    m("target_cadence", "Target cadence", "rpm", 4),
    m("speed", "Speed", "km/h", 4),
    m("distance", "Distance", "km", 4),
    m("elapsed", "Elapsed", "s", 4),
    m("remaining", "Remaining", "s", 1),
    m("block_remaining", "Block remaining", "s", 4),
    m("block_index", "Block #", "", 4),
    m("compliance", "Compliance", "%", 1),
  ];
}

export const ALL_METRICS: readonly MetricId[] = metricCatalog(DEFAULT_ATHLETE).map((m) => m.id);
