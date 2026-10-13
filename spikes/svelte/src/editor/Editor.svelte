<script lang="ts">
  import type { RewriteIntent, WorkoutNode } from "@neo/shared";
  import { editor } from "./state.svelte";
  import { nodeAt, type Path } from "./tree";
  import { isTyping, nav } from "../lib/nav.svelte";
  import { ride } from "../lib/ride.svelte";
  import { backend } from "../lib/backend";
  import Profile from "./Profile.svelte";
  import Outline from "./Outline.svelte";
  import DslPane from "./DslPane.svelte";
  import Summary from "./Summary.svelte";

  const INTENTS: { id: RewriteIntent; label: string }[] = [
    { id: "vary", label: "Vary" },
    { id: "harder", label: "Harder" },
    { id: "easier", label: "Easier" },
    { id: "shorter_same_stimulus", label: "Shorter, same stimulus" },
  ];

  const hasSel = $derived(editor.selection.length > 0);
  const focusNode = $derived(editor.focus ? nodeAt(editor.workout.nodes, editor.focus) : undefined);
  let snippetName = $state("");

  // Visible rows (collapsed repeats hide their children), for ↑/↓ navigation.
  const visible = $derived.by(() => {
    const out: Path[] = [];
    const walk = (nodes: WorkoutNode[], parent: Path) => {
      nodes.forEach((n, i) => {
        const p = [...parent, i];
        out.push(p);
        if (n.type === "repeat" && !editor.collapsed.has(n.id)) walk(n.children, p);
      });
    };
    walk(editor.workout.nodes, []);
    return out;
  });

  function onkeydown(e: KeyboardEvent): void {
    const mod = e.metaKey || e.ctrlKey;
    const k = e.key.toLowerCase();
    if (mod && k === "z") {
      // undo/redo work everywhere except inside the DSL textarea (native text undo there)
      if (e.target instanceof HTMLTextAreaElement) return;
      e.preventDefault();
      if (e.shiftKey) editor.redo();
      else editor.undo();
      return;
    }
    if (mod && k === "y") {
      e.preventDefault();
      editor.redo();
      return;
    }
    if (isTyping(e)) return;
    if (mod && k === "g") {
      e.preventDefault();
      if (e.shiftKey) editor.unwrapFocus();
      else editor.wrapSelection();
    } else if (mod && k === "d") {
      e.preventDefault();
      editor.duplicateFocus();
    } else if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      editor.deleteSelection();
    } else if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      editor.moveFocus(e.key === "ArrowUp" ? -1 : 1);
    } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      e.preventDefault();
      editor.step(e.key === "ArrowUp" ? -1 : 1, visible);
    } else if (e.key === "Escape") {
      if (editor.proposal) editor.rejectProposal();
      else editor.select(null);
    } else if (e.key === "Enter" && editor.proposal) {
      editor.acceptProposal();
    }
  }

  function rideThis(): void {
    ride.load($state.snapshot(editor.workout));
    backend.ride.start();
    nav.go("ride");
  }
</script>

<svelte:window {onkeydown} />

