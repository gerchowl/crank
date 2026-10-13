// Svelte attachments for imperative charts.
//
// Both follow the same split:
//   - the attachment body (an effect) reads only *structural* inputs (theme, series config);
//     when those change the chart is torn down and rebuilt;
//   - a nested $effect reads the *data*; when data changes the chart is updated in place
//     (uPlot.setData / canvas redraw) — no DOM re-render, no rebuild.
import type { Attachment } from "svelte/attachments";
import uPlot from "uplot";

export interface Size {
  w: number;
  h: number;
}

function observeSize(el: Element, onSize: (s: Size) => void): () => void {
  const ro = new ResizeObserver((entries) => {
    const r = entries[0]?.contentRect;
    if (r) onSize({ w: Math.floor(r.width), h: Math.floor(r.height) });
  });
  ro.observe(el);
  return () => {
    ro.disconnect();
  };
}

/**
 * uPlot with lifecycle bound to the element. `options` is read reactively (rebuild),
 * `data` is read reactively in a nested effect (setData). `bind` hands the instance out for
 * imperative streaming updates (sparklines) and gets `null` on destroy.
 */
export function uplot(
  options: () => Omit<uPlot.Options, "width" | "height">,
  data: () => uPlot.AlignedData,
  bind?: (u: uPlot | null) => void,
): Attachment<HTMLElement> {
  return (node) => {
    const opts = options();
    const u = new uPlot({ ...opts, width: Math.max(10, node.clientWidth), height: Math.max(10, node.clientHeight) }, [[]], node);
    bind?.(u);
    const stop = observeSize(node, ({ w, h }) => {
      if (w > 0 && h > 0) u.setSize({ width: w, height: h });
    });
    $effect(() => {
      u.setData(data());
    });
    return () => {
      stop();
      bind?.(null);
      u.destroy();
    };
  };
}

/**
 * HiDPI canvas. `draw` runs in an effect: any reactive state it reads (data, theme, hover)
 * schedules a redraw; resizes redraw too.
 */
export function canvas2d(draw: (ctx: CanvasRenderingContext2D, size: Size) => void): Attachment<HTMLCanvasElement> {
  return (cv) => {
    let size = $state<Size>({ w: 0, h: 0 });
    const stop = observeSize(cv, (s) => {
      size = s;
    });
    const ctx = cv.getContext("2d");
    $effect(() => {
      const { w, h } = size;
      if (!ctx || w === 0 || h === 0) return;
      const dpr = window.devicePixelRatio || 1;
      if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) {
        cv.width = Math.round(w * dpr);
        cv.height = Math.round(h * dpr);
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      draw(ctx, size);
    });
    return stop;
  };
}
