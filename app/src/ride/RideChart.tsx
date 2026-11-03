// The whole-ride overview (DESIGN §5.2 bottom strip, #96 MVP): the workout's planned power (edge stroke in
// `--target`, the fill is decoration, §6.3) with the ridden power drawn over it and a cursor at the workout clock.
// It draws what the core sent (compiled blocks, stream samples): no smoothing, no zones, no derived values.
import type { WorkoutBlock } from "../bindings";
import type { PowerPoint } from "./session";

const W = 1000;
const H = 100;
const MAX_POINTS = 1000;
/** Headroom above the highest value so the line never touches the frame. */
const HEADROOM = 1.1;

interface Props {
  blocks: readonly WorkoutBlock[];
  /** Planned length, s; `null` when the workout has none (a free ride): the axis follows the ride. */
  totalS: number | null;
  power: readonly PowerPoint[];
  /** The workout clock, s. */
  workoutS: number;
}

export function RideChart({ blocks, totalS, power, workoutS }: Props) {
  const planned = blocks.filter(
    (b) => b.startS !== null && b.durS !== null && b.powerFrom !== null,
  );
  const xMax = Math.max(totalS ?? 0, workoutS, 1);
  const yMax =
    HEADROOM *
    Math.max(
      100,
      ...planned.flatMap((b) => [b.powerFrom ?? 0, b.powerTo ?? 0]),
      ...power.map((p) => p.w),
    );
  const x = (s: number) => (s / xMax) * W;
  const y = (w: number) => H - (w / yMax) * H;

  const stride = Math.max(1, Math.ceil(power.length / MAX_POINTS));
  const ridden = power
    .filter((_, i) => i % stride === 0)
    .map((p) => `${x(p.workoutS).toFixed(1)},${y(p.w).toFixed(1)}`)
    .join(" ");

  return (
    <svg
      className="ride-chart"
      role="img"
      aria-label="Workout overview: planned power, ridden power and your position"
      viewBox={`0 0 ${W} ${H}`}
      preserveAspectRatio="none"
    >
      {planned.map((b, i) => {
        const [s, d] = [b.startS ?? 0, b.durS ?? 0];
        const [from, to] = [b.powerFrom ?? 0, b.powerTo ?? b.powerFrom ?? 0];
        const path = `M${x(s)},${y(from)} L${x(s + d)},${y(to)}`;
        return (
          <g key={i} data-testid="chart-block">
            <path
              d={`${path} L${x(s + d)},${H} L${x(s)},${H} Z`}
              fill="var(--accent-soft)"
            />
            <path
              d={path}
              fill="none"
              stroke="var(--target)"
              strokeWidth={2}
              vectorEffect="non-scaling-stroke"
            />
          </g>
        );
      })}
      {ridden !== "" && (
        <polyline
          data-testid="chart-power"
          points={ridden}
          fill="none"
          stroke="var(--series-power)"
          strokeWidth={2}
          vectorEffect="non-scaling-stroke"
        />
      )}
      <line
        data-testid="chart-cursor"
        x1={x(workoutS)}
        x2={x(workoutS)}
        y1={0}
        y2={H}
        stroke="var(--ink-primary)"
        strokeWidth={2}
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}
