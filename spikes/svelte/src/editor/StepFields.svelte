<script lang="ts">
  import { formatDuration, parseDurationWord, type PowerTarget, type StepEnd, type StepNode } from "@neo/shared";

  // Inline step editor. `node` is part of the editor's deep $state tree: writing its fields
  // is the edit — profile, DSL text and summary update from the same tree.
  let { node }: { node: StepNode } = $props();

  const END_DEFAULTS: Record<StepEnd["kind"], StepEnd> = {
    time: { kind: "time", s: 300 },
    kj: { kind: "kj", kj: 50 },
    lap: { kind: "lap" },
    hr_recovery: { kind: "hr_recovery", targetBpm: 120, minS: 120, maxS: 240, onUnreachable: "lower_power" },
  };
  const TARGET_DEFAULTS: Record<PowerTarget["kind"], PowerTarget> = {
    pct_ftp: { kind: "pct_ftp", value: 75 },
    watts: { kind: "watts", value: 200 },
    range: { kind: "range", lo: 70, hi: 80 },
    ramp: { kind: "ramp", from: 50, to: 75 },
    hr: { kind: "hr", bpm: 140 },
    free: { kind: "free" },
  };

  function setDuration(e: Event & { currentTarget: HTMLInputElement }, apply: (s: number) => void): void {
    const s = parseDurationWord(e.currentTarget.value.trim().toLowerCase());
    if (s === null || s <= 0) {
      e.currentTarget.setCustomValidity("e.g. 30s, 4m, 1m30s");
      e.currentTarget.reportValidity();
      return;
    }
    e.currentTarget.setCustomValidity("");
    apply(s);
  }
</script>

<div class="fields">
  <select
    value={node.end.kind}
    onchange={(e) => (node.end = structuredClone(END_DEFAULTS[e.currentTarget.value as StepEnd["kind"]]))}
    aria-label="End condition"
  >
    <option value="time">time</option>
    <option value="kj">kJ</option>
    <option value="lap">lap</option>
    <option value="hr_recovery">HR recovery</option>
  </select>
  {#if node.end.kind === "time"}
    {@const end = node.end}
    <input class="dur" value={formatDuration(end.s)} onchange={(e) => setDuration(e, (s) => (end.s = s))} aria-label="Duration" />
  {:else if node.end.kind === "kj"}
    <input type="number" min="1" bind:value={node.end.kj} aria-label="kJ" /> kJ
  {:else if node.end.kind === "hr_recovery"}
    {@const end = node.end}
    HR&lt;<input type="number" min="60" max="200" bind:value={end.targetBpm} aria-label="Target HR" />
    <input class="dur" value={formatDuration(end.minS)} onchange={(e) => setDuration(e, (s) => (end.minS = s))} aria-label="Min duration" />..
    <input class="dur" value={formatDuration(end.maxS)} onchange={(e) => setDuration(e, (s) => (end.maxS = s))} aria-label="Max duration" />
    <select bind:value={end.onUnreachable} aria-label="If unreachable">
      <option value="lower_power">lower power</option>
      <option value="cap">cap at max</option>
      <option value="ask_coach">ask coach</option>
    </select>
  {/if}

  <select
    value={node.target.kind}
    onchange={(e) => (node.target = structuredClone(TARGET_DEFAULTS[e.currentTarget.value as PowerTarget["kind"]]))}
    aria-label="Target"
  >
    <option value="pct_ftp">% FTP</option>
    <option value="watts">watts</option>
    <option value="range">range %</option>
    <option value="ramp">ramp %</option>
    <option value="hr">HR</option>
    <option value="free">free</option>
  </select>
  {#if node.target.kind === "pct_ftp"}
    <input type="number" min="0" max="400" bind:value={node.target.value} aria-label="% FTP" />%
  {:else if node.target.kind === "watts"}
    <input type="number" min="0" max="2000" bind:value={node.target.value} aria-label="Watts" /> W
  {:else if node.target.kind === "range"}
    <input type="number" bind:value={node.target.lo} aria-label="Low %" />–<input type="number" bind:value={node.target.hi} aria-label="High %" />%
  {:else if node.target.kind === "ramp"}
    <input type="number" bind:value={node.target.from} aria-label="From %" />→<input type="number" bind:value={node.target.to} aria-label="To %" />%
  {:else if node.target.kind === "hr"}
    <input type="number" bind:value={node.target.bpm} aria-label="HR target" /> bpm
  {/if}

  {#if node.cadence}
    {@const cad = node.cadence}
    <input type="number" bind:value={cad.lo} aria-label="Cadence low" />–<input type="number" bind:value={cad.hi} aria-label="Cadence high" />rpm
    <button type="button" class="ghost x" onclick={() => {
        delete node.cadence;
      }} title="Remove cadence target">✕</button>
  {:else}
    <button type="button" class="ghost x" onclick={() => (node.cadence = { lo: 85, hi: 95 })}>+ rpm</button>
  {/if}
</div>

<style>
  .fields {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.8rem;
  }
  input[type="number"] {
    width: 4rem;
  }
  .dur {
    width: 4.5rem;
  }
  select,
  input {
    padding: 0.1rem 0.3rem;
  }
  .x {
    min-height: 24px;
    padding: 0 0.35rem;
    font-size: 0.75rem;
  }
</style>
