<script lang="ts">
  import type { CoachNudge, MetricId } from "@neo/shared";
  import uPlot from "uplot";
  import { uplot } from "../charts/attach.svelte";
  import { axisStyle } from "../charts/style";
  import { theme, seriesColor, zoneColor, withAlpha } from "../lib/theme.svelte";
  import { backend } from "../lib/backend";
  import { clock, label, unit, fmtValue } from "../lib/format";
  import { zoneIndex } from "@neo/shared";
  import type { RideRecord } from "./record.svelte";

  // P05 linear stacked panels (shared x, one crosshair, no dual axes): power (+ target,
  // lap bands by zone), HR, cadence, DFA-α1 (P14 reference bands), W'bal; coach decisions as
  // markers in every panel. Drag to brush-zoom all panels; the selection shows its stats.
  let { rec, nudges }: { rec: RideRecord; nudges: CoachNudge[] } = $props();

  interface PanelDef {
    m: MetricId;
    refs?: { v: number; text: string }[];
    target?: boolean;
    laps?: boolean;
    grow?: number;
  }
  const PANELS: PanelDef[] = [
    { m: "power", target: true, laps: true, grow: 1.6 },
    { m: "hr" },
    { m: "cadence" },
    { m: "dfa_a1", refs: [{ v: 0.75, text: "≈LT1 0.75" }, { v: 0.5, text: "≈LT2 0.5" }] },
    { m: "wbal_pct", refs: [{ v: 25, text: "floor" }] },
  ];

  const key = `stack-${Math.random().toString(36).slice(2)}`;
  const charts: (uPlot | null)[] = PANELS.map(() => null);
  let zoom = $state<[number, number] | null>(null);
  let hoverIdx = $state<number | null>(null);

  const fullRange = (): [number, number] => [0, Math.max(60, (rec.t.at(-1) ?? 0) + 1)];

  // propagate zoom imperatively to every panel (setData re-ranges through `range` below)
  $effect(() => {
    const [min, max] = zoom ?? fullRange();
    for (const u of charts) u?.setScale("x", { min, max });
  });

  function data(p: PanelDef): () => uPlot.AlignedData {
    return () => {
      rec.track(); // reactive dependency: new seconds arrived
      const out: uPlot.AlignedData = p.target ? [rec.t, rec.get(p.m), rec.get("target_power")] : [rec.t, rec.get(p.m)];
      return out;
    };
  }

  function drawLaps(u: uPlot): void {
    const c = theme.colors;
    const bi = rec.get("block_index").map((v) => (v === null ? null : Math.round(v)));
    const pw = rec.get("target_power");
    const ctx = u.ctx;
    const top = u.bbox.top;
    const h = u.bbox.height;
    let start = 0;
    for (let i = 1; i <= bi.length; i++) {
      if (i < bi.length && bi[i] === bi[start]) continue;
      const tgt = pw[start] ?? null;
      if (tgt !== null) {
        const x0 = u.valToPos(rec.t[start] ?? 0, "x", true);
        const x1 = u.valToPos(rec.t[i - 1] ?? 0, "x", true);
        const zi = zoneIndex(backend.zones.power, tgt);
        ctx.fillStyle = withAlpha(zoneColor(c, zi, backend.zones.power.length), 0.16);
        ctx.fillRect(x0, top, Math.max(1, x1 - x0), h);
      }
      start = i;
    }
  }

  function drawMarks(u: uPlot, p: PanelDef): void {
    const c = theme.colors;
    const ctx = u.ctx;
    const dpr = devicePixelRatio;
    ctx.save();
    // reference lines (P14 bands, W'bal floor)
    for (const r of p.refs ?? []) {
      const y = u.valToPos(r.v, "y", true);
      ctx.strokeStyle = c.muted;
      ctx.setLineDash([4 * dpr, 4 * dpr]);
      ctx.lineWidth = dpr;
      ctx.beginPath();
      ctx.moveTo(u.bbox.left, y);
      ctx.lineTo(u.bbox.left + u.bbox.width, y);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.fillStyle = c.muted;
      ctx.font = `${10 * dpr}px ${c.font}`;
      ctx.fillText(r.text, u.bbox.left + 4 * dpr, y - 3 * dpr);
    }
    // coach decisions
    for (const n of nudges) {
      const x = u.valToPos(n.t, "x", true);
      if (x < u.bbox.left || x > u.bbox.left + u.bbox.width) continue;
      ctx.strokeStyle = c.ink;
      ctx.lineWidth = dpr;
      ctx.setLineDash([2 * dpr, 3 * dpr]);
      ctx.beginPath();
      ctx.moveTo(x, u.bbox.top);
      ctx.lineTo(x, u.bbox.top + u.bbox.height);
      ctx.stroke();
      ctx.setLineDash([]);
      if (p.laps) {
        ctx.fillStyle = c.ink;
        ctx.beginPath();
        const y = u.bbox.top + 6 * dpr;
        ctx.moveTo(x, y - 5 * dpr);
        ctx.lineTo(x + 5 * dpr, y);
        ctx.lineTo(x, y + 5 * dpr);
        ctx.lineTo(x - 5 * dpr, y);
        ctx.closePath();
        ctx.fill();
      }
    }
    ctx.restore();
  }

  function options(p: PanelDef, last: boolean): () => Omit<uPlot.Options, "width" | "height"> {
    return () => {
      const c = theme.colors;
      const col = seriesColor(c, p.m);
      const series: uPlot.Series[] = [{}, { stroke: col, width: 1.5, points: { show: false } }];
      if (p.target) series.push({ stroke: c.target, width: 1.5, dash: [6, 4], points: { show: false } });
      return {
        legend: { show: false },
        cursor: {
          sync: { key },
          y: false,
          drag: { x: true, y: false, setScale: false },
          points: { size: 7, stroke: c.surface, width: 2 },
        },
        select: { show: true, left: 0, top: 0, width: 0, height: 0 },
        scales: { x: { time: false, range: () => zoom ?? fullRange() }, y: { auto: true, range: p.m === "dfa_a1" ? [0, 1.6] : p.m === "wbal_pct" ? [0, 100] : (_u, lo, hi) => [Math.max(0, Math.floor(lo * 0.9)), Math.ceil(hi * 1.05)] } },
        axes: [
          axisStyle(c, { show: last, size: last ? 24 : 0, values: (_u, s) => s.map((x: number | null) => (x === null ? "" : clock(x))) }),
          axisStyle(c, { size: 40 }),
        ],
        series,
        padding: [4, 10, 0, 0],
        hooks: {
          drawClear: p.laps ? [drawLaps] : [],
          draw: [(u) => drawMarks(u, p)],
          setSelect: [
            (u) => {
              if (u.select.width < 4) return;
              const a = u.posToVal(u.select.left, "x");
              const b = u.posToVal(u.select.left + u.select.width, "x");
              zoom = [Math.max(0, a), b];
              u.setSelect({ left: 0, top: 0, width: 0, height: 0 }, false);
            },
          ],
          setCursor: [
            (u) => {
              hoverIdx = u.cursor.idx ?? null;
            },
          ],
        },
      };
    };
  }

  const stats = $derived.by(() => {
    rec.track();
    const z = zoom;
    if (!z) return null;
    const lo = Math.max(0, Math.ceil(z[0]));
    const hi = Math.min(rec.t.length - 1, Math.floor(z[1]));
    if (hi <= lo) return null;
    const avg = (m: MetricId) => {
      const xs = rec.get(m).slice(lo, hi + 1).filter((v): v is number => v !== null);
      return xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null;
    };
    return { dur: hi - lo, power: avg("power"), hr: avg("hr"), cadence: avg("cadence") };
  });
