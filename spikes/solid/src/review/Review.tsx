import { createMemo, For, Show } from "solid-js";
import { createStore } from "solid-js/store";
import type uPlot from "uplot";
import type { CoachNudge, FlatBlock, Widget } from "@neo/shared";
import { backend } from "../backend";
import { Canvas, type Hover } from "../charts/Canvas";
import { axis, tooltipPlugin, UPlot } from "../charts/UPlot";
import { clock, formatValue } from "../lib/format";
import { ride } from "../state/ride";
import { router } from "../state/router";
import type { ThemeTokens } from "../theme/theme";
import { BandChip, StatusBadge, type StatusKind } from "../widgets/chips";
import { Heatmap } from "../widgets/Heatmap";
import { MmpCurve } from "../widgets/MmpCurve";
import { TableView } from "../widgets/TableView";
import { LinearPanels } from "./LinearPanels";
import { createRideRecord, lastValue, mean, type RideRecord } from "./streams";

const VERDICT: Record<CoachNudge["verdict"], string> = { ease_off: "ease off", hold: "hold", push: "push", replan: "replan" };
const STATUS: Record<CoachNudge["status"], [StatusKind, string]> = {
  pending: ["warning", "pending"],
  auto_applied: ["good", "auto"],
  accepted: ["good", "accepted"],
  declined: ["serious", "declined"],
  expired: ["serious", "expired"],
};

const asWidget = (id: string, kind: Widget["kind"], metrics: Widget["metrics"]): Widget => ({ id, kind, x: 0, y: 0, w: 4, h: 3, metrics, rateHz: 0.2, colorByZone: true });

/** V5 — post-ride review. */
export function Review() {
  const record = createRideRecord();
  const blocks = () => ride.current()?.compiled.blocks ?? [];
  const decisions = () => ride.nudges;
  const tauMean = createMemo(() => {
    const rs = record().recoveries.filter((r) => r.fit);
    return rs.length ? rs.reduce((s, r) => s + (r.fit?.tauS ?? 0), 0) / rs.length : null;
  });
  const tile = (m: "np" | "if" | "tss" | "kj" | "decoupling" | "compliance") => lastValue(record().series[m]);

  return (
    <div class="page review">
      <header class="review-head">
        <h1>Ride review</h1>
        <span class="muted">{ride.current()?.workout.name}</span>
        <Show when={ride.status.state !== "finished"}>
          <span class="muted small">(ride {ride.status.state} — updating live)</span>
          <button onClick={() => router.go("ride")}>Back to ride</button>
        </Show>
      </header>

      <div class="tiles">
        <Tile label="Duration" value={clock(record().len)} />
        <Tile label="Work" value={formatValue("kj", tile("kj"))} unit="kJ" />
        <Tile label="NP" value={formatValue("np", tile("np"))} unit="W" />
        <Tile label="IF" value={formatValue("if", tile("if"))} />
        <Tile label="TSS" value={formatValue("tss", tile("tss"))} />
        <Tile label="Decoupling" value={formatValue("decoupling", tile("decoupling"))} unit="%" />
        <Tile label="Compliance" value={formatValue("compliance", tile("compliance"))} unit="%" />
        <Tile label="τ̄ recovery" value={tauMean() === null ? "—" : Math.round(tauMean() ?? 0).toString()} unit="s" />
      </div>

      <LinearPanels record={record} blocks={blocks()} nudges={decisions()} />

      <Decisions />

      <div class="review-grid">
        <div class="card chart-card">
          <MmpCurve widget={asWidget("r-mmp", "mmp_curve", ["power"])} editing={false} />
        </div>
        <div class="card chart-card">
          <Heatmap widget={asWidget("r-heat", "heatmap2d", ["power", "hr"])} editing={false} />
        </div>
        <div class="card chart-card">
          <FatigueScatter record={record()} />
        </div>
        <div class="card chart-card">
          <TauPerBlock record={record()} />
        </div>
      </div>

      <Laps record={record()} blocks={blocks()} />
    </div>
  );
}

function Tile(props: { label: string; value: string; unit?: string }) {
  return (
    <div class="tile">
      <span class="stat-label">{props.label}</span>
      <span class="stat-value">
        {props.value} <span class="muted small">{props.unit ?? ""}</span>
      </span>
    </div>
  );
}

