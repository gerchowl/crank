// CRANK wordmark geometry (docs/design/BRAND.md §Construction). Framework-free on purpose: the same
// builders render the spike page (src/main.ts), export assets/brand/*.svg (scripts/export.ts) and can be
// imported unchanged by the React prototype (#49) or the production app.
//
// Units are font units of the outlined glyphs (src/glyphs.ts): cap height H = 700, y down, baseline y = 0.
import { CAP_HEIGHT as H, GLYPHS } from "./glyphs.ts";

export type Paint = { ink: string; accent: string };
export type Art = { viewBox: [number, number, number, number]; body: string };
export type ConceptId = "slice" | "blur" | "lockup" | "kinetic" | "instrument";

const DEG = Math.PI / 180;
/** Crank axis of the app mark (assets/icon/logo.svg, docs/design/ICON.md). Every diagonal derives from it. */
export const AXIS = 50;
/** Oblique of the letters: "slightly italic". Shear, not the font's italic, so the angle is ours. */
export const SLANT = 12;
/** Extra letter spacing; a heavy wordmark needs air that text kerning does not give. */
export const TRACK = 28;

const tSlant = Math.tan(SLANT * DEG);
const tAxis = Math.tan(AXIS * DEG);
const shear = `matrix(1 0 ${-tSlant.toFixed(5)} 1 0 0)`;
const n = (v: number) => +v.toFixed(2);

/** x origin of each glyph, plus the ink extent of the set (unsheared). */
function layout(from = 0) {
  const xs: number[] = [];
  let x = 0;
  for (const g of GLYPHS) {
    xs.push(x);
    x += g.advance + g.kern + TRACK;
  }
  const last = GLYPHS.length - 1;
  return { xs, left: xs[from]! + GLYPHS[from]!.xMin, right: xs[last]! + GLYPHS[last]!.xMax };
}

/** Sheared letters. `from` = 1 drops the C (the mark replaces it). */
function letters(paint: Paint, from = 0, dx = 0, extra = "") {
  const { xs } = layout(from);
  const paths = GLYPHS.slice(from)
    .map((g, i) => `<g class="wm-l" style="--i:${i}"><path transform="translate(${n(xs[i + from]! + dx)} 0)" d="${g.d}"/></g>`)
    .join("");
  // Animated elements never carry a transform attribute (CSS transform would replace it): wrappers do.
  return `<g class="wm-letters" fill="${paint.ink}"${extra}><g transform="${shear}">${paths}</g></g>`;
}

/** Union of horizontal strips covering [-H-pad, pad] minus the given bands (unsheared y). */
function stripsExcept(bands: [number, number][], x0: number, x1: number, pad = 60) {
  const edges = [-H - pad, ...bands.flat().sort((a, b) => a - b), pad];
  const rects: string[] = [];
  for (let i = 0; i < edges.length; i += 2)
    rects.push(`<rect x="${n(x0)}" y="${n(edges[i]!)}" width="${n(x1 - x0)}" height="${n(edges[i + 1]! - edges[i]!)}"/>`);
  return rects.join("");
}

/** A horizontal bar from x0 to x1 at y (centre) with both ends cut on the crank axis. */
function axisBar(x0: number, x1: number, y: number, h: number, cls = "") {
  const run = h / tAxis; // horizontal run of a 50° edge over the bar height
  const top = y - h / 2;
  const bot = y + h / 2;
  return `<path class="${cls}" d="M${n(x0 + run)} ${n(top)}H${n(x1 + run)}L${n(x1)} ${n(bot)}H${n(x0)}Z"/>`;
}

