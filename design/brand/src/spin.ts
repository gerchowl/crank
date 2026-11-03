// F · Spin-up (#83, BRAND.md §F · Spin-up): the app mark spins up on its crank axis until the sweep closes the
// ring, then the ring glides into place and opens into the wordmark's C; R A N K come out of it, and one slice
// on the 50° crank axis runs right → left through the word and on as the speed lines — landing on A · Slice
// exactly. Every element is a pure function of time (`frame`), painted by `paint`; the page drives t from the
// clock (src/main.ts), the screenshots set it (?t=). No CSS animation, no raster blur, no translucent ink: the
// motion blur is stepped arc sectors whose sweep grows with angular speed, and the cuts open geometrically
// behind the slice front. The markup at rest IS the settled wordmark (A's paths, nothing else visible), so
// reduced motion = no script = the final frame.
import { H, SLICE, layout, n, tAxis } from "./geometry.ts";
import { SLICE_ART } from "./slice.ts";
import { SPIN_ART } from "./spin-art.ts";
import type { Art, Paint } from "./wordmark.ts";

const DEG = Math.PI / 180;

/** Timing (ms from the first frame) and physics. BRAND.md §F · Spin-up explains every number. */
export const SPIN = {
  /** Constant angular acceleration from rest until `up`: ω reaches ω_max and the sweep closes the ring. */
  up: 420,
  /** Trail persistence: an arm's trail spans ω·τ behind it (τ > one 60 Hz frame, so no wagon-wheel strobe). */
  tau: 60,
  /** Trail sweep at ω_max (°): two arms 180° apart close the ring; the 6° overlap leaves no seam. */
  sweep: 186,
  /** Trail steps per arm (nested sectors); ghost copies of the crank (opacity at full blur) and their lag span. */
  steps: 4,
  ghosts: [0.35, 0.15],
  ghostSpan: 60,
  /** Ring glides into the C's place and opens into it (`glide` curve). */
  morph: [420, 760] as const,
  /** C first, then R A N K out of it, left → right: R once the ring has cleared it, the rest 35 ms apart. */
  letter: 260,
  stagger: 35,
  /** The slice: a front on the 50° crank axis runs right → left through the uncut word, opening the two cuts
   *  behind it, and on along the speed lines until it lies on their chamfers (start, duration; `settle`). */
  slice: [780, 300] as const,
  /** Everything at rest: the settled wordmark. Target ≤ 1100 ms, under the 1.2 s launch budget. */
  end: 1080,
  /** The mark's size at the start: ring outer diameter = 1.5 H, centred on the letters. */
  scale: (1.5 * H) / 825,
};

/** ω_max in °/ms, and the constant angular acceleration α that reaches it at `up`. */
const W_MAX = SPIN.sweep / SPIN.tau;
const ALPHA = W_MAX / SPIN.up;
/** The ring band of the mark (logo.svg: radius 350, stroke 125). */
const R_IN = 287.5;
const R_OUT = 412.5;
/** Hub at the start: the optical centre of the letters, mid cap height. */
const HUB = [SLICE_ART.focus, -H / 2] as const;
/** The slice front x = f − y / tan 50°: from right of all ink (f at the drawing's right edge) to the line through
 *  the speed lines' chamfers (f = left − lead, BRAND.md §Construction), where it stops on A's geometry. */
const FRONT = [SLICE_ART.box[0] + SLICE_ART.box[2], layout().left - SLICE.lead] as const;
/** The ring's two faces start collinear and part into the C's opening. Until they are `SEAM_PX` apart on screen
 *  the slit between them is filled by `morph.seam` (in the shape's colour), so two abutting edges never show as an
 *  anti-aliasing hairline that opens, closes and opens again (#142). The fill is stroked in its own colour
 *  `SEAM_PX + 1` px wide, so it overlaps each face by ≥ ½ px instead of abutting it (abutting edges conflate to
 *  a crack of their own, and a stroke under a pixel wide cannot cover a whole pixel row). */
