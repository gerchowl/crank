<script lang="ts">
  import { editor } from "./state.svelte";

  // Text DSL view. While typing, the draft is shown (not the canonical reprint); valid text
  // replaces the tree immediately, so the profile/outline follow keystroke by keystroke.
  // Errors are highlighted in place via a mirrored backdrop behind the textarea.
  let ta = $state<HTMLTextAreaElement>();
  let backdrop = $state<HTMLDivElement>();

  const err = $derived(editor.dslError);
  const parts = $derived.by(() => {
    const t = editor.text;
    const e = err;
    if (!e) return null;
    const a = Math.max(0, Math.min(t.length, e.offset));
    const b = Math.min(t.length, a + Math.max(1, e.length));
    return { before: t.slice(0, a), mark: t.slice(a, b) || " ", after: t.slice(b) };
  });
  const where = $derived.by(() => {
    if (!err) return "";
    const before = editor.text.slice(0, err.offset);
    const line = before.split("\n").length;
    const col = err.offset - before.lastIndexOf("\n");
    return `line ${line}, col ${col}`;
  });

  function jumpToError(): void {
    if (!ta || !err) return;
    ta.focus();
    ta.setSelectionRange(err.offset, err.offset + Math.max(0, err.length));
  }
</script>

<div class="dsl">
  <div class="editor">
    <div class="backdrop" bind:this={backdrop} aria-hidden="true">
      {#if parts}{parts.before}<mark>{parts.mark}</mark>{parts.after}{/if}
    </div>
    <textarea
      bind:this={ta}
      value={editor.text}
      oninput={(e) => editor.editText(e.currentTarget.value)}
      onblur={() => editor.commitText()}
      onscroll={(e) => {
        if (backdrop) backdrop.scrollTop = e.currentTarget.scrollTop;
      }}
      spellcheck="false"
      aria-label="Workout DSL"
      aria-invalid={err !== null}
    ></textarea>
  </div>
  {#if err}
    <button type="button" class="error" onclick={jumpToError}>
      <span class="status critical">error</span>
      {err.message} <span class="muted">({where})</span>
    </button>
  {:else}
    <p class="ok muted">✓ valid · edits sync to the profile and outline</p>
  {/if}
</div>

<style>
  .dsl {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .editor {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  textarea,
  .backdrop {
    position: absolute;
    inset: 0;
    margin: 0;
    padding: 0.5rem 0.6rem;
    font-family: var(--mono);
    font-size: 0.82rem;
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }
  textarea {
    resize: none;
    background: transparent;
    color: var(--ink-primary);
    z-index: 1;
    overflow: auto;
  }
  .backdrop {
    color: transparent;
    background: var(--surface-1);
    overflow: hidden;
    border-color: transparent;
  }
  .backdrop mark {
    color: transparent;
    background: color-mix(in srgb, var(--status-critical) 35%, transparent);
    border-bottom: 2px solid var(--status-critical);
    border-radius: 2px;
  }
  .error {
    text-align: left;
    font-size: 0.8rem;
    background: var(--surface-2);
  }
  .ok {
    margin: 0;
    font-size: 0.75rem;
  }
</style>
