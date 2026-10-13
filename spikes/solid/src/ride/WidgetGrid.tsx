import { type Component, createMemo, createSignal, For, Show } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { Page, Widget, WidgetKind } from "@neo/shared";
import { GRID_COLS, layoutState } from "../state/layout";
import { BigNumber } from "../widgets/BigNumber";
import { CoachCard } from "../widgets/CoachCard";
import type { WidgetProps } from "../widgets/common";
import { Heatmap } from "../widgets/Heatmap";
import { Histogram } from "../widgets/Histogram";
import { HrRecovery } from "../widgets/HrRecovery";
import { LogTimeChart } from "../widgets/LogTimeChart";
import { MmpCurve } from "../widgets/MmpCurve";
import { NumberSparkline } from "../widgets/NumberSparkline";
import { WorkoutStrip } from "../widgets/WorkoutStrip";
import { ZoneBar } from "../widgets/ZoneBar";

export const WIDGETS: Record<WidgetKind, Component<WidgetProps>> = {
  big_number: BigNumber,
  number_sparkline: NumberSparkline,
  zone_bar: ZoneBar,
  log_time_chart: LogTimeChart,
  histogram: Histogram,
  heatmap2d: Heatmap,
  mmp_curve: MmpCurve,
  workout_strip: WorkoutStrip,
  hr_recovery: HrRecovery,
  coach_card: CoachCard,
};

interface Drag {
  id: string;
  mode: "move" | "resize";
  startX: number;
  startY: number;
  orig: { x: number; y: number; w: number; h: number };
  cellW: number;
  cellH: number;
}

/** 12-column widget grid for one page; in edit mode widgets can be dragged and resized. */
export function WidgetGrid(props: { page: Page; editing: boolean }) {
  const [grid, setGrid] = createSignal<HTMLDivElement>();
  const rows = createMemo(() => Math.max(layoutState.device() === "tv" ? 6 : 8, ...props.page.widgets.map((w) => w.y + w.h)));
  let drag: Drag | null = null;

  const begin = (e: PointerEvent, w: Widget, mode: Drag["mode"]) => {
    const g = grid();
    if (!props.editing || !g) return;
    e.preventDefault();
    e.stopPropagation();
    layoutState.actions.select(w.id);
    const r = g.getBoundingClientRect();
    drag = { id: w.id, mode, startX: e.clientX, startY: e.clientY, orig: { x: w.x, y: w.y, w: w.w, h: w.h }, cellW: r.width / GRID_COLS, cellH: r.height / rows() };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };
  const move = (e: PointerEvent) => {
    if (!drag) return;
    const dx = Math.round((e.clientX - drag.startX) / drag.cellW);
    const dy = Math.round((e.clientY - drag.startY) / drag.cellH);
    const o = drag.orig;
    const next =
      drag.mode === "move"
        ? { x: Math.max(0, Math.min(GRID_COLS - o.w, o.x + dx)), y: Math.max(0, o.y + dy), w: o.w, h: o.h }
        : { x: o.x, y: o.y, w: Math.max(1, Math.min(GRID_COLS - o.x, o.w + dx)), h: Math.max(1, o.h + dy) };
    const cur = props.page.widgets.find((w) => w.id === drag?.id);
    if (cur && (cur.x !== next.x || cur.y !== next.y || cur.w !== next.w || cur.h !== next.h)) layoutState.actions.updateWidget(drag.id, next);
  };
  const end = () => {
    drag = null;
  };

  return (
    <div
      class="widget-grid"
      classList={{ editing: props.editing }}
      ref={setGrid}
      style={{ "grid-template-columns": `repeat(${GRID_COLS}, 1fr)`, "grid-template-rows": `repeat(${rows()}, minmax(0, 1fr))` }}
    >
      <For each={props.page.widgets}>
        {(w) => (
          <section
            class={`widget kind-${w.kind}`}
            classList={{ selected: props.editing && layoutState.selected() === w.id }}
            style={{ "grid-column": `${w.x + 1} / span ${w.w}`, "grid-row": `${w.y + 1} / span ${w.h}` }}
            data-widget={w.id}
          >
            <Dynamic component={WIDGETS[w.kind]} widget={w} editing={props.editing} />
            <Show when={props.editing}>
              <div class="edit-overlay" onPointerDown={(e) => begin(e, w, "move")} onPointerMove={move} onPointerUp={end} onPointerCancel={end}>
                <span class="edit-tag">
                  {w.kind} · {w.metrics.join(", ") || "—"} · {w.rateHz} Hz
                </span>
                <button
                  class="edit-remove"
                  title="Remove widget"
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={() => layoutState.actions.removeWidget(w.id)}
                >
                  ✕
                </button>
                <div class="edit-resize" title="Resize" onPointerDown={(e) => begin(e, w, "resize")} onPointerMove={move} onPointerUp={end} />
              </div>
            </Show>
          </section>
        )}
      </For>
    </div>
  );
}