export const SEAM_PX = 1;

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
  const at = (deg: number, r: number): [number, number] => [cx + s * r * Math.cos(deg * DEG), cy + s * r * Math.sin(deg * DEG)];
  const fill = (count: number, f: (k: number) => [number, number]) => {
    for (let k = 0; k < count; k++) ring.push(f(k / count));
  };
  // Same order as the C's runs: outer arc, top face, inner arc (back), bottom face. Increasing angle is
  // clockwise on screen (y down): bottom-right → bottom → left → top → top-right. Both faces start on the same
  // radius at 3 o'clock (collinear: they part without crossing, so the outer edge never steps).
  fill(runs[0].length, (p) => at(360 * p, R_OUT));
  fill(runs[1].length, (p) => at(360, R_OUT + (R_IN - R_OUT) * p));
  fill(runs[2].length, (p) => at(360 * (1 - p), R_IN));
  fill(runs[3].length, (p) => at(0, R_IN + (R_OUT - R_IN) * p));
}
const cPts = runs.flat();

/** Where the four runs start in the vertex list (outer arc, top face, inner arc, bottom face). */
const FACE_TOP = runs[0].length;
const FACE_BOTTOM = FACE_TOP + runs[1].length + runs[2].length;
const FACE_VERTS = runs[1].length;

/** Both faces run between the same two radii: top face outer → inner, bottom face inner → outer, so top vertex
 *  i and bottom vertex FACE_VERTS − i sit on one radius (vertex 0 pairs with the outer arc's first, via the
 *  wrap). Their distance is zero for the ring and u · (the C's face separation) on the way. */
const facePair = (i: number) => [FACE_TOP + i, (FACE_BOTTOM + FACE_VERTS - i) % ring.length] as const;
/** Smallest separation of the two faces along their length, per unit of morph progress u (the faces are collinear
 *  in the ring, so it is linear in u). The slit stays filled until even the narrowest point is `SEAM_PX` apart. */
const FACE_GAP = Math.min(
  ...Array.from({ length: FACE_VERTS }, (_, i) => {
    const [a, b] = facePair(i);
    return Math.hypot(cPts[a]![0] - cPts[b]![0], cPts[a]![1] - cPts[b]![1]);
  }),
);

function morphShape(u: number) {
  const pts = ring.map(([x0, y0], i): [number, number] => {
    const [x1, y1] = cPts[i]!;
    return [n(x0 + (x1 - x0) * u), n(y0 + (y1 - y0) * u)];
  });
  const poly = (list: [number, number][]) => `${list.map(([x, y], i) => `${i ? "L" : "M"}${x} ${y}`).join("")}Z`;
  // The slit: down the top face (outer → inner) and back up the bottom face (inner → outer).
  const slit = Array.from({ length: FACE_VERTS }, (_, i) => pts[facePair(i)[0]]!).concat(
    Array.from({ length: FACE_VERTS }, (_, i) => pts[facePair(FACE_VERTS - i)[1]]!),
  );
  return { d: poly(pts), seam: poly(slit), gap: n(FACE_GAP * u) };
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
  mark: { theta: number; sweep: number; trail: number; crank: number; tip: number; ghosts: { lag: number; opacity: number }[] } | null;
  /** The ring → C shape while it glides, else null; `seam` is the slit between its faces (outline of the area
   *  to fill) and `gap` their smallest separation (units), which `paint` weighs against `SEAM_PX`. */
  morph: { d: string; u: number; seam: string; gap: number } | null;
  /** C (from the moment it has landed) then R A N K: x offset (units) and opacity. */
  letters: { dx: number; opacity: number }[];
  /** The slice front (x at y = 0) while it runs; null before (word uncut) and after (A at rest). */
  front: number | null;
  /** Before the slice the letters are uncut; after it they are A's cut letters only. */
  uncut: boolean;
  settled: boolean;
};

