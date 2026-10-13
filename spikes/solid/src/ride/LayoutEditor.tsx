import { createMemo, For, Index, Match, Show, Switch } from "solid-js";
import type { Filter, Layout, MetricId, Phase, Widget, WidgetKind } from "@neo/shared";
import { backend } from "../backend";
import { layoutState, type WidgetPatch } from "../state/layout";
import { FILTER_PRESETS, filterLabel } from "../widgets/common";

const KINDS: { kind: WidgetKind; label: string }[] = [
  { kind: "big_number", label: "Big number" },
  { kind: "number_sparkline", label: "Number + sparkline" },
  { kind: "zone_bar", label: "Zone bar" },
  { kind: "log_time_chart", label: "Log-time chart" },
  { kind: "histogram", label: "Histogram" },
  { kind: "heatmap2d", label: "Heatmap 2D" },
  { kind: "mmp_curve", label: "MMP curve" },
  { kind: "workout_strip", label: "Workout strip" },
  { kind: "hr_recovery", label: "HR recovery" },
  { kind: "coach_card", label: "Coach card" },
];
const PHASES: Phase[] = ["warmup", "work", "recovery", "cooldown", "free"];
const DEVICES: Layout["device"][] = ["laptop", "tablet", "tv"];
const SPANS = [60, 300, 600, 900, 1800, 3600, 7200];
const MULTI: ReadonlySet<WidgetKind> = new Set(["log_time_chart"]);
const RATE_STEPS = [0.1, 0.2, 0.5, 1, 2, 4, 10];

/** V9 — layout editor side panel: page settings, widget palette, per-widget settings. */
export function LayoutEditor() {
  const L = layoutState;
  const selected = createMemo(() => L.page()?.widgets.find((w) => w.id === L.selected()));

  return (
    <aside class="layout-editor" aria-label="Layout editor">
      <div class="le-section">
        <h3>Layout</h3>
        <div class="seg">
          <For each={DEVICES}>
            {(d) => (
              <button classList={{ on: L.device() === d }} onClick={() => L.actions.loadDevice(d)}>
                {d}
              </button>
            )}
          </For>
        </div>
        <div class="le-actions">
          <button class="primary" disabled={!L.dirty()} onClick={() => L.actions.save()}>
            Save
          </button>
          <button onClick={() => L.actions.discard()} disabled={!L.dirty()}>
            Discard
          </button>
          <button onClick={() => L.actions.reset()} title="Back to the built-in default">
            Reset to default
          </button>
        </div>
        <Show when={L.dirty()}>
          <p class="muted small">Unsaved changes</p>
        </Show>
      </div>

      <Show when={L.page()}>
        {(page) => (
          <div class="le-section">
            <h3>Page</h3>
            <input class="text" value={page().name} onInput={(e) => L.actions.renamePage(e.currentTarget.value)} aria-label="Page name" />
            <div class="le-phases">
              <span class="muted small">auto-show in</span>
              <For each={PHASES}>
                {(ph) => (
                  <label class="chip-toggle">
                    <input type="checkbox" checked={page().autoPhase?.includes(ph) ?? false} onChange={() => L.actions.togglePhase(ph)} />
                    {ph}
                  </label>
                )}
              </For>
            </div>
            <div class="le-actions">
              <button onClick={() => L.actions.addPage()}>+ Page</button>
              <button onClick={() => L.actions.removePage()} disabled={L.layout.pages.length <= 1}>
                Delete page
              </button>
            </div>
          </div>
        )}
      </Show>

      <div class="le-section">
        <h3>Add widget</h3>
        <div class="palette">
          <For each={KINDS}>
            {(k) => (
              <button class="palette-item" onClick={() => L.actions.addWidget(k.kind)}>
                + {k.label}
              </button>
            )}
          </For>
        </div>
      </div>

      <Show when={selected()} fallback={<p class="muted le-section">Select a widget to edit its settings. Drag to move, corner to resize.</p>}>
        {(w) => <WidgetSettings widget={w()} />}
      </Show>
    </aside>
  );
}

