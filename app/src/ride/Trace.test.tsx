// @vitest-environment jsdom
import { cleanup, render } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import type { Point } from "./session";
import { perSecond, Trace } from "./Trace";

afterEach(cleanup);

/** Samples at `hz` from session second 0 for `seconds`, each value its own index so a vertex names its sample. */
const samples = (hz: number, seconds: number, from = 0): Point[] =>
  Array.from({ length: hz * seconds }, (_, i) => ({
    t: from + i / hz,
    workoutS: from + i / hz,
    v: 100 + from * hz + i,
  }));

const vertices = (el: Element) =>
  [...el.querySelectorAll("polyline")].map((p) =>
    (p.getAttribute("points") ?? "").split(" "),
  );

// DESIGN §18.7 rule 17, §18.5.
test("advances one sample per second and never interpolates", () => {
  const points = samples(4, 5);
  const runs = perSecond(points);
  // Four samples a second become one vertex a second: the last sample of that second, exactly as sent.
  expect(runs).toHaveLength(1);
  expect(runs[0]!.map((p) => p.t)).toEqual([0.75, 1.75, 2.75, 3.75, 4.75]);
  expect(runs[0]!.every((p) => points.includes(p))).toBe(true);
  // A quarter second more adds nothing new; the next whole second adds one vertex.
  expect(
    perSecond(samples(4, 5).concat({ t: 4.9, workoutS: 4.9, v: 1 }))[0],
  ).toHaveLength(5);
  expect(perSecond(samples(4, 6))[0]).toHaveLength(6);

  const { container } = render(
    <Trace
      series="hr"
      label="Heart rate"
      unit="bpm"
      points={points}
      xMax={10}
      now={5}
    />,
  );
  expect(vertices(container)).toEqual([
    runs[0]!.map((p) =>
      expect.stringMatching(`^${(p.workoutS * 100).toFixed(1)},`),
    ),
  ]);
});

test("a second with no sample is a gap: no line is drawn across it", () => {
  // Seconds 0–2, then nothing until second 6 (a lost trainer sends no power).
  const points = [...samples(1, 3), ...samples(1, 2, 6)];
  const runs = perSecond(points);
  expect(runs.map((r) => r.map((p) => p.t))).toEqual([
    [0, 1, 2],
    [6, 7],
  ]);
  const { container } = render(
    <Trace
      series="power"
      label="Power"
      unit="W"
      points={points}
      xMax={10}
      now={8}
    />,
  );
  expect(vertices(container).map((v) => v.length)).toEqual([3, 2]);
});
