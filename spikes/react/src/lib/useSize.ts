import { useCallback, useState } from "react";

/** Element size via a callback ref (React 19 ref cleanup). Re-renders on resize only. */
export function useSize(): [(el: Element | null) => (() => void) | undefined, { w: number; h: number }] {
  const [size, setSize] = useState({ w: 0, h: 0 });
  const ref = useCallback((el: Element | null) => {
    if (!el) return undefined;
    const ro = new ResizeObserver(([e]) => {
      if (e) setSize({ w: Math.round(e.contentRect.width), h: Math.round(e.contentRect.height) });
    });
    ro.observe(el);
    return () => {
      ro.disconnect();
    };
  }, []);
  return [ref, size];
}
