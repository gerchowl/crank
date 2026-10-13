<script lang="ts">
  import type { WorkoutNode } from "@neo/shared";
  import { editor } from "./state.svelte";
  import { endText, targetText } from "../lib/format";
  import { isPrefix, pathKey, samePath, type Path } from "./tree";
  import StepFields from "./StepFields.svelte";

  // Outline tree as a *recursive snippet* — no separate row component, no prop drilling.
  // HTML5 drag-and-drop: drop on the top/bottom third of a row = before/after, middle of a
  // repeat = into it.
  let dragFrom: Path | null = $state(null);
  let dropAt = $state<{ key: string; where: "before" | "after" | "into" } | null>(null);

  function whereFor(e: DragEvent, node: WorkoutNode): "before" | "after" | "into" {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const f = (e.clientY - r.top) / r.height;
    if (node.type === "repeat" && f > 0.3 && f < 0.7) return "into";
    return f < 0.5 ? "before" : "after";
  }

  function inProposal(p: Path): boolean {
    const pr = editor.proposal;
    if (!pr) return false;
    const i = p[pr.parent.length] ?? -1;
    return isPrefix(pr.parent, p) && p.length > pr.parent.length && i >= pr.lo && i < pr.lo + pr.oldCount;
  }

  const sumText = (n: WorkoutNode): string =>
    n.type === "step" ? `${endText(n.end)} @ ${targetText(n.target)}${n.cadence ? ` · ${n.cadence.lo}–${n.cadence.hi} rpm` : ""}` : `${n.count}×`;
</script>

{#snippet row(node: WorkoutNode, path: Path)}
  {@const key = pathKey(path)}
  {@const sel = editor.isSelected(path)}
  <li class:repeat={node.type === "repeat"}>
    <div
      class="row"
      class:sel
      class:focus={editor.focus !== null && samePath(editor.focus, path)}
      class:replaced={inProposal(path)}
      class:drop-before={dropAt?.key === key && dropAt.where === "before"}
      class:drop-after={dropAt?.key === key && dropAt.where === "after"}
      class:drop-into={dropAt?.key === key && dropAt.where === "into"}
      draggable="true"
      role="treeitem"
      aria-selected={sel}
      aria-level={path.length}
      aria-expanded={node.type === "repeat" ? !editor.collapsed.has(node.id) : undefined}
      tabindex="-1"
      onclick={(e) => editor.select(path, e.shiftKey)}
      onkeydown={(e) => {
        if (e.key === "Enter") editor.select(path);
      }}
      ondragstart={(e) => {
        dragFrom = path;
        e.dataTransfer?.setData("text/plain", key);
      }}
      ondragover={(e) => {
        if (!dragFrom || isPrefix(dragFrom, path)) return;
        e.preventDefault();
        dropAt = { key, where: whereFor(e, node) };
      }}
      ondragleave={() => {
        if (dropAt?.key === key) dropAt = null;
      }}
      ondrop={(e) => {
        e.preventDefault();
        if (dragFrom && dropAt) editor.dropOn(dragFrom, path, dropAt.where);
        dragFrom = null;
        dropAt = null;
      }}
      ondragend={() => {
        dragFrom = null;
        dropAt = null;
      }}
    >
      <span class="grip" aria-hidden="true">⋮⋮</span>
      {#if node.type === "repeat"}
        <button
          type="button"
          class="ghost tw"
          onclick={(e) => {
            e.stopPropagation();
            if (editor.collapsed.has(node.id)) editor.collapsed.delete(node.id);
            else editor.collapsed.add(node.id);
          }}
          aria-label={editor.collapsed.has(node.id) ? "Expand" : "Collapse"}>{editor.collapsed.has(node.id) ? "▸" : "▾"}</button
        >
      {/if}
      {#if sel && editor.selection.length === 1}
        <input class="label" placeholder="label" value={node.label ?? ""} onchange={(e) => {
            if (e.currentTarget.value) node.label = e.currentTarget.value;
            else delete node.label;
          }} onclick={(e) => e.stopPropagation()} />
        {#if node.type === "repeat"}
          <span class="rep" role="presentation" onclick={(e) => e.stopPropagation()}>
            <input type="number" min="1" max="99" bind:value={node.count} aria-label="Repeat count" />×
            <label title="Progression watts per rep">+W <input type="number" value={node.progression?.wattsPerRep ?? 0} onchange={(e) => (node.progression = { ...node.progression, wattsPerRep: Number(e.currentTarget.value) })} /></label>
            <label title="Progression % FTP per rep">+% <input type="number" value={node.progression?.pctPerRep ?? 0} onchange={(e) => (node.progression = { ...node.progression, pctPerRep: Number(e.currentTarget.value) })} /></label>
            <label title="Skip the last rest on the final rep"><input type="checkbox" checked={node.skipLastRest ?? false} onchange={(e) => (node.skipLastRest = e.currentTarget.checked)} /> skip last rest</label>
          </span>
        {:else}
          <span role="presentation" onclick={(e) => e.stopPropagation()}><StepFields {node} /></span>
        {/if}
      {:else}
        {#if node.label}<b class="lbl">{node.label}</b>{/if}
        <span class="sum">{sumText(node)}</span>
        {#if node.type === "repeat"}
          <span class="muted small">
            {#if node.progression?.wattsPerRep}+{node.progression.wattsPerRep} W/rep{/if}
            {#if node.progression?.pctPerRep}+{node.progression.pctPerRep}%/rep{/if}
            {#if node.skipLastRest}· skip last rest{/if}
          </span>
        {/if}
      {/if}
    </div>
    {#if node.type === "repeat" && !editor.collapsed.has(node.id)}
      <ul role="group">
        {#each node.children as child, i (child.id)}
          {@render row(child, [...path, i])}
        {/each}
      </ul>
    {/if}
  </li>
{/snippet}

<ul class="tree" role="tree" aria-multiselectable="true">
  {#each editor.workout.nodes as node, i (node.id)}
    {@render row(node, [i])}
  {/each}
</ul>
{#if editor.workout.nodes.length === 0}
  <p class="muted">Empty workout — type DSL on the right or insert a snippet.</p>
{/if}

<style>
  .tree,
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  ul ul {
    margin-left: 1.1rem;
    border-left: 2px solid var(--grid);
    padding-left: 0.35rem;
  }
  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.4rem;
    padding: 0.25rem 0.4rem;
    border-radius: 6px;
    border: 1px solid transparent;
    cursor: default;
    font-size: 0.88rem;
    min-height: 32px;
  }
  .row:hover {
    background: var(--surface-2);
  }
  .row.sel {
    background: var(--accent-soft);
    border-color: var(--accent);
  }
  .row.replaced {
    outline: 2px dashed var(--ink-muted);
    opacity: 0.7;
  }
  .row.drop-before {
    box-shadow: inset 0 3px 0 var(--accent);
  }
  .row.drop-after {
    box-shadow: inset 0 -3px 0 var(--accent);
  }
  .row.drop-into {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .grip {
    color: var(--ink-muted);
    cursor: grab;
    font-size: 0.75rem;
    letter-spacing: -2px;
  }
  .tw {
    min-height: 22px;
    padding: 0 0.3rem;
  }
  .lbl {
    font-weight: 600;
  }
  .repeat > .row .sum {
    font-weight: 700;
  }
  .small {
    font-size: 0.75rem;
  }
  .label {
    width: 8rem;
    padding: 0.1rem 0.3rem;
    font-size: 0.8rem;
  }
  .rep {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.8rem;
  }
  .rep input[type="number"] {
    width: 3.5rem;
    padding: 0.1rem 0.3rem;
  }
</style>