/** The whole sequence at time t (ms). Pure: same t, same frame. */
export function frame(t: number): Frame {
  const [m0, m1] = SPIN.morph;
  const [s0, sd] = SPIN.slice;
  const { theta, speed } = spinAt(t);
  let mark: Frame["mark"] = null;
  if (t < m0) {
    const sweep = Math.min(SPIN.sweep, W_MAX * speed * SPIN.tau);
    // Arms dissolve into their own blur from half speed on; the trail saturates to a solid ring at ω_max.
    const crank = 1 - clamp((speed - 0.45) / 0.55) ** 2;
    const lagSpan = Math.min(sweep, SPIN.ghostSpan);
    mark = {
      theta,
      sweep,
      trail: 0.3 + 0.7 * speed ** 2,
      crank,
      // The pedal end beyond the ring moves fastest: it blurs out first (gone by 60 % speed), never a bead.
      tip: crank * (1 - clamp((speed - 0.3) / 0.3)),
      ghosts: SPIN.ghosts.map((o, k) => ({ lag: (lagSpan * (k + 1)) / SPIN.ghosts.length, opacity: o * crank * speed })),
    };
  }
  const u = glide(progress(t, m0, m1 - m0));
  const landed = t >= m1;
  const f = progress(t, s0, sd);
  return {
    t,
    mark,
    morph: t >= m0 && !landed ? { ...morphShape(u), u } : null,
    letters: [
      { dx: 0, opacity: landed ? 1 : 0 },
      ...LETTER_START.map((s) => {
        const e = settle(progress(t, s, SPIN.letter));
        return { dx: -60 * (1 - e), opacity: t < s ? 0 : e };
      }),
    ],
    front: f > 0 && f < 1 ? FRONT[0] + (FRONT[1] - FRONT[0]) * settle(f) : null,
    uncut: f < 1,
    settled: t >= SPIN.end,
  };
}

/** Is the slit still filled? True until the faces are `SEAM_PX` apart on screen (`px` = screen px per unit). */
export const seamFilled = (gapUnits: number, px: number) => gapUnits * px < SEAM_PX;

// --- markup + painting -----------------------------------------------------------------------------------

/** Annular sector from angle a0 to a1 (°, a0 < a1) between radii r0 < r1, mark units. */
function sector(a0: number, a1: number, r0: number, r1: number) {
  const p = (a: number, r: number) => `${n(r * Math.cos(a * DEG))} ${n(r * Math.sin(a * DEG))}`;
  const large = a1 - a0 > 180 ? 1 : 0;
  return `M${p(a0, r1)}A${r1} ${r1} 0 ${large} 1 ${p(a1, r1)}L${p(a1, r0)}A${r0} ${r0} 0 ${large} 0 ${p(a0, r0)}Z`;
}

/** The half-plane left (`ahead`) or right of the slice front x = f − y / tan 50°, as a clip outline. */
function halfPlane(f: number, left: boolean) {
  const [y0, y1] = [-1200, 400];
  const far = left ? -1e4 : 1e4;
  return `M${far} ${y0}L${n(f - y0 / tAxis)} ${y0}L${n(f - y1 / tAxis)} ${y1}L${far} ${y1}Z`;
}

/** F's start-screen markup. At rest it paints exactly A · Slice (same paths, same order); the motion layers
 *  and the uncut letters are hidden until `paint` shows them. `id` keeps the two clip ids unique per page. */
export function spin(paint: Paint, id: string): Art {
  const { letters, bars, small, box, focus } = SLICE_ART;
  const trails = Array.from({ length: 2 * SPIN.steps }, () => `<path class="sp-trail"/>`).join("");
  const ghosts = SPIN.ghosts.map(() => `<path class="sp-ghost" d="${SPIN_ART.ghost}"/>`).join("");
  return {
    viewBox: box,
    focus,
    body:
      `<defs><clipPath id="${id}-ahead" class="sp-ahead" clipPathUnits="userSpaceOnUse"><path/></clipPath>` +
      `<clipPath id="${id}-behind" class="sp-behind" clipPathUnits="userSpaceOnUse"><path/></clipPath></defs>` +
      `<g fill="${paint.accent}">${bars.map((d) => `<path class="sp-line" data-clip="url(#${id}-behind)" d="${d}"/>`).join("")}</g>` +
      `<g fill="${paint.ink}">${letters
        .map(
          (d, i) =>
            `<g class="sp-l"><path d="${d}"/>` +
            `<path class="sp-uncut" display="none" data-clip="url(#${id}-ahead)" d="${small[i]}"/></g>`,
        )
        .join("")}</g>` +
      `<path class="sp-morph" display="none"/><path class="sp-seam" display="none"/>` +
      `<g class="sp-mark" display="none" fill="${paint.accent}" transform="translate(${n(HUB[0])} ${HUB[1]}) scale(${n(SPIN.scale)})">` +
      `<path d="M164.32 -309.03A350 350 0 1 0 164.32 309.03" fill="none" stroke="${paint.accent}" stroke-width="125"/>` +
      `${trails}${ghosts}<path class="sp-crank" d="${SPIN_ART.crank}"/><path class="sp-tip" d="${SPIN_ART.tip}"/></g>`,
  };
}

