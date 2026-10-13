<script lang="ts">
  import { backend } from "../lib/backend";
  import { ride } from "../lib/ride.svelte";
  import { layout } from "../lib/layout.svelte";
  import { liveMetric } from "../lib/live.svelte";
  import { clock, PHASE_LABEL } from "../lib/format";
  import { nav } from "../lib/nav.svelte";
  import ThemeMenu from "../ui/ThemeMenu.svelte";

  // Top bar reads only fine-grained deriveds (tSec, state, trainer…): the 2 Hz status
  // heartbeat re-renders the clock text and nothing else.
  const remaining = liveMetric(
    () => "remaining",
    () => ({ rateHz: 1 }),
  );
  const CONN: Record<string, string> = { connected: "good", connecting: "warning", lost: "critical" };
  const SPEEDS = [1, 10, 60] as const;
</script>

<header class="bar">
  <div class="clock tabular">
    <b>{clock(ride.tSec)}</b><span class="muted"> / {clock(ride.tSec + (remaining.v ?? Math.max(0, ride.totalS - ride.tSec)))}</span>
    <span class="secondary rem">−{clock(remaining.v ?? 0)}</span>
  </div>
  <span class="chip mode">{ride.mode.toUpperCase()}</span>
  <span class="chip">{PHASE_LABEL[ride.phase]}</span>
  <span class="status {CONN[ride.trainer]}" title="Trainer {ride.trainer}">NEO {ride.trainer === "connected" ? "" : ride.trainer}</span>
  <span class="status {CONN[ride.hrSensor]}" title="HR strap {ride.hrSensor}">HR {ride.hrSensor === "connected" ? "" : ride.hrSensor}</span>

  <div class="group">
    <button type="button" onclick={() => backend.ride.adjust(-5)} title="Easier 5 % (−)"><kbd>−</kbd> 5%</button>
    <span class="offset tabular" class:off={ride.manualOffsetPct !== 0}>{ride.manualOffsetPct > 0 ? "+" : ""}{ride.manualOffsetPct}%</span>
    <button type="button" onclick={() => backend.ride.adjust(5)} title="Harder 5 % (+)"><kbd>+</kbd> 5%</button>
    <button type="button" onclick={() => backend.ride.skipBlock()} title="Skip block (S)"><kbd>S</kbd> skip</button>
    <button type="button" class="primary play" onclick={() => ride.togglePause()} title="Pause / resume (Space)">
      {ride.state === "riding" ? "Pause" : ride.state === "paused" ? "Resume" : ride.state === "finished" ? "Restart" : "Start"}
    </button>
  </div>

  <div class="group speed" role="group" aria-label="Simulation speed">
    {#each SPEEDS as s (s)}
      <button type="button" class="ghost" aria-pressed={ride.speed === s} onclick={() => backend.ride.setSpeed(s)}>{s}×</button>
    {/each}
  </div>

  <nav class="pages" aria-label="Pages">
    {#each layout.layout.pages as p, i (p.id)}
      <button type="button" class="dot" aria-pressed={layout.pageIndex === i} onclick={() => (layout.pageIndex = i)} title="{p.name}{p.autoPhase ? ` (auto: ${p.autoPhase.join(', ')})` : ''}">
        <i></i><span class="pname">{p.name}</span>
      </button>
    {/each}
    <label class="auto" title="Switch page automatically by workout phase">
      <input type="checkbox" bind:checked={layout.autoSwitch} /> auto
    </label>
  </nav>

  <div class="group right">
    <button type="button" class="ghost" aria-pressed={layout.editing} onclick={() => (layout.editing ? layout.cancelEdit() : layout.startEdit())} title="Edit layout (E)">
      ⊞<span class="wide"> Layout</span>
    </button>
    <ThemeMenu />
    <button type="button" class="ghost" onclick={() => (nav.help = true)} title="Keyboard shortcuts (?)"><kbd>?</kbd></button>
    <button type="button" class="ghost" onclick={() => nav.go("review")} title="End ride → review">Exit</button>
  </div>
</header>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.4rem 0.75rem;
    background: var(--surface-1);
    border-bottom: 1px solid var(--border);
    flex-wrap: nowrap;
    font-size: 0.85rem;
    white-space: nowrap;
  }
  .clock {
    font-size: 1.25rem;
    min-width: 11.5rem;
  }
  @media (max-width: 1500px) {
    .bar {
      gap: 0.4rem;
      padding: 0.4rem 0.5rem;
    }
    .wide {
      display: none;
    }
    .bar :global(button) {
      padding: 0.35rem 0.5rem;
    }
    .pname {
      display: none;
    }
    .dot[aria-pressed="true"] .pname {
      display: inline;
    }
  }
  .clock b {
    font-weight: 650;
  }
  .rem {
    font-size: 0.85rem;
    margin-left: 0.4rem;
  }
  .chip {
    padding: 0.1rem 0.5rem;
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--ink-secondary);
    font-size: 0.75rem;
  }
  .mode {
    font-weight: 700;
    color: var(--ink-primary);
  }
  .group {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }
  .offset {
    min-width: 3rem;
    text-align: center;
    color: var(--ink-muted);
  }
  .offset.off {
    color: var(--ink-primary);
    font-weight: 600;
  }
  .play {
    min-width: 5.5rem;
  }
  .speed button {
    padding: 0.3rem 0.5rem;
  }
  .pages {
    display: flex;
    align-items: center;
    gap: 0.2rem;
  }
  .dot {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    background: transparent;
    border-color: transparent;
    padding: 0.3rem 0.5rem;
  }
  .dot i {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 2px solid var(--ink-muted);
  }
  .dot[aria-pressed="true"] i {
    background: var(--accent);
    border-color: var(--accent);
  }
  .dot[aria-pressed="true"] {
    background: var(--accent-soft);
  }
  .auto {
    font-size: 0.75rem;
    color: var(--ink-secondary);
    display: flex;
    align-items: center;
    gap: 0.2rem;
  }
  .right {
    margin-left: auto;
  }
</style>