<div class="editor">
  <div class="toolbar">
    <input class="name" bind:value={editor.workout.name} aria-label="Workout name" />
    <div class="grp">
      <button type="button" onclick={() => editor.undo()} disabled={!editor.canUndo} title="Undo (⌘Z)">↶</button>
      <button type="button" onclick={() => editor.redo()} disabled={!editor.canRedo} title="Redo (⇧⌘Z)">↷</button>
    </div>
    <div class="grp">
      <button type="button" onclick={() => editor.wrapSelection()} disabled={!hasSel} title="Wrap selection in repeat (⌘G)">Wrap ⌘G</button>
      <button type="button" onclick={() => editor.unwrapFocus()} disabled={focusNode?.type !== "repeat"} title="Unwrap repeat (⇧⌘G)">Unwrap</button>
      <button type="button" onclick={() => editor.duplicateFocus()} disabled={!hasSel} title="Duplicate (⌘D)">Duplicate</button>
      <button type="button" onclick={() => editor.deleteSelection()} disabled={!hasSel} title="Delete (⌫)">Delete</button>
      <button type="button" onclick={() => editor.moveFocus(-1)} disabled={!hasSel} title="Move up (⌥↑)">↑</button>
      <button type="button" onclick={() => editor.moveFocus(1)} disabled={!hasSel} title="Move down (⌥↓)">↓</button>
    </div>
    <div class="grp ai" role="group" aria-label="AI on selection">
      <span class="muted">AI on selection:</span>
      {#each INTENTS as it (it.id)}
        <button type="button" onclick={() => editor.rewrite(it.id)} disabled={!hasSel || editor.aiBusy !== null || editor.proposal !== null}>
          {editor.aiBusy === it.id ? "…" : it.label}
        </button>
      {/each}
    </div>
    <button type="button" class="primary ride" onclick={rideThis}>▶ Ride this</button>
  </div>

  {#if editor.proposal}
    <div class="proposal" role="alert">
      <b>AI proposal · {editor.proposal.intent.replaceAll("_", " ")}</b>
      <span class="muted">{editor.proposal.oldCount} node(s) → {editor.proposal.nodes.length}: <code>{backend.printDsl(editor.proposal.nodes)}</code></span>
      <span class="spacer"></span>
      <button type="button" class="primary" onclick={() => editor.acceptProposal()}>Accept <kbd>↵</kbd></button>
      <button type="button" onclick={() => editor.rejectProposal()}>Reject <kbd>Esc</kbd></button>
    </div>
  {/if}

  <section class="profile card"><Profile /></section>

  <div class="panes">
    <section class="card outline">
      <h3>Outline <span class="muted">{editor.selection.length > 1 ? `${editor.selection.length} selected` : ""}</span></h3>
      <div class="scroll"><Outline /></div>
    </section>
    <section class="card text">
      <h3>Text</h3>
      <DslPane />
    </section>
    <section class="card side">
      <h3>{editor.previewCompiled ? "Summary · proposal vs current" : "Summary"}</h3>
      <Summary compiled={editor.previewCompiled ?? editor.compiled} compare={editor.previewCompiled ? editor.compiled : null} />
      <h3>Snippets</h3>
      <div class="snippets">
        {#each editor.snippets as s, i (`${s.name}-${i}`)}
          <button type="button" onclick={() => editor.insertSnippet(s.nodes)} title={backend.printDsl(s.nodes)}>+ {s.name}</button>
        {/each}
      </div>
      <form
        class="save"
        onsubmit={(e) => {
          e.preventDefault();
          if (!snippetName.trim()) return;
          editor.saveSnippet(snippetName.trim());
          snippetName = "";
        }}
      >
        <input placeholder="snippet name" bind:value={snippetName} disabled={!hasSel} />
        <button type="submit" disabled={!hasSel || !snippetName.trim()}>Save selection</button>
      </form>
    </section>
  </div>
</div>

<style>
  .editor {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px;
    min-height: 640px;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
  }
  .name {
    font-size: 1.05rem;
    font-weight: 600;
    width: 18rem;
    background: transparent;
  }
  .grp {
    display: flex;
    gap: 0.25rem;
    align-items: center;
  }
  .grp button {
    padding: 0.3rem 0.55rem;
    font-size: 0.82rem;
  }
  .ai {
    font-size: 0.8rem;
  }
  .ride {
    margin-left: auto;
  }
  .proposal {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    border: 2px solid var(--accent);
    border-radius: var(--radius);
    background: var(--accent-soft);
    font-size: 0.85rem;
  }
  .proposal code {
    font-family: var(--mono);
    font-size: 0.78rem;
    color: var(--ink-primary);
  }
  .spacer {
    flex: 1;
  }
  .card {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0.5rem 0.6rem;
    min-height: 0;
  }
  .profile {
    flex: 0 0 34%;
    min-height: 220px;
  }
  .panes {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr) minmax(260px, 0.8fr);
    gap: 8px;
  }
  .outline,
  .text,
  .side {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .side {
    overflow: auto;
  }
  .scroll {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }
  h3 {
    margin: 0;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-secondary);
  }
  .snippets {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }
  .snippets button {
    font-size: 0.8rem;
    padding: 0.25rem 0.5rem;
    min-height: 30px;
  }
  .save {
    display: flex;
    gap: 0.3rem;
  }
  .save input {
    flex: 1;
    min-width: 0;
  }
  .save button {
    font-size: 0.8rem;
  }
</style>
