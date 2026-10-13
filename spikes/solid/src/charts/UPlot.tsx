import { type Accessor, createEffect, createSignal, onCleanup, untrack } from "solid-js";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { createElementSize } from "../lib/primitives";
import { theme, type ThemeTokens } from "../theme/theme";

export type PlotOptions = Omit<uPlot.Options, "width" | "height">;

interface Props {
  /** Builds options from the theme. Tracked: when anything it reads changes, the plot is rebuilt. */
  options: (t: ThemeTokens) => PlotOptions;
  /** Data, pushed imperatively with setData() — never rebuilds the plot. */
  data: Accessor<uPlot.AlignedData>;
  class?: string;
  onCreate?: (u: uPlot) => void;
}

/**
 * uPlot lifecycle bound to the component: created on mount, rebuilt only when options/theme
 * change, resized by a ResizeObserver, destroyed on unmount. Data updates go straight to setData.
 */
export function UPlot(props: Props) {
  const [el, setEl] = createSignal<HTMLDivElement>();
  const size = createElementSize(el);
  const [plot, setPlot] = createSignal<uPlot>();

  createEffect(() => {
    const node = el();
    if (!node) return;
    const opts = props.options(theme.tokens());
    const s = untrack(size);
    const u = new uPlot({ ...opts, width: Math.max(s.width, 50), height: Math.max(s.height, 30) }, untrack(props.data), node);
    setPlot(u);
    untrack(() => props.onCreate?.(u));
    onCleanup(() => {
      u.destroy();
      setPlot(undefined);
    });
  });

  createEffect(() => {
    const u = plot();
    const s = size();
    if (u && s.width > 0 && s.height > 0) u.setSize({ width: s.width, height: s.height });
  });

  createEffect(() => {
    const u = plot();
    const d = props.data();
    if (u) u.setData(d);
  });

  return <div class={`uplot-host ${props.class ?? ""}`} ref={setEl} />;
}

/** Recessive axis styling from tokens. */
export function axis(t: ThemeTokens, extra: uPlot.Axis = {}): uPlot.Axis {
  return {
    stroke: t.muted,
    font: `11px ${t.font}`,
    grid: { stroke: t.grid, width: 1 },
    ticks: { stroke: t.axis, width: 1, size: 4 },
    gap: 4,
    ...extra,
  };
}
