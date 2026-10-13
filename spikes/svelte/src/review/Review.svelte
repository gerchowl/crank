<script lang="ts">
  import type { CoachNudge, HrRecoveryState, MetricId } from "@neo/shared";
  import { SvelteMap } from "svelte/reactivity";
  import { backend } from "../lib/backend";
  import { ride } from "../lib/ride.svelte";
  import { liveMetric, poll } from "../lib/live.svelte";
  import { clock, fmtValue, label } from "../lib/format";
  import { RideRecord } from "./record.svelte";
  import RideStack from "./RideStack.svelte";
  import MmpChart from "../charts/MmpChart.svelte";
  import HeatmapChart from "../charts/HeatmapChart.svelte";
  import FatigueScatter from "./FatigueScatter.svelte";
  import TauChart from "./TauChart.svelte";

  // V5 ride review: summary tiles, P05 stacked panels with coach markers, P20 decisions with
  // 👍/👎 rating, P09, P07, P16, P12-per-block, laps table. Works mid-ride too (polls ≤ 1 Hz).
  const rec = new RideRecord();
  $effect(() => {
    rec.pull();
    if (ride.state !== "riding") return;
    const id = setInterval(() => {
      rec.pull();
    }, 1000);
    return () => {
      clearInterval(id);
    };
  });

  const recs = poll(
    () => backend.recoveries(),
    () => 0.5,
    () => ride.state === "riding",
  );
  const mmp = poll(
    () => backend.mmp(),
    () => 0.2,
    () => ride.state === "riding",
  );
  const heat = poll(
    () => backend.heatmap("power", "hr", 20, 4),
    () => 0.2,
    () => ride.state === "riding",
  );

  const TILES: MetricId[] = ["kj", "np", "if", "tss", "decoupling", "compliance"];
  const tiles = TILES.map((m) => ({
    m,
    s: liveMetric(
      () => m,
      () => ({ rateHz: 1 }),
    ),
  }));
  const tauMean = $derived.by(() => {
    const f = (recs.value ?? []).flatMap((r: HrRecoveryState) => (r.fit ? [r.fit.tauS] : []));
    return f.length ? f.reduce((a, b) => a + b, 0) / f.length : null;
  });

  // P20 decisions with rating (rating is sent to the coach; kept locally for the UI)
  const ratings = new SvelteMap<string, "right" | "wrong">();
  function rate(n: CoachNudge, r: "right" | "wrong"): void {
    ratings.set(n.callId, r);
    backend.coach.rate(n.callId, r, r === "right");
  }
  const VERDICT: Record<CoachNudge["verdict"], string> = { ease_off: "ease off", hold: "hold", push: "push", replan: "re-plan" };
  const BAND_STATUS = { strong: "good", lean: "warning", uncertain: "serious" } as const;

  // Laps = runs of equal block_index in the 1 Hz record
  const laps = $derived.by(() => {
    rec.track();
    // per-second means: a second spanning a block change averages two indices → round
    const bi = rec.get("block_index").map((v) => (v === null ? null : Math.round(v)));
    const P = rec.get("power");
    const H = rec.get("hr");
    const C = rec.get("cadence");
    const T = rec.get("target_power");
    const out: { n: number; start: number; dur: number; target: number | null; p: number | null; hr: number | null; cad: number | null; compliance: number | null }[] = [];
    const avg = (xs: (number | null)[]) => {
      const v = xs.filter((x): x is number => x !== null);
      return v.length ? v.reduce((a, b) => a + b, 0) / v.length : null;
    };
    let s = 0;
    for (let i = 1; i <= bi.length; i++) {
      if (i < bi.length && bi[i] === bi[s]) continue;
      const sl = <T,>(a: T[]) => a.slice(s, i);
      const tg = avg(sl(T));
      const within = sl(P).filter((p, k) => {
        const t = T[s + k];
        return p !== null && t !== null && t !== undefined && t > 0 && Math.abs(p - t) <= t * 0.05;
      }).length;
      out.push({ n: bi[s] ?? 0, start: rec.t[s] ?? s, dur: i - s, target: tg, p: avg(sl(P)), hr: avg(sl(H)), cad: avg(sl(C)), compliance: tg ? (within / (i - s)) * 100 : null });
      s = i;
    }
    return out;
  });
  const tauByStart = $derived(new Map((recs.value ?? []).map((r) => [Math.round(r.startedT), r.fit?.tauS])));
  const lapTau = (start: number) => {
    for (const [t, tau] of tauByStart) if (Math.abs(t - start) <= 2) return tau;
    return undefined;
  };
</script>

