// Shedding text that must stay on one line (DESIGN §6.1, §16.1): the first of the candidates, longest first, whose width
// in the element's own font fits the element's box. Measured with a canvas in the computed font, again when the box
// resizes and once the fonts have loaded; where the browser has no layout (jsdom: no ResizeObserver) the first stays.
import { useLayoutEffect, useRef, useState, type RefObject } from "react";

let canvas: HTMLCanvasElement | null = null;

export function useFit<E extends HTMLElement>(
  candidates: readonly string[],
): [RefObject<E | null>, string] {
  const ref = useRef<E>(null);
  const [at, setAt] = useState(0);
  const key = candidates.join("\n");
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const all = key.split("\n");
    const fit = () => {
      canvas ??= document.createElement("canvas");
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      // Not the `font` shorthand: it serializes as "" when a longhand it cannot express is set (font-variant-numeric).
      const cs = getComputedStyle(el);
      ctx.font = `${cs.fontStyle} ${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
      const box = el.clientWidth;
      const i = all.findIndex((c) => ctx.measureText(c).width <= box);
      setAt(i < 0 ? all.length - 1 : i);
    };
    fit();
    const watch = new ResizeObserver(fit);
    watch.observe(el);
    void document.fonts.ready.then(fit);
    return () => watch.disconnect();
  }, [key]);
  return [ref, candidates[Math.min(at, candidates.length - 1)] ?? ""];
}
