<script lang="ts">
  import type { Checkin, Proposal } from "@neo/shared";
  import { backend } from "../lib/backend";
  import { ride } from "../lib/ride.svelte";
  import { nav } from "../lib/nav.svelte";
  import { editor } from "../editor/state.svelte";
  import { clock } from "../lib/format";
  import MiniProfile from "../charts/MiniProfile.svelte";

  // V2 check-in (≤ 10 s): four 1–5 rows of big tap targets, minutes chips, optional note,
  // then planner.propose → proposal card → accept / regenerate / edit.
  type Score = 1 | 2 | 3 | 4 | 5;
  const SCORES: Score[] = [1, 2, 3, 4, 5];
  const ROWS: { key: "sleep" | "legs" | "stress" | "motivation"; label: string; lo: string; hi: string; invert?: boolean }[] = [
    { key: "sleep", label: "Sleep", lo: "awful", hi: "great" },
    { key: "legs", label: "Legs", lo: "dead", hi: "fresh" },
    { key: "stress", label: "Stress", lo: "calm", hi: "high", invert: true },
    { key: "motivation", label: "Motivation", lo: "none", hi: "fired up" },
  ];
  const MINUTES = [30, 45, 60, 90];

  let form = $state<Record<"sleep" | "legs" | "stress" | "motivation", Score>>({ sleep: 3, legs: 3, stress: 3, motivation: 3 });
  let minutes = $state(60);
  let custom = $state(false);
  let note = $state("");
  let busy = $state(false);
  let proposal = $state.raw<Proposal | null>(null);

  const compiled = $derived(proposal ? backend.compileWorkout(proposal.workout) : null);

  // P23-style readiness tiles: status = good/warning/serious from the 1–5 answers.
  const readiness = $derived(
    ROWS.map((r) => {
      const v = form[r.key];
      const good = r.invert ? 6 - v : v;
      return { label: r.label, v, status: good >= 4 ? "good" : good === 3 ? "warning" : "serious", text: good >= 4 ? "ok" : good === 3 ? "so-so" : "low" };
    }),
  );

  async function submit(): Promise<void> {
    busy = true;
    const c: Checkin = { ...form, minutes, ...(note.trim() ? { note: note.trim() } : {}) };
    try {
      proposal = await backend.planner.propose(c);
    } finally {
      busy = false;
    }
  }

  function accept(): void {
    if (!proposal) return;
    ride.load(proposal.workout);
    backend.ride.start();
    nav.go("ride");
  }

  function edit(): void {
    if (!proposal) return;
    editor.load(proposal.workout);
    nav.go("editor");
  }
</script>

<div class="page">
  <h1>How are you today?</h1>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
  >
    {#each ROWS as r (r.key)}
      <fieldset class="row">
        <legend>{r.label}</legend>
        <span class="muted end">{r.lo}</span>
        <div class="taps" role="radiogroup" aria-label={r.label}>
          {#each SCORES as s (s)}
            <button type="button" role="radio" aria-checked={form[r.key] === s} class="tap" class:on={form[r.key] === s} onclick={() => (form[r.key] = s)}>{s}</button>
          {/each}
        </div>
        <span class="muted end">{r.hi}</span>
      </fieldset>
    {/each}

    <fieldset class="row">
      <legend>Time</legend>
      <div class="chips">
        {#each MINUTES as m (m)}
          <button type="button" class="chip" aria-pressed={!custom && minutes === m} onclick={() => ((minutes = m), (custom = false))}>{m}′</button>
        {/each}
        <button type="button" class="chip" aria-pressed={custom} onclick={() => (custom = true)}>custom</button>
        {#if custom}<input type="number" min="15" max="300" step="5" bind:value={minutes} aria-label="Minutes" /> min{/if}
      </div>
    </fieldset>

    <label class="note">
      <span>How do you feel? <span class="muted">(optional)</span></span>
      <input bind:value={note} placeholder="e.g. a bit heavy after yesterday" />
    </label>

    <div class="readiness" aria-label="Readiness">
      {#each readiness as t (t.label)}
        <div class="tile"><span class="muted">{t.label}</span><span class="status {t.status}">{t.text} · {t.v}/5</span></div>
      {/each}
    </div>

    <div class="actions">
      <button type="submit" class="primary big" disabled={busy}>{busy ? "Planning…" : proposal ? "Regenerate" : "Get my workout"}</button>
    </div>
  </form>

  {#if proposal && compiled}
    <section class="proposal" aria-live="polite">
      <header>
        <h2>{proposal.workout.name}</h2>
        <span class="secondary tabular">{clock(compiled.totalS)} · TSS {Math.round(compiled.tss)} · IF {compiled.if.toFixed(2)}</span>
      </header>
      <div class="mini"><MiniProfile {compiled} ftp={proposal.workout.ftp} /></div>
      <p class="why"><span class="muted">why:</span> {proposal.rationale}</p>
      <div class="actions">
        <button type="button" class="primary big" onclick={accept}>▶ Accept &amp; ride</button>
        <button type="button" class="big" onclick={() => void submit()} disabled={busy}>↻ Regenerate</button>
        <button type="button" class="big" onclick={edit}>✎ Edit</button>
      </div>
    </section>
  {/if}
</div>

<style>
  .page {
    max-width: 880px;
    margin: 0 auto;
    padding: 1.5rem 1.5rem 3rem;
  }
  h1 {
    font-size: 1.6rem;
    margin: 0 0 1rem;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .row {
    display: grid;
    grid-template-columns: 7rem 4rem 1fr 4rem;
    align-items: center;
    gap: 0.75rem;
    border: none;
    margin: 0;
    padding: 0.35rem 0;
  }
  legend {
    float: left;
    font-weight: 600;
    padding: 0;
  }
  .row:has(.chips) {
    grid-template-columns: 7rem 1fr;
  }
  .end {
    font-size: 0.8rem;
    text-align: center;
  }
  .taps {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 0.4rem;
  }
  .tap {
    min-height: 56px;
    font-size: 1.3rem;
    font-weight: 600;
  }
  .tap.on {
    background: var(--accent);
    color: var(--accent-ink);
    border-color: transparent;
  }
  .chips {
    display: flex;
    gap: 0.4rem;
    align-items: center;
  }
  .chip {
    min-height: 48px;
    min-width: 64px;
    font-size: 1.05rem;
  }
  .chips input {
    width: 5rem;
  }
  .note {
    display: grid;
    grid-template-columns: 7rem 1fr;
    gap: 0.75rem;
    align-items: center;
  }
  .note > span {
    font-weight: 600;
  }
  .note input {
    min-height: 40px;
  }
  .readiness {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin-top: 0.4rem;
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0.4rem 0.7rem;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-size: 0.82rem;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.6rem;
  }
  .big {
    min-height: 52px;
    padding: 0 1.4rem;
    font-size: 1.05rem;
  }
  .proposal {
    margin-top: 1.5rem;
    padding: 1rem 1.2rem;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .proposal header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 1rem;
  }
  h2 {
    margin: 0;
    font-size: 1.2rem;
  }
  .mini {
    height: 110px;
    margin: 0.75rem 0;
  }
  .why {
    margin: 0;
    color: var(--ink-secondary);
  }
</style>
