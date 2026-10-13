// F · Spin-up (#83, BRAND.md §F · Spin-up): the app mark spins up on its crank axis until the sweep closes the
// ring, then the ring glides into place and opens into the wordmark's C while R A N K and the speed lines
// emerge — landing on A · Slice exactly. Every element is a pure function of time (`frame`), painted by
// `paint`; the page drives t from the clock (src/main.ts), the screenshots set it (?t=). No CSS animation, no
// raster blur: the motion blur is stepped arc sectors whose sweep grows with angular speed. The markup at rest
// IS the settled wordmark (A's paths, nothing else visible), so reduced motion = no script = the final frame.
import { H, n } from "./geometry.ts";
import { SLICE_ART } from "./slice.ts";
import { SPIN_ART } from "./spin-art.ts";
import type { Art, Paint } from "./wordmark.ts";

const DEG = Math.PI / 180;

/** Timing (ms from the first frame) and physics. BRAND.md §F · Spin-up explains every number. */
export const SPIN = {
  /** Constant angular acceleration from rest until `up`: ω reaches ω_max and the sweep closes the ring. */
  up: 450,
  /** Trail persistence: an arm's trail spans ω·τ behind it (τ > one 60 Hz frame, so no wagon-wheel strobe). */
  tau: 60,
  /** Trail sweep at ω_max (°): two arms 180° apart close the ring; the 6° overlap leaves no seam. */
  sweep: 186,
  /** Trail steps per arm (nested sectors) and ghost copies of the crank. */
  steps: 4,
  ghosts: [0.5, 0.3, 0.15],
  /** Ring glides into the C's place and opens into it (`glide` curve). */
  morph: [450, 830] as const,
  /** C first, then R A N K out of it, left → right: R once the ring has cleared it, the rest 40 ms apart. */
  letter: 300,
  stagger: 40,
  /** Speed lines drawn out right → left behind the word, as in A (start, duration). */
  lines: [
    [830, 320],
    [860, 320],
  ] as const,
  /** The two cuts open through the C: the uncut C fades off the cut one (start, duration). */
  cuts: [830, 150] as const,
  /** Everything at rest: the settled wordmark. ≤ the 1.2 s launch budget. */
  end: 1200,
  /** Boot ready within this → cut straight to the settled wordmark (hand-off, main.ts). */
  cut: 150,
  /** The mark's size at the start: ring outer diameter = 1.5 H, centred on the letters. */
  scale: (1.5 * H) / 825,
};

/** ω_max in °/ms, and the constant angular acceleration α that reaches it at `up`. */
const W_MAX = SPIN.sweep / SPIN.tau;
const ALPHA = W_MAX / SPIN.up;
/** The ring band of the mark (logo.svg: radius 350, stroke 125) and its gap seam overlap (°). */
const R_IN = 287.5;
const R_OUT = 412.5;
const OVERLAP = 6;
/** Hub at the start: the optical centre of the letters, mid cap height. */
const HUB = [SLICE_ART.focus, -H / 2] as const;

/** CSS cubic-bezier(x1, y1, x2, y2) as a function of progress 0…1. */
function bezier(x1: number, y1: number, x2: number, y2: number) {
  const b = (a1: number, a2: number, s: number) => 3 * a1 * s * (1 - s) ** 2 + 3 * a2 * s * s * (1 - s) + s ** 3;
  return (x: number) => {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    let lo = 0;
    let hi = 1;
    for (let i = 0; i < 32; i++) {
      const m = (lo + hi) / 2;
      if (b(x1, x2, m) < x) lo = m;
      else hi = m;
    }
    return b(y1, y2, (lo + hi) / 2);
  };
}
/** `settle` is A's curve (BRAND.md §Motion); `glide` eases the ring out of rest and lands it without overshoot. */
export const settle = bezier(0.16, 1, 0.3, 1);
export const glide = bezier(0.3, 0, 0, 1);
const clamp = (x: number) => Math.min(1, Math.max(0, x));
const progress = (t: number, start: number, dur: number) => clamp((t - start) / dur);

/** Crank angle (°, clockwise = forward pedalling seen from the drive side) and speed ω/ω_max. */
export function spinAt(t: number) {
  const tc = Math.min(t, SPIN.up);
  const theta = 0.5 * ALPHA * tc * tc + W_MAX * Math.max(0, t - SPIN.up);
  return { theta, speed: tc / SPIN.up };
}

