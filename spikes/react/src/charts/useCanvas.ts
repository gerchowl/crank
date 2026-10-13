// A DPR-aware canvas that repaints on resize, when `version` changes (theme, data, config) and on demand.
// Uses a React 19 callback ref with cleanup, so it also works for canvases that mount later
// (conditional rendering, table-view toggles).
import { useCallback, useEffect, useLayoutEffect, useRef } from "react";

export type Paint = (ctx: CanvasRenderingContext2D, w: number, h: number) => void;

export function useCanvas(paint: Paint, version: unknown) {
  const canvas = useRef<HTMLCanvasElement | null>(null);
  const size = useRef({ w: 0, h: 0 });
  const paintRef = useRef(paint);
  useLayoutEffect(() => {
    paintRef.current = paint;
  });

  const redraw = useCallback(() => {
    const c = canvas.current;
    const ctx = c?.getContext("2d");
    if (!c || !ctx) return;
    const { w, h } = size.current;
    if (w <= 0 || h <= 0) return;
    ctx.setTransform(devicePixelRatio, 0, 0, devicePixelRatio, 0, 0);
    ctx.clearRect(0, 0, w, h);
    paintRef.current(ctx, w, h);
  }, []);

  const ref = useCallback(
    (c: HTMLCanvasElement | null) => {
      canvas.current = c;
      if (!c) return;
      const ro = new ResizeObserver(([e]) => {
        if (!e) return;
        const { width, height } = e.contentRect;
        size.current = { w: width, h: height };
        c.width = Math.round(width * devicePixelRatio);
        c.height = Math.round(height * devicePixelRatio);
        redraw();
      });
      ro.observe(c);
      return () => {
        ro.disconnect();
        canvas.current = null;
      };
    },
    [redraw],
  );

  useEffect(() => {
    redraw();
  }, [version, redraw]);

  return { ref, redraw };
}