/** Writes frame f into F's markup. At `settled` the visible DOM is back at rest: the motion layers and the
 *  uncut letters hidden, no transform, opacity or clip left on A's paths. */
export function paint(svg: SVGSVGElement, f: Frame, colours: Paint) {
  const q = <E extends SVGElement>(sel: string) => [...svg.querySelectorAll<E>(sel)];
  const show = (el: Element, on: boolean) => (on ? el.removeAttribute("display") : el.setAttribute("display", "none"));
  const opacity = (el: Element, o: number) => (o >= 1 ? el.removeAttribute("opacity") : el.setAttribute("opacity", `${n(o)}`));
  const clip = (el: Element, on: boolean) => (on ? el.setAttribute("clip-path", el.getAttribute("data-clip")!) : el.removeAttribute("clip-path"));

  const [mark] = q(".sp-mark");
  show(mark!, f.mark !== null);
  if (f.mark) {
    const { theta, sweep, trail, crank, tip, ghosts } = f.mark;
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
    for (const [sel, o] of [[".sp-crank", crank], [".sp-tip", tip]] as const) {
      const [c] = q(sel);
      c!.setAttribute("transform", `rotate(${n(theta)})`);
      opacity(c!, o);
    }
  }

  const [morph] = q<SVGPathElement>(".sp-morph");
  const [seam] = q<SVGPathElement>(".sp-seam");
  show(morph!, f.morph !== null);
  show(seam!, false);
  if (f.morph) {
    // Brass ring → ink letter, mixed in Oklab along the same curve as the shape.
    const fill = `color-mix(in oklab, ${colours.ink} ${n(100 * f.morph.u)}%, ${colours.accent})`;
    morph!.setAttribute("d", f.morph.d);
    morph!.style.fill = fill;
    // The slit stays filled until the faces are visibly (SEAM_PX) apart; the scale is the drawing's on screen.
    const px = svg.getBoundingClientRect().width / svg.viewBox.baseVal.width;
    if (seamFilled(f.morph.gap, px)) {
      show(seam!, true);
      seam!.setAttribute("d", f.morph.seam);
      seam!.style.fill = fill;
      seam!.style.stroke = fill;
      seam!.style.strokeWidth = `${n((SEAM_PX + 1) / px)}`;
    }
  }

  const sliced = f.front !== null;
  if (sliced) {
    q(".sp-ahead path")[0]!.setAttribute("d", halfPlane(f.front!, true));
    q(".sp-behind path")[0]!.setAttribute("d", halfPlane(f.front!, false));
  }
  q(".sp-l").forEach((el, i) => {
    const { dx, opacity: o } = f.letters[i]!;
    if (dx) el.setAttribute("transform", `translate(${n(dx)} 0)`);
    else el.removeAttribute("transform");
    // Group opacity: the cut letter and its uncut cover fade as one, so the cut bands never show through.
    opacity(el, o);
  });
  q(".sp-uncut").forEach((el) => {
    show(el, f.uncut);
    clip(el, sliced);
  });
  // Lines: hidden until the front reaches their right end, drawn behind it, whole at rest.
  q(".sp-line").forEach((el) => {
    show(el, !f.uncut || sliced);
    clip(el, sliced);
  });
}
