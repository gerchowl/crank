// Shared construction constants + layout (docs/design/BRAND.md §Construction). Used by the concepts
// (src/wordmark.ts) and by the build-time outline generator (scripts/slice.ts).
//
// Units are font units of the outlined glyphs (src/glyphs.ts): cap height H = 700, y down, baseline y = 0.
import { CAP_HEIGHT, GLYPHS } from "./glyphs.ts";

export const H = CAP_HEIGHT;
const DEG = Math.PI / 180;
/** Crank axis of the app mark (assets/icon/logo.svg, docs/design/ICON.md). Every diagonal derives from it. */
export const AXIS = 50;
/** Oblique of the letters: "slightly italic". Shear, not the font's italic, so the angle is ours. */
export const SLANT = 12;
/** Extra letter spacing; a heavy wordmark needs air that text kerning does not give. */
export const TRACK = 28;
/** Hand kerning that replaces the font's pair value (font R→A is +15, which reads looser than N→K). */
export const KERN: Record<string, number> = { RA: -12 };

export const tSlant = Math.tan(SLANT * DEG);
export const tAxis = Math.tan(AXIS * DEG);
export const shear = `matrix(1 0 ${-tSlant.toFixed(5)} 1 0 0)`;
export const n = (v: number) => +v.toFixed(2);

/** x origin of each glyph, plus the ink extent of the set (unsheared). */
export function layout(from = 0) {
  const xs: number[] = [];
  let x = 0;
  GLYPHS.forEach((g, i) => {
    xs.push(x);
    const pair = g.char + (GLYPHS[i + 1]?.char ?? "");
    x += g.advance + (KERN[pair] ?? g.kern) + TRACK;
  });
  const last = GLYPHS.length - 1;
  return { xs, left: xs[from]! + GLYPHS[from]!.xMin, right: xs[last]! + GLYPHS[last]!.xMax };
}

/** A — Slice. Cut thickness g; cut centres (unsheared y); `lead` = how far the speed lines reach left of the
 *  C at the baseline. Line starts lie on x = left − lead − y / tan 50° — a true 50° line, independent of the
 *  12° shear of the letters. Each bar's right end runs parallel to the C's outline at a horizontal gap g. */
export const SLICE = { g: 40, cuts: [-305, -570], lead: 940 };
