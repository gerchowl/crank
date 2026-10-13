import { createMemo, Show } from "solid-js";
import { ftp } from "../backend";
import { Canvas, type Hover } from "../charts/Canvas";
import { layoutProfile, paintProfile, profileView, timeOfX } from "../charts/profile";
import { clock } from "../lib/format";
import { ride } from "../state/ride";
import type { ThemeTokens } from "../theme/theme";
import type { WidgetProps } from "./common";

/** P13 strip — full profile (or a window with spanS), "you are here", AI-changed blocks outlined. */
export function WorkoutStrip(props: WidgetProps) {
  const blocks = () => ride.current()?.compiled.blocks ?? [];
  const windowed = () => props.widget.spanS !== undefined;
  const next = createMemo(() => blocks()[ride.status.blockIndex + 1]);
  const view = () => {
    const all = blocks();
    if (!windowed()) return profileView(all, { pad: { l: 0, r: 0, t: 14, b: 2 } });
    const span = props.widget.spanS ?? 900;
    const t0 = Math.max(0, ride.status.t - span * 0.15);
    return profileView(all, { t0, t1: t0 + span, pad: { l: 0, r: 0, t: 14, b: 2 } });
  };
  const draw = (ctx: CanvasRenderingContext2D, w: number, h: number, t: ThemeTokens) => {
    const v = view();
    const rects = layoutProfile(blocks(), w, h, v);
    paintProfile(ctx, rects, t, w, v, { hereT: ride.status.t, ftpLine: true });
  };
  const tooltip = (p: Hover, w: number) => {
    const v = view();
    const s = timeOfX(p.x, w, v);
    const b = blocks().find((x) => s >= x.startS && s < x.startS + x.durS);
    if (!b) return null;
    return (
      <>
        <b>
          {clock(b.durS)} @ {Math.round(b.powerFrom)}
          {b.powerTo !== b.powerFrom ? `→${Math.round(b.powerTo)}` : ""} W
        </b>
        <div class="muted">
          {b.phase} · {Math.round((b.powerFrom / ftp) * 100)}% FTP{b.aiChanged ? " · AI-changed" : ""}
        </div>
      </>
    );
  };
  const aiCount = () => blocks().filter((b) => b.aiChanged).length;
  return (
    <div class="chart-widget strip">
      <div class="w-label">
        <span>
          <Show when={windowed()} fallback="Workout">
            <Show when={next()} fallback="Next: finish">
              {(n) => (
                <>
                  NEXT: {clock(n().durS)} @ {Math.round(n().powerFrom)} W <span class="muted">({Math.round((n().powerFrom / ftp) * 100)}% · {n().phase})</span>
                </>
              )}
            </Show>
          </Show>
        </span>
        <span class="muted">
          <Show when={aiCount() > 0}>
            <span class="ai-tag">AI ×{aiCount()}</span>{" "}
          </Show>
          {clock(ride.status.t)} / {clock(ride.current()?.compiled.totalS ?? 0)}
        </span>
      </div>
      <Canvas draw={draw} tooltip={tooltip} ariaLabel="workout profile" />
    </div>
  );
}
