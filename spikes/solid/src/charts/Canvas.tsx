import { createEffect, createSignal, type JSX, Show } from "solid-js";
import { createElementSize } from "../lib/primitives";
import { theme, type ThemeTokens } from "../theme/theme";

export interface Hover {
  x: number;
  y: number;
}

interface Props {
  /**
   * Draw function. Runs inside an effect, so every signal it reads (data polls, props, theme)
   * re-triggers a redraw of this canvas only.
   */
  draw: (ctx: CanvasRenderingContext2D, w: number, h: number, t: ThemeTokens) => void;
  /** Return tooltip content for a pointer position (CSS px), or null. */
  tooltip?: (p: Hover, w: number, h: number) => JSX.Element | null;
  class?: string;
  onPointerDown?: (p: Hover, e: PointerEvent) => void;
  ariaLabel?: string;
}

export function Canvas(props: Props) {
  const [wrap, setWrap] = createSignal<HTMLDivElement>();
  const [canvas, setCanvas] = createSignal<HTMLCanvasElement>();
  const size = createElementSize(wrap);
  const [hover, setHover] = createSignal<Hover | null>(null);

  createEffect(() => {
    const c = canvas();
    const { width, height } = size();
    if (!c || width === 0 || height === 0) return;
    const dpr = window.devicePixelRatio || 1;
    if (c.width !== Math.round(width * dpr) || c.height !== Math.round(height * dpr)) {
      c.width = Math.round(width * dpr);
      c.height = Math.round(height * dpr);
    }
    const ctx = c.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    props.draw(ctx, width, height, theme.tokens());
  });

  const pos = (e: PointerEvent): Hover => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  const tip = () => {
    const h = hover();
    const s = size();
    return h && props.tooltip ? { h, content: props.tooltip(h, s.width, s.height) } : null;
  };

  return (
    <div class={`canvas-host ${props.class ?? ""}`} ref={setWrap}>
      <canvas
        ref={setCanvas}
        role="img"
        aria-label={props.ariaLabel ?? "chart"}
        onPointerMove={(e) => setHover(pos(e))}
        onPointerLeave={() => setHover(null)}
        onPointerDown={(e) => props.onPointerDown?.(pos(e), e)}
      />
      <Show when={tip()?.content}>
        {(content) => (
          <div
            class="tooltip"
            style={{
              left: `${Math.min((tip()?.h.x ?? 0) + 12, size().width - 140)}px`,
              top: `${Math.max((tip()?.h.y ?? 0) - 10, 0)}px`,
            }}
          >
            {content()}
          </div>
        )}
      </Show>
    </div>
  );
}
