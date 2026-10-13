<script lang="ts">
  import { nav } from "../lib/nav.svelte";

  const GROUPS: { title: string; keys: [string, string][] }[] = [
    {
      title: "Ride",
      keys: [
        ["Space", "pause / resume"],
        ["+ / −", "harder / easier 5 %"],
        ["S", "skip block"],
        ["Y / N", "accept / decline coach nudge"],
        ["← / →", "previous / next page"],
        ["1 / 2 / 3", "speed 1× / 10× / 60×"],
        ["E", "edit layout"],
        ["Esc", "cancel edit / close"],
      ],
    },
    {
      title: "Editor",
      keys: [
        ["⌘Z / ⇧⌘Z", "undo / redo"],
        ["⌘G", "wrap selection in repeat"],
        ["⇧⌘G", "unwrap repeat"],
        ["⌘D", "duplicate"],
        ["Delete", "delete selection"],
        ["⌥↑ / ⌥↓", "move up / down"],
        ["↑ / ↓", "select previous / next"],
        ["Shift-click", "extend selection (siblings)"],
      ],
    },
    { title: "Anywhere", keys: [["?", "this overlay"]] },
  ];

  let dialog = $state<HTMLDialogElement>();
  $effect(() => {
    if (nav.help) dialog?.showModal();
    else dialog?.close();
  });
</script>

<dialog bind:this={dialog} onclose={() => (nav.help = false)}>
  <header>
    <h2>Keyboard shortcuts</h2>
    <button type="button" class="ghost" onclick={() => (nav.help = false)}>✕</button>
  </header>
  <div class="cols">
    {#each GROUPS as g (g.title)}
      <section>
        <h3>{g.title}</h3>
        <dl>
          {#each g.keys as [k, d] (k)}
            <dt><kbd>{k}</kbd></dt>
            <dd>{d}</dd>
          {/each}
        </dl>
      </section>
    {/each}
  </div>
</dialog>

<style>
  dialog {
    background: var(--surface-1);
    color: var(--ink-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1rem 1.25rem;
    max-width: 760px;
    width: 90vw;
  }
  dialog::backdrop {
    background: rgba(0, 0, 0, 0.5);
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  h2 {
    margin: 0;
    font-size: 1.1rem;
  }
  h3 {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-secondary);
  }
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 1rem;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.35rem 0.75rem;
    margin: 0;
  }
  dd {
    margin: 0;
    color: var(--ink-secondary);
  }
</style>
