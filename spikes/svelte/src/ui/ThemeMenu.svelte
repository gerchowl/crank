<script lang="ts">
  import { theme, type Mode, type Preset } from "../lib/theme.svelte";

  // Settings popover (native [popover]: light-dismiss + Esc for free). Changes apply live:
  // the root effect in App re-applies attributes and every chart redraws from theme.colors.
  const MODES: Mode[] = ["dark", "light", "auto"];
  // swatches: Z1 / Z4 / Z7 light steps (tokens.css), preview only
  const PRESETS: { id: Preset; name: string; note: string; swatch: string[] }[] = [
    { id: "graphite", name: "Graphite", note: "quiet, grey zones", swatch: ["#b1afab", "#75736e", "#3d3c38"] },
    { id: "ocean", name: "Ocean", note: "default, CVD-friendly", swatch: ["#7cb2f8", "#3273c3", "#013a76"] },
    { id: "ember", name: "Ember", note: "red zones, magenta critical", swatch: ["#fc8a7e", "#c33e38", "#74050a"] },
  ];
  const id = `theme-${Math.random().toString(36).slice(2, 7)}`;
</script>

<button type="button" class="ghost" popovertarget={id} title="Theme">◐ Theme</button>
<div popover="auto" {id} class="pop">
  <h3>Theme</h3>
  <div class="row" role="group" aria-label="Mode">
    {#each MODES as m (m)}
      <button type="button" aria-pressed={theme.mode === m} onclick={() => (theme.mode = m)}>{m}</button>
    {/each}
  </div>
  <div class="presets">
    {#each PRESETS as p (p.id)}
      <button type="button" class="preset" aria-pressed={theme.preset === p.id} onclick={() => (theme.preset = p.id)}>
        <span class="ramp">
          {#each p.swatch as z, i (i)}<i style:background={z}></i>{/each}
        </span>
        <b>{p.name}</b>
        <span class="muted">{p.note}</span>
      </button>
    {/each}
  </div>
  <label><input type="checkbox" bind:checked={theme.classicZones} /> Classic rainbow zones <span class="muted">(not CVD-safe)</span></label>
  <label><input type="checkbox" bind:checked={theme.oled} /> OLED black (dark mode)</label>
  <div class="zones">
    {#each theme.colors.zones as z, i (i)}
      <span><i style:background={z}></i>Z{i + 1}</span>
    {/each}
  </div>
  <div class="series">
    <span><i style:background={theme.colors.power}></i>power</span>
    <span><i style:background={theme.colors.hr}></i>HR</span>
    <span><i style:background={theme.colors.cadence}></i>cadence</span>
    <span><i style:background={theme.colors.dfa}></i>DFA α1</span>
    <span><i class="dash"></i>target</span>
  </div>
</div>

<style>
  .pop {
    margin: 0;
    inset: 3.2rem 1rem auto auto;
    width: 320px;
    padding: 0.9rem;
    background: var(--surface-1);
    color: var(--ink-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.35);
  }
  h3 {
    margin: 0 0 0.6rem;
    font-size: 0.85rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-secondary);
  }
  .row {
    display: flex;
    gap: 0.3rem;
    margin-bottom: 0.6rem;
  }
  .row button {
    flex: 1;
    text-transform: capitalize;
  }
  .presets {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    margin-bottom: 0.6rem;
  }
  .preset {
    display: grid;
    grid-template-columns: auto 1fr;
    text-align: left;
    gap: 0 0.6rem;
  }
  .preset .ramp {
    grid-row: span 2;
    display: flex;
    align-items: stretch;
    width: 56px;
  }
  .preset .ramp i {
    flex: 1;
  }
  .preset .muted {
    font-size: 0.75rem;
  }
  label {
    display: flex;
    gap: 0.4rem;
    align-items: center;
    font-size: 0.85rem;
    margin: 0.3rem 0;
  }
  .zones,
  .series {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    font-size: 0.72rem;
    margin-top: 0.5rem;
    color: var(--ink-secondary);
  }
  .zones i,
  .series i {
    display: inline-block;
    width: 12px;
    height: 12px;
    border-radius: 3px;
    margin-right: 3px;
    vertical-align: -2px;
  }
  .series i.dash {
    height: 0;
    border-top: 2px dashed var(--target);
    border-radius: 0;
    vertical-align: middle;
  }
</style>
