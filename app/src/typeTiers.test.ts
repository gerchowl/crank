import { expect, test } from "vitest";
import {
  BANNER_MAX_CHARS,
  FALLBACK_NOTICE,
  NOTICE_MAX_CHARS,
  TOO_SMALL_TEXT,
  FALLBACK_TOKENS,
  computeTiers,
  type Monitor,
} from "./typeTiers";

// Reference devices (DESIGN §6.2): CSS px per mm = widthPx / scale / widthMm.
const laptop: Monitor = { widthPx: 1440, scaleFactor: 1, widthMm: 288 }; // 5.0 px/mm
const tablet: Monitor = { widthPx: 1024, scaleFactor: 1, widthMm: 1024 / 5.2 };
const compute = (
  monitor: Monitor,
  distanceM: number,
  window: { width: number; height: number },
  widthMmOverride: number | null = null,
) => computeTiers({ monitor, widthMmOverride, distanceM, window });

const near = (got: number, want: number) =>
  expect(Math.abs(got - want)).toBeLessThanOrEqual(1);

test("1440×900 @ 1.0 m, 5.0 pt/mm matches the §6.2 table ±1 px", () => {
  const t = compute(laptop, 1.0, { width: 1440, height: 900 });
  expect(t.kind).toBe("physical");
  const px = t.px!;
  near(px.t1, 182);
  near(px.t2, 101);
  near(px.t3p, 40);
  near(px.t3, 32);
  expect(t.tooSmallForT2).toBe(false);
});

test("1024×768 @ 0.8 m, 5.2 pt/mm matches the §6.2 table ±1 px", () => {
  const t = compute(tablet, 0.8, { width: 1024, height: 768 });
  const px = t.px!;
  near(px.t1, 151);
  near(px.t2, 84);
  near(px.t3p, 34);
  near(px.t3, 27);
  expect(t.tooSmallForT2).toBe(false);
});

test("scale factor cancels: the same panel at 2× reports the same px", () => {
  const retina: Monitor = { widthPx: 2880, scaleFactor: 2, widthMm: 288 };
  expect(compute(retina, 1.0, { width: 1440, height: 900 }).px).toEqual(
    compute(laptop, 1.0, { width: 1440, height: 900 }).px,
  );
});

test("exports tokens in px, with T1r = 0.62 × T1 and fixed T4", () => {
  const t = compute(laptop, 1.0, { width: 1440, height: 900 });
  expect(t.tokens["--t1"]).toBe("182px");
  expect(t.tokens["--t1r"]).toBe(`${Math.round(182 * 0.62)}px`);
  expect(t.tokens["--t4"]).toBe("16px");
});

test("override wins over the measured width", () => {
  const measured = { ...laptop, widthMm: 400 };
  const t = compute(measured, 1.0, { width: 1440, height: 900 }, 288);
  near(t.px!.t2, 101);
});

test("unknown physical size falls back to the prototype clamp rules with a notice", () => {
  const t = compute({ ...laptop, widthMm: null }, 1.0, {
    width: 1440,
    height: 900,
  });
  expect(t.kind).toBe("fallback");
  expect(t.tokens).toEqual(FALLBACK_TOKENS);
  expect(t.notice).toBe(FALLBACK_NOTICE);
  expect(t.tooSmallForT2).toBe(false);
});

test.each([0, -1, NaN])("invalid distance %s degrades to the fallback", (d) => {
  expect(compute(laptop, d, { width: 1440, height: 900 }).kind).toBe(
    "fallback",
  );
});

test("T2 that no longer fits the window sets tooSmallForT2 (1200×700 on the laptop)", () => {
  const t = compute(laptop, 1.0, { width: 1200, height: 700 });
  expect(t.tooSmallForT2).toBe(true);
});

test("a farther rider needs bigger text, so the same window becomes too small", () => {
  const w = { width: 1024, height: 768 };
  expect(compute(tablet, 0.8, w).tooSmallForT2).toBe(false);
  expect(compute(tablet, 1.2, w).tooSmallForT2).toBe(true);
});

test("the reference devices in their real maximized windows do not warn", () => {
  expect(compute(laptop, 1.0, { width: 1440, height: 850 }).tooSmallForT2).toBe(
    false,
  );
  expect(compute(tablet, 0.8, { width: 1024, height: 715 }).tooSmallForT2).toBe(
    false,
  );
});

test("status-bar strings fit their one-line maximum", () => {
  expect(FALLBACK_NOTICE.length).toBeLessThanOrEqual(NOTICE_MAX_CHARS);
  expect(TOO_SMALL_TEXT.length).toBeLessThanOrEqual(BANNER_MAX_CHARS);
});