function WidgetSettings(props: { widget: Widget }) {
  const update = (patch: WidgetPatch) => layoutState.actions.updateWidget(props.widget.id, patch);
  const metrics = backend.metrics;
  const rateIdx = () => {
    const r = props.widget.rateHz;
    let best = 0;
    RATE_STEPS.forEach((s, i) => {
      if (Math.abs(Math.log(s / r)) < Math.abs(Math.log((RATE_STEPS[best] ?? 1) / r))) best = i;
    });
    return best;
  };
  const setMetric = (i: number, m: MetricId) => {
    const next = [...props.widget.metrics];
    next[i] = m;
    update({ metrics: next });
  };
  const toggleMetric = (m: MetricId) => {
    const cur = props.widget.metrics;
    update({ metrics: cur.includes(m) ? cur.filter((x) => x !== m) : [...cur, m] });
  };
  const filters = () => props.widget.filter ?? [];
  const setFilters = (f: Filter[]) => update({ filter: f.length ? f : undefined });

  return (
    <div class="le-section widget-settings">
      <h3>Widget · {props.widget.id}</h3>
      <label class="field">
        <span>Kind</span>
        <select value={props.widget.kind} onChange={(e) => update({ kind: e.currentTarget.value as WidgetKind })}>
          <For each={KINDS}>{(k) => <option value={k.kind}>{k.label}</option>}</For>
        </select>
      </label>

      <Switch>
        <Match when={MULTI.has(props.widget.kind)}>
          <div class="field">
            <span>Series</span>
            <div class="chips">
              <For each={["power", "hr", "cadence", "target_power"] as MetricId[]}>
                {(m) => (
                  <label class="chip-toggle">
                    <input type="checkbox" checked={props.widget.metrics.includes(m)} onChange={() => toggleMetric(m)} />
                    {m}
                  </label>
                )}
              </For>
            </div>
          </div>
        </Match>
        <Match when={props.widget.kind !== "coach_card" && props.widget.kind !== "workout_strip"}>
          <Index each={props.widget.kind === "heatmap2d" || props.widget.kind === "big_number" || props.widget.kind === "zone_bar" ? [0, 1] : [0]}>
            {(i) => (
              <label class="field">
                <span>{i() === 0 ? (props.widget.kind === "heatmap2d" ? "X metric" : "Metric") : props.widget.kind === "heatmap2d" ? "Y metric" : "Target (Δ)"}</span>
                <select value={props.widget.metrics[i()] ?? ""} onChange={(e) => setMetric(i(), e.currentTarget.value as MetricId)}>
                  <Show when={i() > 0}>
                    <option value="">—</option>
                  </Show>
                  <For each={metrics}>{(m) => <option value={m.id}>{m.label}</option>}</For>
                </select>
              </label>
            )}
          </Index>
        </Match>
      </Switch>

      <label class="field">
        <span>Display rate · {props.widget.rateHz} Hz</span>
        <input type="range" min="0" max={RATE_STEPS.length - 1} step="1" value={rateIdx()} onInput={(e) => update({ rateHz: RATE_STEPS[Number(e.currentTarget.value)] ?? 1 })} />
      </label>

      <div class="field">
        <span>Filter chain · {filterLabel(props.widget.filter)}</span>
        <div class="chips">
          <For each={FILTER_PRESETS}>
            {(p) => (
              <button class="mini" classList={{ on: filterLabel(p.filter) === filterLabel(props.widget.filter) }} onClick={() => setFilters(p.filter)}>
                {p.name}
              </button>
            )}
          </For>
        </div>
        <Index each={filters()}>
          {(f, i) => (
            <div class="filter-row">
              <select
                value={f().kind}
                onChange={(e) => {
                  const k = e.currentTarget.value as Filter["kind"];
                  const def: Filter =
                    k === "ema" ? { kind: "ema", tauS: 3 } : k === "sma" ? { kind: "sma", windowS: 5 } : k === "median" ? { kind: "median", n: 5 } : k === "one_euro" ? { kind: "one_euro", minCutoff: 1, beta: 0.01 } : { kind: "hold" };
                  setFilters(filters().map((x, j) => (j === i ? def : x)));
                }}
              >
                <option value="ema">EMA</option>
                <option value="sma">SMA</option>
                <option value="median">median</option>
                <option value="one_euro">1€</option>
                <option value="hold">hold</option>
              </select>
              <FilterParam filter={f()} onChange={(nf) => setFilters(filters().map((x, j) => (j === i ? nf : x)))} />
              <button class="mini" onClick={() => setFilters(filters().filter((_, j) => j !== i))} title="Remove stage">
                ✕
              </button>
            </div>
          )}
        </Index>
        <button class="mini" onClick={() => setFilters([...filters(), { kind: "ema", tauS: 3 }])}>
          + stage
        </button>
      </div>

      <label class="toggle">
        <input type="checkbox" checked={props.widget.colorByZone ?? false} onChange={(e) => update({ colorByZone: e.currentTarget.checked || undefined })} />
        Colour by zone
      </label>

      <Show when={["log_time_chart", "number_sparkline", "workout_strip"].includes(props.widget.kind)}>
        <label class="field">
          <span>Span</span>
          <select value={String(props.widget.spanS ?? "")} onChange={(e) => update({ spanS: e.currentTarget.value ? Number(e.currentTarget.value) : undefined })}>
            <option value="">default</option>
            <For each={SPANS}>{(s) => <option value={s}>{s >= 60 ? `${s / 60} min` : `${s} s`}</option>}</For>
          </select>
        </label>
      </Show>
      <Show when={props.widget.kind === "log_time_chart"}>
        <label class="field">
          <span>Time scale</span>
          <select value={props.widget.scale ?? "log"} onChange={(e) => update({ scale: e.currentTarget.value as "log" | "fisheye" })}>
            <option value="log">log</option>
            <option value="fisheye">fisheye</option>
          </select>
        </label>
      </Show>
      <button class="danger" onClick={() => layoutState.actions.removeWidget(props.widget.id)}>
        Remove widget
      </button>
    </div>
  );
}