// --- the app mark (assets/icon/logo.svg without the squircle), hub at 0,0, ring outer radius 412.5 ---
const RING_OUTER = 350 + 125 / 2;
function arm(len: number, deg: number) {
  return `<path transform="rotate(${deg})" d="M0 -59L${len} -39A39 39 0 0 1 ${len} 39L0 59Z"/>`;
}
/** The crankset mark. The arms + chainring sit in `.crank` so the launch motion can turn them. */
export function mark(paint: Paint, id: string) {
  const pedal = { x: n(420 * Math.cos(-AXIS * DEG)), y: n(420 * Math.sin(-AXIS * DEG)) };
  return (
    `<path class="mk-ring" d="M164.32 -309.03A350 350 0 1 0 164.32 309.03" fill="none" stroke="${paint.accent}" stroke-width="125"/>` +
    `<mask id="${id}-holes" maskUnits="userSpaceOnUse" x="-500" y="-500" width="1000" height="1000">` +
    `<rect x="-500" y="-500" width="1000" height="1000" fill="#fff"/>` +
    `<circle r="56" fill="#000"/><circle cx="${pedal.x}" cy="${pedal.y}" r="26" fill="#000"/></mask>` +
    `<g class="crank" fill="${paint.accent}" mask="url(#${id}-holes)">${arm(310, -230)}${arm(420, -AXIS)}<circle r="108"/></g>`
  );
}

// --- concepts ------------------------------------------------------------------------------------

/** A — the owner's direction: oblique CRANK, two cuts slice through every letter and run on to the
 *  left as speed lines. Cut thickness g; cut spacing Δy; each higher line starts Δy / tan 50° further
 *  right, so the line starts sit on a line parallel to the crank axis. */
export const SLICE = { g: 40, cuts: [-305, -570], lead: 980 };
function slice(paint: Paint, id: string): Art {
  const { left, right } = layout();
  const { g, cuts, lead } = SLICE;
  const bands = cuts.map((y): [number, number] => [y - g / 2, y + g / 2]);
  const starts: number[] = [];
  const lines = cuts
    .map((y, i) => {
      const edge = left - y * tSlant; // the C's leading edge at this height, after the shear
      const start = edge - lead - y / tAxis; // y < 0: higher → further right, on the 50° line
      starts.push(start);
      return axisBar(start, edge - g, y, g, `wm-speed s${i}`);
    })
    .join("");
  const x0 = Math.min(...starts);
  const x1 = right + H * tSlant;
  return {
    viewBox: [x0 - 40, -H - 80, x1 - x0 + 120, H + 160],
    body:
      `<clipPath id="${id}-cut">${stripsExcept(bands, left - 100, x1 + 100)}</clipPath>` +
      `<g fill="${paint.accent}">${lines}</g>` +
      letters(paint, 0, 0, ` clip-path="url(#${id}-cut)"`),
  };
}

/** B — motion blur as stepped strokes: three echoes trail the word, each a set of horizontal strokes
 *  that get thinner with distance (pitch H/8, stroke 60 % → 35 % → 15 %), visible only behind the leading edge. */
export const BLUR = { step: 150, pitch: H / 8, fill: [0.6, 0.35, 0.15] };
function blur(paint: Paint, id: string): Art {
  const { left, right } = layout();
  const { step, pitch, fill } = BLUR;
  const clips = fill
    .map((f, k) => {
      const rows: string[] = [];
      for (let y = -H; y < 0; y += pitch)
        rows.push(`<rect x="${n(left - 400)}" y="${n(y + (pitch * (1 - f)) / 2)}" width="${n(right - left + 800)}" height="${n(pitch * f)}"/>`);
      return `<clipPath id="${id}-e${k}">${rows.join("")}</clipPath>`;
    })
    .join("");
  // Clip is applied in the sheared space of the echo group, so strokes stay horizontal (shear keeps rows).
  const echoes = fill
    .map((_, k) => `<g class="wm-echo e${k}" clip-path="url(#${id}-e${k})">${letters({ ...paint, ink: paint.accent }, 0, -(k + 1) * step)}</g>`)
    .reverse() // thinnest (farthest) first, so nearer, heavier strokes sit on top
    .join("");
  // Echoes only trail the word: clipped to the half-plane behind the C's leading edge (sheared, like the letters).
  const behind = `<clipPath id="${id}-behind"><path transform="${shear}" d="M${n(left - 1200)} ${-H - 60}H${n(left - 24)}V60H${n(left - 1200)}Z"/></clipPath>`;
  const x0 = left - fill.length * step;
  return {
    viewBox: [x0 - 80, -H - 80, right + H * tSlant - x0 + 160, H + 160],
    body: clips + behind + `<g clip-path="url(#${id}-behind)">${echoes}</g>` + letters(paint),
  };
}

