// The block timeline (DESIGN §18.3; replaces the road ahead and the overview of §16.1): the workout's blocks by height,
// the ridden part dimmed, a "now" marker, and each block's name and watts in the number face where they fit. It draws
// the compiled plan only. Coach marks (dashed = proposed, solid = applied) go on it once the coach lane exists (#99).
import type { WorkoutBlock } from "../bindings";
import { useFit } from "./fit";

const W = 1000;
const H = 40;

/** A block's phase as a name; the compiled block carries no label of its own. */
export const PHASE_NAME = {
  warmup: "Warm-up",
  work: "Work",
  recovery: "Recover",
  cooldown: "Cool-down",
  free: "Free",
} as const;

/** "250 W", or "100–180 W" for a ramp. */
export const watts = (b: WorkoutBlock): string =>
  b.powerFrom === null
    ? ""
    : b.powerTo === null || b.powerTo === b.powerFrom
      ? `${b.powerFrom} W`
      : `${b.powerFrom}–${b.powerTo} W`;

function Label({
  block,
  index,
  left,
  width,
}: {
  block: WorkoutBlock;
  index: number;
  left: number;
  width: number;
}) {
  const w = watts(block);
  const [ref, text] = useFit<HTMLSpanElement>([
    `${PHASE_NAME[block.phase]} ${w}`.trim(),
    w,
    "",
  ]);
  return (
    <span
      ref={ref}
      className="timeline-label num"
      data-block-label={index}
      style={{ left: `${left}%`, width: `${width}%` }}
    >
      {text}
    </span>
  );
}

interface Props {
  blocks: readonly WorkoutBlock[];
  /** Planned length, s; `null` without a plan (a free ride): the axis follows the ride. */
  totalS: number | null;
  /** The workout clock, s. */
  workoutS: number;
}

export function Timeline({ blocks, totalS, workoutS }: Props) {
  const xMax = Math.max(totalS ?? 0, workoutS, 1);
  const top = Math.max(
    1,
    ...blocks.map((b) => Math.max(b.powerFrom ?? 0, b.powerTo ?? 0)),
  );
  const x = (s: number) => (s / xMax) * W;
  const y = (w: number) => H - (w / top) * H * 0.95;
  const shown = blocks.filter((b) => b.startS !== null && b.durS !== null);
  return (
    <>
      <div className="timeline-head">
        <span>
          {blocks.length > 0
            ? `Workout · ${blocks.length} blocks`
            : "Free ride · no plan"}
        </span>
      </div>
      {/* eslint-disable-next-line no-restricted-syntax -- a data graphic, not an icon (DESIGN §18.7 rule 4 is about icons). */}
      <svg
        data-graphic
        role="img"
        aria-label="Workout blocks, the ridden part and your position"
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
      >
        {shown.map((b, i) => {
          const [s, e] = [b.startS ?? 0, (b.startS ?? 0) + (b.durS ?? 0)];
          const [from, to] = [b.powerFrom ?? 0, b.powerTo ?? b.powerFrom ?? 0];
          return (
            <path
              key={i}
              data-testid="chart-block"
              data-block={i}
              className="timeline-block"
              data-ridden={
                e <= workoutS ? "done" : s <= workoutS ? "now" : "ahead"
              }
              d={`M${x(s) + 1},${H} L${x(s) + 1},${y(from)} L${x(e) - 1},${y(to)} L${x(e) - 1},${H} Z`}
              vectorEffect="non-scaling-stroke"
            />
          );
        })}
        <rect
          className="timeline-ridden"
          x={0}
          y={0}
          width={x(workoutS)}
          height={H}
        />
        <line
          data-testid="chart-cursor"
          className="timeline-now"
          x1={x(workoutS)}
          x2={x(workoutS)}
          y1={0}
          y2={H}
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      <div className="timeline-labels">
        {shown.map((b, i) => (
          <Label
            key={i}
            block={b}
            index={i}
            left={((b.startS ?? 0) / xMax) * 100}
            width={((b.durS ?? 0) / xMax) * 100}
          />
        ))}
      </div>
    </>
  );
}
