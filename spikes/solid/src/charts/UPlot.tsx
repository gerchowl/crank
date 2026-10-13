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

/**
 * Hover tooltip plugin (crosshair + values at the cursor). Plain DOM inside uPlot's `over`
 * layer, so hovering never touches the Solid tree.
 */
export function tooltipPlugin(fmtX: (x: number) => string, fmtY: (v: number, seriesIdx: number) => string, only?: number[]): uPlot.Plugin {
  let tip: HTMLDivElement | null = null;
  return {
    hooks: {
      init: (u) => {
        tip = document.createElement("div");
        tip.className = "tooltip";
        tip.style.display = "none";
        u.over.appendChild(tip);
        u.over.addEventListener("mouseleave", () => {
          if (tip) tip.style.display = "none";
        });
      },
      setCursor: (u) => {
        const i = u.cursor.idx;
        const left = u.cursor.left ?? -1;
        if (!tip || i == null || left < 0) {
          if (tip) tip.style.display = "none";
          return;
        }
        const x = u.data[0][i];
        if (x == null) return;
        const rows: string[] = [];
        u.series.forEach((s, si) => {
          if (si === 0 || s.show === false || (only && !only.includes(si))) return;
          const v = u.data[si]?.[i];
          if (v == null) return;
          const color = typeof s.stroke === "string" ? s.stroke : "";
          rows.push(`<div><span class="tt-swatch" style="background:${color}"></span>${typeof s.label === "string" ? s.label : ""} <b>${fmtY(v, si)}</b></div>`);
        });
        if (!rows.length) {
          tip.style.display = "none";
          return;
        }
        tip.innerHTML = `<div class="muted">${fmtX(x)}</div>${rows.join("")}`;
        tip.style.display = "block";
        const w = u.over.clientWidth;
        tip.style.left = `${Math.min(left + 12, w - 150)}px`;
        tip.style.top = `${Math.max(0, (u.cursor.top ?? 0) - 20)}px`;
      },
    },
  };
}