// --- the morph: the ring (as placed at the start) and the C, matched run by run --------------------------
const runs = SPIN_ART.c;
const ring: [number, number][] = [];
{
  const [cx, cy] = HUB;
  const s = SPIN.scale;
  const a0 = -OVERLAP / 2;
  const a1 = 360 + OVERLAP / 2;
  const at = (deg: number, r: number): [number, number] => [cx + s * r * Math.cos(deg * DEG), cy + s * r * Math.sin(deg * DEG)];
  const fill = (count: number, f: (k: number) => [number, number]) => {
    for (let k = 0; k < count; k++) ring.push(f(k / count));
  };
  // Same order as the C's runs: outer arc, top face, inner arc (back), bottom face. Increasing angle is
  // clockwise on screen (y down): bottom-right → bottom → left → top → top-right.
  fill(runs[0].length, (p) => at(a0 + (a1 - a0) * p, R_OUT));
  fill(runs[1].length, (p) => at(a1, R_OUT + (R_IN - R_OUT) * p));
  fill(runs[2].length, (p) => at(a1 + (a0 - a1) * p, R_IN));
  fill(runs[3].length, (p) => at(a0, R_IN + (R_OUT - R_IN) * p));
}
const cPts = runs.flat();

function morphPath(u: number) {
  let d = "";
  for (let i = 0; i < ring.length; i++) {
    const [x0, y0] = ring[i]!;
    const [x1, y1] = cPts[i]!;
    d += `${i ? "L" : "M"}${n(x0 + (x1 - x0) * u)} ${n(y0 + (y1 - y0) * u)}`;
  }
  return `${d}Z`;
}

/** When each of R A N K starts: never before the gliding shape's right edge has passed the letter's left ink
 *  edge, and each ≥ `stagger` after the letter on its left — so they come out of the landed C in reading order. */
export const LETTER_START = (() => {
  const [m0, m1] = SPIN.morph;
  const right = (u: number) => Math.max(...ring.map(([x0], i) => x0 + (cPts[i]![0] - x0) * u));
  const starts = SPIN_ART.lefts.map((cx): number => {
    for (let t: number = m0; t <= m1; t += 5) if (right(glide(progress(t, m0, m1 - m0))) <= cx) return t;
    return m1;
  });
  for (let i = 1; i < starts.length; i++) starts[i] = Math.max(starts[i]!, starts[i - 1]! + SPIN.stagger);
  return starts;
})();

export type Frame = {
  t: number;
  /** The spinning mark (until the ring is closed), else null. */
  mark: { theta: number; sweep: number; trail: number; crank: number; ghosts: { lag: number; opacity: number }[] } | null;
  /** The ring → C shape (from the closed ring until the cuts are open), else null. */
  morph: { d: string; u: number; opacity: number } | null;
  /** A's cut C is shown (under the morph shape once that has landed). */
  cutC: boolean;
  /** R A N K: x offset (units) and opacity. */
  letters: { dx: number; opacity: number }[];
  /** Speed lines: left clip inset in %, 100 = hidden, 0 = drawn. */
  lines: number[];
  settled: boolean;
};

/** The whole sequence at time t (ms). Pure: same t, same frame. */
export function frame(t: number): Frame {
  const [m0, m1] = SPIN.morph;
  const { theta, speed } = spinAt(t);
  let mark: Frame["mark"] = null;
  if (t < m0) {
    const sweep = Math.min(SPIN.sweep, W_MAX * speed * SPIN.tau);
    // Arms dissolve into their own blur from half speed on; the trail saturates to a solid ring at ω_max.
    const crank = 1 - clamp((speed - 0.45) / 0.55) ** 2;
    const ghostSpan = Math.min(sweep, 90);
    mark = {
      theta,
      sweep,
      trail: 0.3 + 0.7 * speed ** 2,
      crank,
      ghosts: SPIN.ghosts.map((o, k) => ({ lag: (ghostSpan * (k + 1)) / SPIN.ghosts.length, opacity: o * crank * speed })),
    };
  }
  const [c0, cd] = SPIN.cuts;
  const u = glide(progress(t, m0, m1 - m0));
  const morph = t >= m0 && t < c0 + cd ? { d: morphPath(u), u, opacity: 1 - progress(t, c0, cd) } : null;
  return {
    t,
    mark,
    morph,
    cutC: t >= m1,
    letters: LETTER_START.map((s) => {
      const e = settle(progress(t, s, SPIN.letter));
      return { dx: -60 * (1 - e), opacity: t < s ? 0 : e };
    }),
    lines: SPIN.lines.map(([s, d]) => 100 * (1 - settle(progress(t, s, d)))),
    settled: t >= SPIN.end,
  };
}

// --- markup + painting -----------------------------------------------------------------------------------