/** C — mark-as-C: the crankset mark is the C, RANK follows. One asset for icon + name. */
export const LOCKUP = { scale: (H * 1.12) / 2 / RING_OUTER, gap: 36 };
function lockup(paint: Paint, id: string): Art {
  const { xs, right } = layout(1);
  const s = LOCKUP.scale;
  const r = RING_OUTER * s;
  const cx = r;
  const dx = cx + r + LOCKUP.gap - (xs[1]! + GLYPHS[1].xMin); // R's ink starts one gap after the ring
  return {
    viewBox: [-60, -H - 100, right + dx + H * tSlant + 120, H + 200],
    body: `<g class="mk" transform="translate(${n(cx)} ${-H / 2}) scale(${n(s)})">${mark(paint, id)}</g>` + letters(paint, 1, dx),
  };
}

/** D — kinetic: mark + word side by side; on launch the letters settle and the crank turns once and
 *  locks on its 50° axis (motion spec in BRAND.md). Static frame = horizontal lockup. */
export const KINETIC = { scale: (H * 1.32) / 2 / RING_OUTER, gap: 0.42 * H };
function kinetic(paint: Paint, id: string): Art {
  const { left, right } = layout();
  const r = RING_OUTER * KINETIC.scale;
  const dx = 2 * r + KINETIC.gap - left;
  return {
    viewBox: [-60, -H / 2 - r - 60, right + dx + H * tSlant + 120, 2 * r + 120],
    body: `<g class="mk" transform="translate(${n(r)} ${-H / 2}) scale(${n(KINETIC.scale)})">${mark(paint, id)}</g>` + letters(paint, 0, dx),
  };
}

/** E — instrument: a calm oblique CRANK over a measured scale; one brass index on the scale. Precision,
 *  not speed — the coach you trust. Ticks share the letters' slant; the index is cut on the crank axis. */
export const INSTRUMENT = { y: 150, rule: 12, ticks: 40, index: 0.62 };
function instrument(paint: Paint): Art {
  const { left, right } = layout();
  const { y, rule, ticks, index } = INSTRUMENT;
  const w = right - left;
  const marks: string[] = [];
  for (let i = 0; i <= ticks; i++) {
    const h = i % 10 === 0 ? 70 : i % 5 === 0 ? 46 : 26;
    marks.push(`<rect x="${n(left + (w * i) / ticks - rule / 2)}" y="${y}" width="${rule}" height="${h}"/>`);
  }
  const ix = left + w * index;
  return {
    viewBox: [left - 80, -H - 80, w + H * tSlant + 160, H + y + 200],
    body:
      letters(paint) +
      `<g class="in-scale"><g fill="${paint.ink}" opacity="0.55" transform="${shear}">` +
      `<rect x="${n(left)}" y="${y - rule}" width="${n(w)}" height="${rule}"/>${marks.join("")}</g></g>` +
      `<g class="in-index"><g fill="${paint.accent}" transform="${shear}">${axisBar(ix - 70, ix + 70, y - 46, 40)}</g></g>`,
  };
}

export const CONCEPTS: Record<ConceptId, { name: string; build: (p: Paint, id: string) => Art }> = {
  slice: { name: "A · Slice", build: slice },
  blur: { name: "B · Stepped blur", build: blur },
  lockup: { name: "C · Mark-as-C", build: lockup },
  kinetic: { name: "D · Kinetic lockup", build: kinetic },
  instrument: { name: "E · Instrument", build: instrument },
};

/** The recommended direction (BRAND.md §Recommendation); scripts/export.ts writes it to assets/brand/. */
export const RECOMMENDED: ConceptId = "slice";

export function svg(id: ConceptId, paint: Paint, uid: string = id, attrs = "") {
  const { viewBox, body } = CONCEPTS[id].build(paint, uid);
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox.map(n).join(" ")}"${attrs}>${body}</svg>`;
}
