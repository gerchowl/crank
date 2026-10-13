import { useRef, useState, type Dispatch } from "react";
import type { Filter, Layout, MetricId, Widget, WidgetKind } from "@neo/shared";
import { backend } from "../backend";
import { COLS, DEFAULT_SIZE, defaultMetrics, freeSlot, rowsOf, type LayoutAction } from "./model";
import { WidgetView } from "../ride/widgets/WidgetView";
import { info } from "../ride/widgets/metrics";
import { FilterChainEditor } from "./FilterChainEditor";

const KINDS: WidgetKind[] = ["big_number", "number_sparkline", "zone_bar", "log_time_chart", "histogram", "heatmap2d", "mmp_curve", "workout_strip", "hr_recovery", "coach_card"];
const SPANS = [60, 300, 600, 900, 1800, 7200];

interface Drag { id: string; mode: "move" | "resize"; sx: number; sy: number; ox: number; oy: number; x: number; y: number; w: number; h: number }

export function LayoutEditor(props: { layout: Layout; dispatch: Dispatch<LayoutAction>; pageIdx: number; setPageIdx: (i: number) => void; onDone: () => void }) {
  const { layout, dispatch, pageIdx } = props;
  const page = layout.pages[pageIdx] ?? layout.pages[0];
  const rows = rowsOf(layout);
  const [selected, setSelected] = useState<string | null>(page?.widgets[0]?.id ?? null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const [saved, setSaved] = useState<string>("");
  const grid = useRef<HTMLDivElement>(null);
  const [newMetric, setNewMetric] = useState<MetricId>("power_3s");
  const [newKind, setNewKind] = useState<WidgetKind>("big_number");
  if (!page) return null;
  const sel = page.widgets.find((w) => w.id === selected);

  const cell = () => {
    const r = grid.current?.getBoundingClientRect();
    return { cw: (r?.width ?? 1200) / COLS, ch: (r?.height ?? 600) / rows };
  };
  const startDrag = (e: React.PointerEvent, w: Widget, mode: Drag["mode"]) => {
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    setSelected(w.id);
    setDrag({ id: w.id, mode, sx: e.clientX, sy: e.clientY, ox: mode === "move" ? w.x : w.w, oy: mode === "move" ? w.y : w.h, x: w.x, y: w.y, w: w.w, h: w.h });
  };
  const moveDrag = (e: React.PointerEvent) => {
    if (!drag) return;
    const { cw, ch } = cell();
    const dx = Math.round((e.clientX - drag.sx) / cw);
    const dy = Math.round((e.clientY - drag.sy) / ch);
    if (drag.mode === "move") setDrag({ ...drag, x: Math.max(0, Math.min(COLS - drag.w, drag.ox + dx)), y: Math.max(0, Math.min(rows - drag.h, drag.oy + dy)) });
    else setDrag({ ...drag, w: Math.max(1, Math.min(COLS - drag.x, drag.ox + dx)), h: Math.max(1, Math.min(rows - drag.y, drag.oy + dy)) });
  };
  const endDrag = () => {
    if (!drag) return;
    if (drag.mode === "move") dispatch({ type: "move", page: pageIdx, id: drag.id, x: drag.x, y: drag.y });
    else dispatch({ type: "resize", page: pageIdx, id: drag.id, w: drag.w, h: drag.h });
    setDrag(null);
  };
  const update = (patch: Partial<Widget>) => {
    if (sel) dispatch({ type: "update", page: pageIdx, id: sel.id, patch });
  };
  const add = () => {
    const [w, h] = DEFAULT_SIZE[newKind];
    const pos = freeSlot(page, w, h, rows);
    const id = `${newKind}-${String(Date.now() % 1e6)}`;
    dispatch({ type: "add", page: pageIdx, widget: { id, kind: newKind, ...pos, w, h, metrics: defaultMetrics(newKind, newMetric), rateHz: newKind === "big_number" ? 4 : 1 } });
    setSelected(id);
  };

  return (
    <div className="layout-editor">
      <div className="le-toolbar">
        <strong>Editing “{layout.name}” · {page.name}</strong>
        <span className="spacer" />
        <span className="muted small">{saved}</span>
        <button type="button" className="btn" onClick={() => { dispatch({ type: "add_page", name: `Page ${String(layout.pages.length + 1)}` }); props.setPageIdx(layout.pages.length); }}>+ Page</button>
        <button type="button" className="btn" onClick={() => { dispatch({ type: "reset", layout: backend.layouts.reset(layout.id) }); setSaved("reset to default"); }}>Reset to default</button>
        <button type="button" className="btn primary" onClick={() => { backend.layouts.save(layout); setSaved(`saved ${new Date().toLocaleTimeString()}`); }}>Save layout</button>
        <button type="button" className="btn" onClick={props.onDone}>Done</button>
      </div>
      <div className="le-body">
        <div
          ref={grid}
          className="grid editing"
          style={{ gridTemplateRows: `repeat(${String(rows)}, minmax(0, 1fr))` }}
          onPointerMove={moveDrag}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        >
          {page.widgets.map((w) => {
            const d = drag?.id === w.id ? drag : null;
            const g = d ?? w;
            return (
              <div
                key={w.id}
                className={`cell ${w.id === selected ? "selected" : ""} ${d ? "dragging" : ""}`}
                style={{ gridColumn: `${String(g.x + 1)} / span ${String(g.w)}`, gridRow: `${String(g.y + 1)} / span ${String(g.h)}` }}
              >
                <WidgetView widget={w} />
                <div className="edit-overlay" onPointerDown={(e) => { startDrag(e, w, "move"); }}>
                  <span className="edit-label">{w.kind} · {w.metrics.join(", ")} · {w.rateHz} Hz</span>
                  <button type="button" className="icon-btn remove" title="Remove widget" onPointerDown={(e) => { e.stopPropagation(); }} onClick={() => { dispatch({ type: "remove", page: pageIdx, id: w.id }); }}>✕</button>
                  <span className="resize-handle" title="Resize" onPointerDown={(e) => { startDrag(e, w, "resize"); }} />
                </div>
              </div>
            );
          })}
        </div>
        <aside className="le-side">
          <section>
            <h4>Add widget</h4>
            <label>Kind
              <select value={newKind} onChange={(e) => { setNewKind(e.target.value as WidgetKind); }}>
                {KINDS.map((k) => <option key={k} value={k}>{k}</option>)}
              </select>
            </label>
            <label>Metric
              <select value={newMetric} onChange={(e) => { setNewMetric(e.target.value as MetricId); }}>
                {backend.metrics.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
              </select>
            </label>
            <button type="button" className="btn primary" onClick={add}>+ Add</button>
          </section>
          {sel ? (
            <section key={sel.id}>
              <h4>Widget settings</h4>
              <label>Kind
                <select value={sel.kind} onChange={(e) => { const kind = e.target.value as WidgetKind; update({ kind, metrics: defaultMetrics(kind, sel.metrics[0] ?? "power") }); }}>
                  {KINDS.map((k) => <option key={k} value={k}>{k}</option>)}
                </select>
              </label>
              {sel.metrics.map((m, i) => (
                <label key={i}>{i === 0 ? "Metric" : sel.kind === "heatmap2d" ? "Y metric" : "Series / target"}
                  <select value={m} onChange={(e) => { update({ metrics: sel.metrics.map((x, k) => (k === i ? (e.target.value as MetricId) : x)) }); }}>
                    {backend.metrics.map((mm) => <option key={mm.id} value={mm.id}>{mm.label}</option>)}
                  </select>
                </label>
              ))}
              {(sel.kind === "big_number" || sel.kind === "zone_bar" || sel.kind === "log_time_chart") && sel.metrics.length < (sel.kind === "log_time_chart" ? 4 : 2) && (
                <button type="button" className="btn small" onClick={() => { update({ metrics: [...sel.metrics, sel.kind === "log_time_chart" ? "hr" : "target_power"] }); }}>
                  + {sel.kind === "log_time_chart" ? "series" : "target"}
                </button>
              )}
              <label>Display rate <b>{sel.rateHz} Hz</b>
                <input
                  type="range" min={-1} max={1} step={0.05}
                  value={Math.log10(sel.rateHz)}
                  onChange={(e) => { update({ rateHz: Math.round(Math.pow(10, Number(e.target.value)) * 10) / 10 || 0.1 }); }}
                />
                <span className="muted small">source {info(sel.metrics[0] ?? "power").sourceHz} Hz; more adds nothing</span>
              </label>
              <FilterChainEditor value={sel.filter ?? []} onChange={(f: Filter[]) => { update(f.length ? { filter: f } : { filter: [] }); }} />
              <label className="check">
                <input type="checkbox" checked={!!sel.colorByZone} onChange={(e) => { update({ colorByZone: e.target.checked }); }} /> Colour by zone
              </label>
              {(sel.kind === "log_time_chart" || sel.kind === "number_sparkline" || sel.kind === "workout_strip") && (
                <label>Chart span
                  <select value={sel.spanS ?? 0} onChange={(e) => { update({ spanS: Number(e.target.value) }); }}>
                    <option value={0}>default</option>
                    {SPANS.map((s) => <option key={s} value={s}>{s >= 3600 ? `${String(s / 3600)} h` : `${String(s / 60)} min`}</option>)}
                  </select>
                </label>
              )}
              {sel.kind === "log_time_chart" && (
                <label>Time scale
                  <select value={sel.scale ?? "log"} onChange={(e) => { update({ scale: e.target.value === "fisheye" ? "fisheye" : "log" }); }}>
                    <option value="log">log</option>
                    <option value="fisheye">fisheye</option>
                  </select>
                </label>
              )}
              <p className="muted small">x {sel.x} y {sel.y} · {sel.w}×{sel.h} cells</p>
            </section>
          ) : (
            <p className="muted">Select a widget to edit it.</p>
          )}
        </aside>
      </div>
    </div>
  );
}