</script>

<div class="stack">
  <div class="head">
    <div class="legend">
      {#each PANELS as p (p.m)}<span><i style:background={seriesColor(theme.colors, p.m)}></i>{label(p.m)}</span>{/each}
      <span><i class="dash"></i>target</span>
      <span>◆ coach decision</span>
      <span class="muted">drag to zoom</span>
    </div>
    {#if stats}
      <div class="sel tabular">
        selection {clock(stats.dur)} · {fmtValue("power", stats.power)} W · {fmtValue("hr", stats.hr)} bpm · {fmtValue("cadence", stats.cadence)} rpm
        <button type="button" class="ghost small" onclick={() => (zoom = null)}>reset zoom</button>
      </div>
    {:else if hoverIdx !== null}
      {@const hi = hoverIdx}
      <div class="sel tabular">
        {clock(rec.t[hi] ?? 0)} ·
        {#each PANELS as p (p.m)}{label(p.m)} {fmtValue(p.m, rec.get(p.m)[hi] ?? null)} {unit(p.m)} · {/each}
      </div>
    {/if}
  </div>
  {#each PANELS as p, i (p.m)}
    <div
      class="panel"
      style:flex-grow={p.grow ?? 1}
      {@attach uplot(options(p, i === PANELS.length - 1), data(p), (u) => {
        charts[i] = u;
      })}
    ></div>
  {/each}
</div>

<style>
  .stack {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .head {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    font-size: 0.75rem;
    min-height: 1.6rem;
    align-items: center;
  }
  .legend {
    display: flex;
    gap: 0.8rem;
    color: var(--ink-secondary);
  }
  .legend i {
    display: inline-block;
    width: 12px;
    height: 3px;
    margin-right: 4px;
    vertical-align: middle;
  }
  .legend i.dash {
    background: repeating-linear-gradient(90deg, var(--target) 0 4px, transparent 4px 7px);
  }
  .sel {
    color: var(--ink-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .small {
    min-height: 22px;
    padding: 0 0.4rem;
    font-size: 0.72rem;
  }
  .panel {
    flex: 1 1 0;
    min-height: 0;
    overflow: hidden;
  }
</style>
