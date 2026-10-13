import { useMemo, useState } from "react";
import type { MetricId } from "@neo/shared";
import { backend } from "../backend";
import { useMetric, useNudges, useRideWorkout } from "../lib/live";
import { fmtValue, mmss } from "../lib/format";
import { go } from "../lib/router";
import { info } from "../ride/widgets/metrics";
import { MmpWidget } from "../ride/widgets/MmpWidget";
import { HeatmapWidget } from "../ride/widgets/HeatmapWidget";
import { HistogramWidget } from "../ride/widgets/HistogramWidget";
import { TableView } from "../ride/widgets/TableView";
import { RideTimeline } from "./RideTimeline";
import { FatigueScatter, TauChart } from "./Charts";
import { mean, useRideRecord } from "./record";

function Stat({ metric, label }: { metric: MetricId; label?: string }) {
  const m = info(metric);
  const v = useMetric(metric, 0.5)?.v ?? null;
  return (
    <div className="stat">
      <span className="label">{label ?? m.label}</span>
      <b>{fmtValue(v, m.unit, m.id)}</b>
      <span className="muted small">{m.unit === "s" ? "" : m.unit}</span>
    </div>
  );
}

const VERDICT: Record<string, string> = { ease_off: "Ease off", hold: "Hold", push: "Push", replan: "Replan" };

/** V5 post-ride review (minimal): summary, P05 + P20, decisions with 👍/👎, P09, P07, P06, P16, P12, laps. */
export function Review() {
  const rec = useRideRecord();
  const cur = useRideWorkout();
  const nudges = useNudges();
  const [rated, setRated] = useState<Record<string, "right" | "wrong">>({});
  const blocks = useMemo(() => cur?.compiled.blocks ?? [], [cur]);
  const recoveries = useMemo(() => (rec.toS > 0 ? backend.recoveries() : []), [rec.toS]);
  const decisions = useMemo(() => (nudges.length ? backend.coach.decisions() : []), [nudges]);
  const tauMean = mean(recoveries.map((r) => r.fit?.tauS));

  const laps = useMemo(() => {
    const bi = rec.series["block_index"] ?? [];
    const out: { idx: number; from: number; to: number }[] = [];
    bi.forEach((v, t) => {
      if (v === null) return;
      const last = out[out.length - 1];
      if (last?.idx === v) last.to = t + 1;
      else out.push({ idx: v, from: t, to: t + 1 });
    });
    return out.map((l) => {
      const b = blocks[l.idx - 1];
      const sl = (m: string) => (rec.series[m] ?? []).slice(l.from, l.to);
      const P = sl("power");
      const T = sl("target_power");
      let ok = 0;
      let n = 0;
      P.forEach((p, i) => {
        const tg = T[i];
        if (p == null || tg == null || tg <= 0) return;
        n++;
        if (Math.abs(p - tg) / tg <= 0.05) ok++;
      });
      const r = recoveries.find((x) => x.startedT >= l.from - 1 && x.startedT < l.to);
      return [
        l.idx,
        mmss(l.from),
        mmss(l.to - l.from),
        b ? `${String(Math.round(b.powerFrom))}${b.powerTo !== b.powerFrom ? `→${String(Math.round(b.powerTo))}` : ""} W` : "—",
        fmtValue(mean(P), "W", "power"),
        fmtValue(mean(sl("hr")), "bpm", "hr"),
        fmtValue(mean(sl("cadence")), "rpm", "cadence"),
        n && b?.phase === "work" ? `${String(Math.round((ok / n) * 100))}%` : "—",
        r?.fit ? `${String(Math.round(r.fit.tauS))} s` : "",
      ];
    });
  }, [rec, blocks, recoveries]);

  return (
    <div className="review">
      <header className="rv-summary">
        <h2>{cur?.workout.name ?? "Ride"}</h2>
        <div className="stats">
          <div className="stat"><span className="label">Duration</span><b>{mmss(rec.toS)}</b></div>
          <Stat metric="kj" label="Work" />
          <Stat metric="np" label="NP" />
          <Stat metric="if" label="IF" />
          <Stat metric="tss" label="TSS" />
          <Stat metric="decoupling" label="Decoupling" />
          <Stat metric="compliance" label="Compliance" />
          <div className="stat"><span className="label">τ̄ recovery</span><b>{tauMean === null ? "—" : Math.round(tauMean)}</b><span className="muted small">s</span></div>
        </div>
        <button type="button" className="btn" onClick={() => { go("ride"); }}>Back to ride</button>
      </header>

      <RideTimeline rec={rec} blocks={blocks} nudges={decisions} />

      <section className="rv-card">
        <header className="rv-head"><h3>Coach decisions</h3><span className="muted small">rate each call: was the coach right?</span></header>
        {decisions.length === 0 ? (
          <p className="muted pad">No coach decisions in this ride yet.</p>
        ) : (
          <ul className="decisions">
            {decisions.map((d) => (
              <li key={d.callId}>
                <span className="t">{mmss(d.t)}</span>
                <span>{d.status === "declined" || d.status === "expired" ? "◇" : "◆"}</span>
                <b>{VERDICT[d.verdict]}</b>
                <span className={`band band-${d.band}`}>{d.band}</span>
                <span className="status">{d.status.replace("_", " ")}</span>
                <span className="msg">{d.message}</span>
                <span className="rate" role="group" aria-label="rate decision">
                  {(["right", "wrong"] as const).map((r) => (
                    <button
                      type="button"
                      key={r}
                      className={`btn small ${rated[d.callId] === r ? "primary" : ""}`}
                      aria-pressed={rated[d.callId] === r}
                      onClick={() => {
                        backend.coach.rate(d.callId, r, r === "right");
                        setRated({ ...rated, [d.callId]: r });
                      }}
                    >
                      {r === "right" ? "👍" : "👎"}
                    </button>
                  ))}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <div className="rv-grid">
        <div className="rv-cell"><MmpWidget widget={{ id: "rv-mmp", kind: "mmp_curve", x: 0, y: 0, w: 1, h: 1, metrics: ["power"], rateHz: 0.2 }} /></div>
        <div className="rv-cell"><HeatmapWidget widget={{ id: "rv-heat", kind: "heatmap2d", x: 0, y: 0, w: 1, h: 1, metrics: ["power", "hr"], rateHz: 0.2 }} /></div>
        <div className="rv-cell"><HistogramWidget widget={{ id: "rv-hist", kind: "histogram", x: 0, y: 0, w: 1, h: 1, metrics: ["power"], rateHz: 0.2, colorByZone: true }} /></div>
        <div className="rv-cell"><FatigueScatter rec={rec} /></div>
        <div className="rv-cell"><TauChart recs={recoveries} /></div>
        <div className="rv-cell"><HistogramWidget widget={{ id: "rv-hist-hr", kind: "histogram", x: 0, y: 0, w: 1, h: 1, metrics: ["hr"], rateHz: 0.2, colorByZone: true }} /></div>
      </div>

      <section className="rv-card">
        <header className="rv-head"><h3>Laps</h3></header>
        <TableView head={["#", "start", "dur", "target", "avg P", "avg HR", "cad", "compliance", "τ"]} rows={laps} />
      </section>
    </div>
  );
}
