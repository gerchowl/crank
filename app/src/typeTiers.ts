// Ride-screen type tiers in physical units (DESIGN §6.2, #91). Pure maths: no DOM, no React.
// Character height h at distance d subtends θ ≈ h/d; font px = θ · d · (px per mm) / 0.72 (cap height ≈ 0.72 em).
// "pt/mm" in §6.2 is CSS px per millimetre at default scaling (1 pt = 1 CSS px there), which is what this uses.

/** Rider distance when Settings → Ride screen has none (DESIGN §6.2). */
export const DEFAULT_DISTANCE_M = 1.0;

/** Minimum visual angle per tier in arc minutes (DESIGN §6.2). */
export const TIER_ARCMIN = { t1: 90, t2: 50, t3p: 20, t3: 16 } as const;

/** T1r (recovery band as hero) is 0.62 × T1 (DESIGN §16.1, which supersedes the §6.2 55′ figure). */
export const T1R_OVER_T1 = 0.62;

/** T4 chrome is fixed px, never needed mid-effort (DESIGN §6.2: 15–18 px). */
export const T4_PX = 16;

const CAP_HEIGHT_EM = 0.72;
const ARCMIN_RAD = Math.PI / (180 * 60);

/**
 * The window can host T2 only while T2 stays within the share of the window height the ride layout budgets for
 * it: the prototype's `clamp(84px, 11.4vh, 128px)` (DESIGN §6.2, app.css `--t2`), which fits both reference
 * devices (101 of 900 px, 84 of 768 px).
 */
export const T2_MAX_HEIGHT_SHARE = 0.114;

/** Prototype `clamp(…vh, …)` rules (DESIGN §6.2 / §16.1): the fallback when the physical size is unknown. */
export const FALLBACK_TOKENS = {
  "--t1": "clamp(150px, min(24.5vh, 17.2vw), 260px)",
  "--t1r": "calc(var(--t1) * 0.62)",
  "--t2": "clamp(84px, 11.4vh, 128px)",
  "--t3p": "clamp(34px, 4.5vh, 44px)",
  "--t3": "clamp(27px, 3.7vh, 36px)",
  "--t4": `${T4_PX}px`,
} as const;

export type TierToken = keyof typeof FALLBACK_TOKENS;

/** The monitor as the shell can report it. Widths are device pixels; `widthMm` is null when unknown. */
export interface Monitor {
  widthPx: number;
  scaleFactor: number;
  widthMm: number | null;
}

/** Inputs of {@link computeTiers}. */
export interface TierInput {
  monitor: Monitor;
  /** App-setting override of the monitor's physical width in mm; wins over the measured value. */
  widthMmOverride: number | null;
  distanceM: number;
  /** The window's inner size in CSS px. */
  window: { width: number; height: number };
}

/** Physical sizes known: whole CSS px per tier, plus the too-small verdict. */
export interface PhysicalTiers {
  kind: "physical";
  px: {
    t1: number;
    t1r: number;
    t2: number;
    t3p: number;
    t3: number;
    t4: number;
  };
  tokens: Record<TierToken, string>;
  tooSmallForT2: boolean;
  notice: null;
}

/** Physical size unknown: the prototype clamp rules, which cannot tell whether T2 fits. */
export interface FallbackTiers {
  kind: "fallback";
  px: null;
  tokens: Record<TierToken, string>;
  tooSmallForT2: false;
  notice: string;
}

export type Tiers = PhysicalTiers | FallbackTiers;

export const FALLBACK_NOTICE =
  "Screen size unknown: text sizes are approximated from the window, not measured for your distance.";

const positive = (n: number | null): n is number =>
  n !== null && Number.isFinite(n) && n > 0;

/** CSS px per mm, or null when the physical width is unknown or any input is not a positive number. */
export function pxPerMm(
  monitor: Monitor,
  widthMmOverride: number | null,
): number | null {
  const mm = positive(widthMmOverride) ? widthMmOverride : monitor.widthMm;
  if (
    !positive(mm) ||
    !positive(monitor.widthPx) ||
    !positive(monitor.scaleFactor)
  )
    return null;
  return monitor.widthPx / monitor.scaleFactor / mm;
}

/** Font px for a tier of `arcmin` at `distanceM` on a display of `pxPerMm`. */
export const tierPx = (
  arcmin: number,
  distanceM: number,
  perMm: number,
): number => (arcmin * ARCMIN_RAD * distanceM * 1000 * perMm) / CAP_HEIGHT_EM;

/** Computes the ride-screen tiers (DESIGN §6.2). Unknown physical size or distance degrades to the fallback. */
export function computeTiers(input: TierInput): Tiers {
  const perMm = pxPerMm(input.monitor, input.widthMmOverride);
  if (perMm === null || !positive(input.distanceM)) {
    return {
      kind: "fallback",
      px: null,
      tokens: { ...FALLBACK_TOKENS },
      tooSmallForT2: false,
      notice: FALLBACK_NOTICE,
    };
  }
  const at = (a: number) => tierPx(a, input.distanceM, perMm);
  const t1 = Math.round(at(TIER_ARCMIN.t1));
  const px = {
    t1,
    t1r: Math.round(t1 * T1R_OVER_T1),
    t2: Math.round(at(TIER_ARCMIN.t2)),
    t3p: Math.round(at(TIER_ARCMIN.t3p)),
    t3: Math.round(at(TIER_ARCMIN.t3)),
    t4: T4_PX,
  };
  return {
    kind: "physical",
    px,
    tokens: {
      "--t1": `${px.t1}px`,
      "--t1r": `${px.t1r}px`,
      "--t2": `${px.t2}px`,
      "--t3p": `${px.t3p}px`,
      "--t3": `${px.t3}px`,
      "--t4": `${px.t4}px`,
    },
    tooSmallForT2: px.t2 > T2_MAX_HEIGHT_SHARE * input.window.height,
    notice: null,
  };
}