/** Annular sector from angle a0 to a1 (°, a0 < a1) between radii r0 < r1, mark units. */
function sector(a0: number, a1: number, r0: number, r1: number) {
  const p = (a: number, r: number) => `${n(r * Math.cos(a * DEG))} ${n(r * Math.sin(a * DEG))}`;
  const large = a1 - a0 > 180 ? 1 : 0;
  return `M${p(a0, r1)}A${r1} ${r1} 0 ${large} 1 ${p(a1, r1)}L${p(a1, r0)}A${r0} ${r0} 0 ${large} 0 ${p(a0, r0)}Z`;
}

/** F's start-screen markup. At rest it paints exactly A · Slice (same paths, same order); the motion layers
 *  are hidden until `paint` shows them. */
export function spin(paint: Paint): Art {
  const { letters, bars, box, focus } = SLICE_ART;
  const trails = Array.from({ length: 2 * SPIN.steps }, () => `<path class="sp-trail"/>`).join("");
  const ghosts = SPIN.ghosts.map(() => `<path class="sp-ghost" d="${SPIN_ART.ghost}"/>`).join("");
  return {
    viewBox: box,
    focus,
    body:
      `<g fill="${paint.accent}">${bars.map((d) => `<path class="sp-line" d="${d}"/>`).join("")}</g>` +
      `<g fill="${paint.ink}">${letters.map((d, i) => `<path class="${i ? "sp-l" : "sp-c"}" d="${d}"/>`).join("")}</g>` +
      `<path class="sp-morph" display="none"/>` +
      `<g class="sp-mark" display="none" fill="${paint.accent}" transform="translate(${n(HUB[0])} ${HUB[1]}) scale(${n(SPIN.scale)})">` +
      `<path d="M164.32 -309.03A350 350 0 1 0 164.32 309.03" fill="none" stroke="${paint.accent}" stroke-width="125"/>` +
      `${trails}${ghosts}<path class="sp-crank" d="${SPIN_ART.crank}"/></g>`,
  };
}

/** Writes frame f into F's markup. At `settled` the visible DOM is back at rest: the motion layers hidden,
 *  no transform, opacity or clip left on A's paths. */
export function paint(svg: SVGSVGElement, f: Frame, colours: Paint) {
  const q = <E extends SVGElement>(sel: string) => [...svg.querySelectorAll<E>(sel)];
  const show = (el: Element, on: boolean) => (on ? el.removeAttribute("display") : el.setAttribute("display", "none"));
  const opacity = (el: Element, o: number) => (o >= 1 ? el.removeAttribute("opacity") : el.setAttribute("opacity", `${n(o)}`));

  const [mark] = q(".sp-mark");
  show(mark!, f.mark !== null);
  if (f.mark) {
    const { theta, sweep, trail, crank, ghosts } = f.mark;
    q(".sp-trail").forEach((el, i) => {
      // Arm i % 2 (upper at −50°, lower at 130°), step k: nested sectors, the nearest lag covered most.
      const arm = -50 + 180 * (i % 2) + theta;
      const k = Math.floor(i / 2);
      const lag = (sweep * (k + 1)) / SPIN.steps;
      show(el, sweep > 0);
      if (sweep > 0) el.setAttribute("d", sector(arm - lag, arm, R_IN, R_OUT));
      opacity(el, trail);
    });
    q(".sp-ghost").forEach((el, k) => {
      el.setAttribute("transform", `rotate(${n(theta - ghosts[k]!.lag)})`);
      opacity(el, ghosts[k]!.opacity);
    });
    const [c] = q(".sp-crank");
    c!.setAttribute("transform", `rotate(${n(theta)})`);
    opacity(c!, crank);
  }

  const [morph] = q<SVGPathElement>(".sp-morph");
  show(morph!, f.morph !== null);
  if (f.morph) {
    morph!.setAttribute("d", f.morph.d);
    // Brass ring → ink letter, mixed in Oklab along the same curve as the shape.
    morph!.style.fill = f.morph.u < 1 ? `color-mix(in oklab, ${colours.ink} ${n(100 * f.morph.u)}%, ${colours.accent})` : colours.ink;
    opacity(morph!, f.morph.opacity);
  }
  show(q(".sp-c")[0]!, f.cutC);
  q(".sp-l").forEach((el, i) => {
    const { dx, opacity: o } = f.letters[i]!;
    if (dx) el.setAttribute("transform", `translate(${n(dx)} 0)`);
    else el.removeAttribute("transform");
    opacity(el, o);
  });
  q<SVGPathElement>(".sp-line").forEach((el, i) => {
    el.style.clipPath = f.lines[i]! > 0 ? `inset(0 0 0 ${n(f.lines[i]!)}%)` : "";
  });
}