<div class="review">
  <header class="summary">
    <h1>{ride.current?.workout.name ?? "Ride"} <span class="muted">· {ride.state === "finished" ? "finished" : ride.state === "riding" ? "in progress" : ride.state}</span></h1>
    <div class="tiles">
      <div><span class="k">time</span><b class="tabular">{clock(ride.tSec)}</b></div>
      {#each tiles as t (t.m)}
        <div><span class="k">{label(t.m)}</span><b class="tabular">{fmtValue(t.m, t.s.v)}</b></div>
      {/each}
      <div><span class="k"><span class="greek">τ̄</span> recovery</span><b class="tabular">{tauMean === null ? "—" : `${Math.round(tauMean)} s`}</b></div>
    </div>
  </header>

  <section class="card stack"><RideStack {rec} nudges={ride.nudges} /></section>

  <section class="card decisions">
    <h2>Coach decisions</h2>
    {#if ride.nudges.length === 0}
      <p class="muted">No coach decisions in this ride yet.</p>
    {:else}
      <ul>
        {#each ride.nudges as n (n.callId)}
          <li>
            <span class="tabular t">{clock(n.t)}</span>
            <span>◆ <b>{VERDICT[n.verdict]}</b>{n.proposedPctChange !== undefined ? ` ${n.proposedPctChange > 0 ? "+" : ""}${n.proposedPctChange}%` : ""}</span>
            <span class="status {BAND_STATUS[n.band]}">{n.band}</span>
            <span class="secondary">{n.status.replace("_", " ")}</span>
            <span class="msg muted">{n.message}</span>
            <span class="rate" role="group" aria-label="Rate this decision">
              <button type="button" class="ghost" aria-pressed={ratings.get(n.callId) === "right"} onclick={() => rate(n, "right")} title="Good call">👍</button>
              <button type="button" class="ghost" aria-pressed={ratings.get(n.callId) === "wrong"} onclick={() => rate(n, "wrong")} title="Bad call">👎</button>
            </span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <div class="grid">
    <section class="card"><MmpChart curve={mmp.value} title="Power-duration (MMP)" /></section>
    <section class="card"><HeatmapChart map={heat.value} title="Power × HR" /></section>
    <section class="card"><FatigueScatter {rec} /></section>
    <section class="card"><TauChart recs={recs.value ?? []} /></section>
  </div>

  <section class="card laps">
    <h2>Laps</h2>
    <table class="data">
      <thead>
        <tr><th>block</th><th>start</th><th>dur</th><th>target W</th><th>avg W</th><th>avg HR</th><th>cadence</th><th>compliance</th><th>τ</th></tr>
      </thead>
      <tbody>
        {#each laps as l (l.start)}
          {@const tau = lapTau(l.start)}
          <tr>
            <td>{l.n}</td><td>{clock(l.start)}</td><td>{clock(l.dur)}</td>
            <td>{fmtValue("power", l.target)}</td><td>{fmtValue("power", l.p)}</td><td>{fmtValue("hr", l.hr)}</td><td>{fmtValue("cadence", l.cad)}</td>
            <td>{l.compliance === null ? "—" : `${Math.round(l.compliance)}%`}</td>
            <td>{tau === undefined ? "" : `${Math.round(tau)} s`}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>
</div>

<style>
  .review {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 12px 24px;
  }
  h1 {
    margin: 0.2rem 0 0.4rem;
    font-size: 1.3rem;
  }
  h2 {
    margin: 0 0 0.4rem;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-secondary);
  }
  .tiles {
    display: flex;
    gap: 1.6rem;
    flex-wrap: wrap;
  }
  .tiles div {
    display: flex;
    flex-direction: column;
  }
  .tiles b {
    font-size: 1.6rem;
    font-weight: 650;
  }
  .k {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-secondary);
  }
  .greek {
    text-transform: none;
    font-size: 0.85rem;
  }
  .card {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0.5rem 0.7rem;
  }
  .stack {
    height: 460px;
  }
  .decisions ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 180px;
    overflow: auto;
  }
  .decisions li {
    display: grid;
    grid-template-columns: 3.5rem 9rem 6rem 7rem 1fr auto;
    gap: 0.6rem;
    align-items: center;
    font-size: 0.85rem;
    padding: 0.2rem 0;
    border-bottom: 1px solid var(--grid);
  }
  .msg {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rate button {
    min-height: 30px;
    padding: 0 0.4rem;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }
  @media (min-width: 1400px) {
    .grid {
      grid-template-columns: repeat(4, minmax(0, 1fr));
    }
  }
  .grid .card {
    height: 300px;
  }
  .laps {
    max-height: 420px;
    overflow: auto;
  }
</style>