/** P20 — coach decisions: time, verdict, band, status, 👍/👎 rating (coach.rate). */
function Decisions() {
  const [ratings, setRatings] = createStore<Record<string, "right" | "wrong">>({});
  const rate = (n: CoachNudge, r: "right" | "wrong") => {
    backend.coach.rate(n.callId, r, r === "right");
    setRatings(n.callId, r);
  };
  return (
    <div class="card">
      <h2>Coach decisions ({ride.nudges.length})</h2>
      <Show when={ride.nudges.length} fallback={<p class="muted">No coach decisions in this ride yet.</p>}>
        <ul class="decisions">
          <For each={ride.nudges}>
            {(n) => (
              <li class="decision">
                <span class="dec-time">◆ {clock(n.t)}</span>
                <span class="dec-verdict">
                  {VERDICT[n.verdict]}
                  {n.proposedPctChange ? ` ${n.proposedPctChange > 0 ? "+" : ""}${n.proposedPctChange}%` : ""}
                </span>
                <BandChip band={n.band} p={n.p} />
                <StatusBadge kind={STATUS[n.status][0]} label={STATUS[n.status][1]} />
                <span class="dec-msg">{n.message}</span>
                <span class="dec-rate" role="group" aria-label="Rate this decision">
                  <button class="mini" classList={{ on: ratings[n.callId] === "right" }} onClick={() => rate(n, "right")} aria-pressed={ratings[n.callId] === "right"} title="Good call">
                    👍
                  </button>
                  <button class="mini" classList={{ on: ratings[n.callId] === "wrong" }} onClick={() => rate(n, "wrong")} aria-pressed={ratings[n.callId] === "wrong"} title="Bad call">
                    👎
                  </button>
                </span>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}

/** P16 — power × HR points coloured by kJ spent (sequential fatigue ramp); density fallback > 5k. */
function FatigueScatter(props: { record: RideRecord }) {
  const pts = createMemo(() => {
    const p = props.record.series["power"] ?? [];
    const h = props.record.series["hr"] ?? [];
    const k = props.record.series["kj"] ?? [];
    const out: { p: number; h: number; k: number }[] = [];
    for (let i = 0; i < p.length; i++) {
      const pv = p[i];
      const hv = h[i];
      if (pv != null && hv != null) out.push({ p: pv, h: hv, k: k[i] ?? 0 });
    }
    return out;
  });
  const maxK = createMemo(() => Math.max(1, ...pts().map((x) => x.k)));
  const PAD = { l: 34, r: 8, t: 8, b: 20 };
  const draw = (ctx: CanvasRenderingContext2D, w: number, h: number, t: ThemeTokens) => {
    paintScatter(ctx, w, h, t, pts(), maxK(), PAD);
  };
  const tooltip = (p: Hover, w: number, h: number) => {
    // nearest point within 12 px (per-mark tooltip)
    const X = (v: number) => PAD.l + (Math.min(v, 500) / 500) * (w - PAD.l - PAD.r);
    const Y = (b: number) => PAD.t + (1 - (b - 80) / 120) * (h - PAD.t - PAD.b);
    let best: { p: number; h: number; k: number } | null = null;
    let bd = 144;
    for (const pt of pts()) {
      const d = (X(pt.p) - p.x) ** 2 + (Y(pt.h) - p.y) ** 2;
      if (d < bd) {
        bd = d;
        best = pt;
      }
    }
    if (!best) return null;
    return (
      <>
        <b>
          {Math.round(best.p)} W · {Math.round(best.h)} bpm
        </b>
        <div class="muted">after {Math.round(best.k)} kJ</div>
      </>
    );
  };
  return (
    <div class="chart-widget">
      <div class="w-label">
        <span>Power × HR by fatigue (kJ)</span>
        <span class="legend-inline">
          <For each={[1, 2, 3, 4, 5]}>{(i) => <span class="fatigue-swatch" style={{ background: `var(--fatigue-${i})` }} />}</For>
          <span class="muted">0 → {Math.round(maxK())} kJ</span>
        </span>
      </div>
      <Canvas draw={draw} tooltip={tooltip} ariaLabel="power by heart rate scatter coloured by kJ" />
    </div>
  );
}

function paintScatter(ctx: CanvasRenderingContext2D, w: number, h: number, t: ThemeTokens, pts: { p: number; h: number; k: number }[], maxK: number, PAD: { l: number; r: number; t: number; b: number }) {
  const X = (p: number) => PAD.l + (Math.min(p, 500) / 500) * (w - PAD.l - PAD.r);
  const Y = (b: number) => PAD.t + (1 - (b - 80) / 120) * (h - PAD.t - PAD.b);
  ctx.font = `11px ${t.font}`;
  ctx.strokeStyle = t.grid;
  ctx.fillStyle = t.muted;
  for (let p = 0; p <= 500; p += 100) {
    const x = Math.round(X(p)) + 0.5;
    ctx.beginPath();
    ctx.moveTo(x, PAD.t);
    ctx.lineTo(x, h - PAD.b);
    ctx.stroke();
    ctx.fillText(`${p}`, x - 8, h - 5);
  }
  for (let b = 80; b <= 200; b += 40) {
    const y = Math.round(Y(b)) + 0.5;
    ctx.beginPath();
    ctx.moveTo(PAD.l, y);
    ctx.lineTo(w - PAD.r, y);
    ctx.stroke();
    ctx.fillText(`${b}`, 4, y + 4);
  }
  const density = pts.length > 5000;
  const r = density ? 1.5 : 2.5;
  for (const pt of pts) {
    const q = Math.min(4, Math.floor((pt.k / maxK) * 5));
    ctx.fillStyle = t.fatigue[q] ?? t.ink2;
    ctx.globalAlpha = density ? 0.25 : 0.8;
    ctx.fillRect(X(pt.p) - r, Y(pt.h) - r, r * 2, r * 2);
  }
  ctx.globalAlpha = 1;
}

/** P12 per block — τ of each finished recovery block, usual τ as reference. */
function TauPerBlock(props: { record: RideRecord }) {
  const data = createMemo((): uPlot.AlignedData => {
    const rs = props.record.recoveries.filter((r) => r.fit);
    return [rs.map((_, i) => i + 1), rs.map((r) => r.fit?.tauS ?? null), rs.map((r) => r.fit?.usualTauS ?? null)];
  });
  return (
    <div class="chart-widget">
      <div class="w-label">
        <span>HR recovery τ per block</span>
        <span class="legend-inline">
          <span class="chip-dot" /> τ <span class="chip-dash" /> usual τ
        </span>
      </div>
      <Show when={props.record.recoveries.length} fallback={<div class="empty">No finished HR-recovery blocks yet.</div>}>
        <UPlot
          data={data}
          options={(t) => ({
            legend: { show: false },
            plugins: [tooltipPlugin((x) => `recovery #${x}`, (v) => `${Math.round(v)} s`)],
            cursor: { y: false, drag: { x: false, y: false } },
            padding: [8, 12, 0, 0],
            scales: { x: { time: false, range: (_u, lo, hi) => [lo - 0.5, hi + 0.5] }, y: { range: (_u, _lo, hi) => [0, Math.ceil((hi + 10) / 10) * 10] } },
            axes: [axis(t, { size: 22, splits: (u) => (u.data[0] as number[]).slice(), values: (_u, v) => v.map((x) => `#${x}`) }), axis(t, { size: 40, values: (_u, v) => v.map((x) => `${x}s`) })],
            series: [{}, { label: "τ", stroke: t.hr, width: 0, points: { show: true, size: 10, fill: t.hr, stroke: t.surface1, width: 2 } }, { label: "usual", stroke: t.target, width: 1.5, dash: [6, 4], points: { show: false } }],
          })}
        />
      </Show>
    </div>
  );
}

/** Laps table — per compiled block: target, avg power, HR, cadence, compliance, τ. */
function Laps(props: { record: RideRecord; blocks: FlatBlock[] }) {
  const rows = createMemo(() => {
    const r = props.record;
    const power = r.series["power"];
    return props.blocks
      .filter((b) => b.startS < r.len)
      .map((b, i) => {
        const from = b.startS;
        const to = Math.min(r.len, b.startS + b.durS);
        const target = (b.powerFrom + b.powerTo) / 2;
        let ok = 0;
        let n = 0;
        for (let s = Math.ceil(from + 5); s < to; s++) {
          const v = power?.[s];
          if (v == null) continue;
          n++;
          if (Math.abs(v - target) <= target * 0.05) ok++;
        }
        const tau = r.recoveries.find((x) => Math.abs(x.startedT - b.startS) < Math.max(30, b.durS))?.fit?.tauS;
        return [
          `${i + 1} · ${b.phase}${b.aiChanged ? " · AI" : ""}`,
          clock(b.durS),
          Math.round(target),
          Math.round(mean(power, from, to) ?? 0),
          Math.round(mean(r.series["hr"], from, to) ?? 0),
          Math.round(mean(r.series["cadence"], from, to) ?? 0),
          n ? `${Math.round((ok / n) * 100)}%` : "—",
          tau ? `${Math.round(tau)} s` : "",
        ];
      });
  });
  return (
    <div class="card laps">
      <h2>Laps</h2>
      <TableView caption="per block (expected timing)" head={["block", "dur", "target W", "avg W", "avg HR", "cad", "compliance", "τ"]} rows={rows()} />
    </div>
  );
}
