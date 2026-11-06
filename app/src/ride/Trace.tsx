// One trace of the ride screen (DESIGN §18.3, §18.5): a live series over the whole workout on the shared x axis (workout
// seconds) with the "now" cursor; the power trace also draws the planned target and its ±5 % band, ghosted ahead of now.
// It draws what the core sent: one vertex per second of session time, the last sample of that second (§18.7 rule 17),
// straight runs between consecutive seconds only, and a gap where a second has no sample. No smoothing, no zones.
import { useId } from "react";
import type { WorkoutBlock } from "../bindings";
import type { Point, Series } from "./session";

const W = 1000;
const H = 100;
/** The compliance band around the controlled target (`StreamCompliance.withinS`: within ±5 %, ADR-0002 §7.8). */
export const BAND = 0.05;
const HEADROOM = 1.1;
const LINE_SHARE = 0.75;

/** One vertex per whole second of session time (the last sample in it); a missing second starts a new run. */
export function perSecond(points: readonly Point[]): Point[][] {
  const runs: Point[][] = [];
  let run: Point[] = [];
  let sec = Number.NaN;
  for (const p of points) {
    const s = Math.floor(p.t);
    if (s === sec) {
      run[run.length - 1] = p;
      continue;
    }
    if (s !== sec + 1 && run.length > 0) {
      runs.push(run);
      run = [];
    }
    run.push(p);
    sec = s;
  }
  if (run.length > 0) runs.push(run);
  return runs;
}

const planned = (blocks: readonly WorkoutBlock[]) =>
  blocks.flatMap((b) =>
    b.startS === null || b.durS === null || b.powerFrom === null
      ? []
      : [
          {
            s: b.startS,
            e: b.startS + b.durS,
            from: b.powerFrom,
            to: b.powerTo ?? b.powerFrom,
          },
        ],
  );

interface Props {
  series: Series;
  /** The series' word, its colour's label (§18.7 rule 11). */
  label: string;
  unit: string;
  points: readonly Point[];
  /** The x axis' end, workout seconds. */
  xMax: number;
  /** The workout clock, s. */
  now: number;
  /** The plan, for the power trace. */
  plan?: readonly WorkoutBlock[];
}

export function Trace({ series, label, unit, points, xMax, now, plan }: Props) {
  const clip = useId();
  const blocks = planned(plan ?? []);
  const values = points.map((p) => p.v);
  const [lo, hi] =
    series === "power"
      ? [
          0,
          HEADROOM *
            Math.max(100, ...values, ...blocks.flatMap((b) => [b.from, b.to])),
        ]
      : values.length === 0
        ? [0, 1]
        : [Math.min(...values) - 10, Math.max(...values) + 10];
  const x = (s: number) => (s / xMax) * W;
  // The top quarter is the label's: no value draws under it.
  const y = (v: number) => H - ((v - lo) / (hi - lo)) * H * LINE_SHARE;
  const bands = blocks.map((b, i) => {
    const [x0, x1] = [x(b.s), x(b.e)];
    return (
      <g key={i}>
        <path
          className="trace-band"
          d={`M${x0},${y(b.from * (1 + BAND))} L${x1},${y(b.to * (1 + BAND))} L${x1},${y(b.to * (1 - BAND))} L${x0},${y(b.from * (1 - BAND))} Z`}
          vectorEffect="non-scaling-stroke"
        />
        <path
          className="trace-target"
          d={`M${x0},${y(b.from)} L${x1},${y(b.to)}`}
          vectorEffect="non-scaling-stroke"
        />
      </g>
    );
  });
  return (
    <div className="trace" data-series={series}>
      <span className="trace-label">
        <span data-series-label>{label}</span> · {unit}
      </span>
      {/* eslint-disable-next-line no-restricted-syntax -- a data graphic, not an icon (DESIGN §18.7 rule 4 is about icons). */}
      <svg
        data-graphic
        role="img"
        aria-label={`${label} over the workout`}
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
      >
        {blocks.length > 0 && (
          <>
            <clipPath id={clip}>
              <rect x={0} y={0} width={x(now)} height={H} />
            </clipPath>
            <g className="trace-plan" data-plan="ahead">
              {bands}
            </g>
            <g
              className="trace-plan"
              data-plan="past"
              clipPath={`url(#${clip})`}
            >
              {bands}
            </g>
          </>
        )}
        {perSecond(points).map((run, i) => (
          <polyline
            key={i}
            data-testid={`chart-${series}`}
            className="trace-line"
            points={run
              .map((p) => `${x(p.workoutS).toFixed(1)},${y(p.v).toFixed(1)}`)
              .join(" ")}
            vectorEffect="non-scaling-stroke"
          />
        ))}
        <line
          className="trace-now"
          x1={x(now)}
          x2={x(now)}
          y1={0}
          y2={H}
          vectorEffect="non-scaling-stroke"
        />
      </svg>
    </div>
  );
}