function FilterParam(props: { filter: Filter; onChange: (f: Filter) => void }) {
  const num = (v: string) => Math.max(0, Number(v) || 0);
  return (
    <Switch>
      <Match when={props.filter.kind === "ema" && props.filter}>
        {(f) => <input class="num" type="number" min="0.1" step="0.5" value={f().tauS} onChange={(e) => props.onChange({ kind: "ema", tauS: num(e.currentTarget.value) })} title="τ (s)" />}
      </Match>
      <Match when={props.filter.kind === "sma" && props.filter}>
        {(f) => <input class="num" type="number" min="1" step="1" value={f().windowS} onChange={(e) => props.onChange({ kind: "sma", windowS: num(e.currentTarget.value) })} title="window (s)" />}
      </Match>
      <Match when={props.filter.kind === "median" && props.filter}>
        {(f) => <input class="num" type="number" min="1" step="2" value={f().n} onChange={(e) => props.onChange({ kind: "median", n: num(e.currentTarget.value) })} title="n" />}
      </Match>
      <Match when={props.filter.kind === "one_euro" && props.filter}>
        {(f) => (
          <>
            <input class="num" type="number" step="0.1" value={f().minCutoff} onChange={(e) => props.onChange({ kind: "one_euro", minCutoff: num(e.currentTarget.value), beta: f().beta })} title="min cutoff" />
            <input class="num" type="number" step="0.005" value={f().beta} onChange={(e) => props.onChange({ kind: "one_euro", minCutoff: f().minCutoff, beta: num(e.currentTarget.value) })} title="β" />
          </>
        )}
      </Match>
    </Switch>
  );
}
