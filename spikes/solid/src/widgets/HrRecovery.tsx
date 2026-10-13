import { createMemo, Show } from "solid-js";
import type uPlot from "uplot";
import type { HrRecoveryState } from "@neo/shared";
import { backend } from "../backend";
import { axis, tooltipPlugin, UPlot } from "../charts/UPlot";
import { clock } from "../lib/format";
import { createPoll } from "../lib/primitives";
import { ride } from "../state/ride";
import { withAlpha } from "../theme/theme";
import { StatusBadge } from "./chips";
import type { WidgetProps } from "./common";

/** Aligned uPlot data: x = s since block start; observed, fit, predicted mean, lo, hi, target. */
export function recoveryData(st: HrRecoveryState | undefined): uPlot.AlignedData {
  if (!st || st.observed.length === 0) return [[], [], [], [], [], [], []];
  const rows = new Map<number, [number | null, number | null, number | null, number | null, number | null]>();
  const t0 = st.startedT;
  const row = (x: number) => {
    const k = Math.round(x * 4) / 4;
    let r = rows.get(k);
    if (!r) {
      r = [null, null, null, null, null];
      rows.set(k, r);
    }
    return r;
  };
  const f = st.fit;
  for (const s of st.observed) {
    const r = row(s.t - t0);
    r[0] = s.v;
    if (f && s.t >= st.peakT) r[1] = f.hrInf + f.amp * Math.exp(-(s.t - st.peakT) / f.tauS);
  }
  for (const p of st.predicted ?? []) {
    const r = row(p.t - t0);
    r[2] = p.mean;
    r[3] = p.lo;
    r[4] = p.hi;
  }
  const xs = [...rows.keys()].sort((a, b) => a - b);
  const col = (i: number) => xs.map((x) => rows.get(x)?.[i] ?? null);
  return [xs, col(0), col(1), col(2), col(3), col(4), xs.map(() => st.targetBpm)];
}

/** P12 — live HR recovery: observed dots, fitted exponential, dotted prediction + band, target. */
export function HrRecovery(props: WidgetProps) {
  const inRecovery = () => ride.status.phase === "recovery";
  const state = createPoll(
    () => backend.hrRecovery(),
    () => (inRecovery() ? Math.min(Math.max(props.widget.rateHz, 1), 2) : 0.2),
  );
  const data = createMemo(() => recoveryData(state()));
  const slower = () => {
    const f = state()?.fit;
    return f ? f.tauS > f.usualTauS * 1.15 : false;
  };
  const ready = () => state()?.readyInS;

  return (
    <div class="hrrec" data-active={state()?.active === true && (state()?.observed.length ?? 0) > 5}>
      <div class="w-label">
        <span>
          HR recovery <Show when={state() && !state()?.active}>
            <span class="muted">· last block</span>
          </Show>
        </span>
        <span class="legend-inline">
          <span class="chip-dot" /> observed <span class="chip-line" style={{ background: "var(--series-hr)" }} /> fit <span class="chip-dots" /> prediction{" "}
          <span class="chip-dash" /> target {state()?.observed.length ? state()?.targetBpm : "—"}
        </span>
      </div>
      <Show when={(state()?.observed.length ?? 0) > 0} fallback={<div class="empty">Waiting for an HR-guided recovery block…</div>}>
        <div class="hrrec-plot">
          <UPlot
            data={data}
            options={(t) => ({
              legend: { show: false },
              plugins: [tooltipPlugin((x) => `${Math.round(x)} s into block`, (v) => `${Math.round(v)} bpm`, [1, 3])],
              cursor: { y: false, drag: { x: false, y: false }, points: { size: 6 } },
              padding: [10, 12, 0, 0],
              scales: { x: { time: false }, y: { range: (_u, lo, hi) => [Math.floor((lo - 5) / 10) * 10, Math.ceil((hi + 5) / 10) * 10] } },
              axes: [axis(t, { values: (_u, v) => v.map((s) => `${Math.round(s)}s`), size: 22 }), axis(t, { size: 40, values: (_u, v) => v.map((b) => `${b}`) })],
              bands: [{ series: [5, 4], fill: withAlpha(t.hr, 0.16) }],
              series: [
                {},
                { label: "observed", stroke: t.hr, width: 0, points: { show: true, size: 4, fill: t.hr, stroke: t.hr } },
                { label: "fit", stroke: t.hr, width: 2, points: { show: false } },
                { label: "prediction", stroke: t.hr, width: 2, dash: [2, 4], points: { show: false } },
                { label: "lo", stroke: "transparent", points: { show: false } },
                { label: "hi", stroke: "transparent", points: { show: false } },
                { label: "target", stroke: t.target, width: 1.5, dash: [6, 4], points: { show: false } },
              ],
            })}
          />
        </div>
        <div class="hrrec-facts">
          <div class="ready">
            <span class="w-label">Ready in</span>
            <span class="bignum-value small">
              <Show when={ready()} fallback="—">
                {(r) => `≈ ${clock(r().mean)}`}
              </Show>
            </span>
            <Show when={ready()}>
              {(r) => (
                <span class="muted">
                  ({clock(r().lo)}–{clock(r().hi)})
                </span>
              )}
            </Show>
          </div>
          <Show when={state()?.fit}>
            {(f) => (
              <div>
                τ <b>{Math.round(f().tauS)} s</b> <span class="muted">(usual {Math.round(f().usualTauS)} s)</span>{" "}
                <Show when={slower()}>
                  <StatusBadge kind="warning" label="slower" />
                </Show>
              </div>
            )}
          </Show>
          <Show when={state()?.unreachable}>
            <StatusBadge kind="serious" label={`target unreachable — HR settles ≈ ${Math.round(state()?.fit?.hrInf ?? 0)} bpm`} />
          </Show>
        </div>
      </Show>
    </div>
  );
}
