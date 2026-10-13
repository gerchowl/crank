// uPlot lifecycle as a React 19 callback ref: the chart is created when its element mounts (or when
// `opts` change, e.g. on theme switch — the ref's identity changes, React runs the cleanup and re-attaches),
// destroyed on unmount, resized with its container. Data is pushed imperatively via setData.
import { useCallback, useRef } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";

export type PlotOpts = Omit<uPlot.Options, "width" | "height">;

export function useUPlot(opts: PlotOpts) {
  const plot = useRef<uPlot | null>(null);
  const data = useRef<uPlot.AlignedData>([[]]);

  const el = useCallback(
    (host: HTMLDivElement | null) => {
      if (!host) return;
      const r = host.getBoundingClientRect();
      const u = new uPlot({ ...opts, width: Math.max(50, r.width), height: Math.max(40, r.height) }, data.current, host);
      plot.current = u;
      const ro = new ResizeObserver(([e]) => {
        if (!e) return;
        u.setSize({ width: Math.max(50, e.contentRect.width), height: Math.max(40, e.contentRect.height) });
      });
      ro.observe(host);
      return () => {
        ro.disconnect();
        u.destroy();
        if (plot.current === u) plot.current = null;
      };
    },
    [opts],
  );

  const setData = useCallback((d: uPlot.AlignedData) => {
    data.current = d;
    plot.current?.setData(d);
  }, []);

  return { el, setData };
}
