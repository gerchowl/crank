import { createSignal, For, Match, Show, Switch } from "solid-js";
import { reId, type RewriteIntent } from "@neo/shared";
import { createKeydown } from "../lib/primitives";
import { ride } from "../state/ride";
import { router } from "../state/router";
import { HelpOverlay } from "../ui/HelpOverlay";
import { DslPane } from "./DslPane";
import { Outline } from "./Outline";
import { ProfileEditor } from "./ProfileEditor";
import { editor } from "./state";
import { Summary } from "./Summary";
import { siblingRange } from "./tree";

const INTENTS: { intent: RewriteIntent; label: string }[] = [
  { intent: "vary", label: "Vary" },
  { intent: "harder", label: "Harder" },
  { intent: "easier", label: "Easier" },
  { intent: "shorter_same_stimulus", label: "Shorter, same stimulus" },
];

const SHORTCUTS: [string, string][] = [
  ["⌘/Ctrl Z", "Undo"],
  ["⇧⌘/Ctrl Z", "Redo"],
  ["⌘/Ctrl G", "Wrap selection in repeat"],
  ["⇧⌘/Ctrl G", "Unwrap repeat"],
  ["⌘/Ctrl D", "Duplicate selection"],
  ["Delete", "Delete selection"],
  ["Shift-click", "Add to selection (siblings)"],
  ["Esc", "Clear selection / reject AI proposal"],
  ["?", "This overlay"],
];

/** V4 — workout editor: profile + outline + DSL in sync, summary, AI rewrite with diff, snippets. */
export function Editor() {
  const [help, setHelp] = createSignal(false);
  const [snippetName, setSnippetName] = createSignal("");
  const hasSel = () => editor.selection().size > 0;
  const siblings = () => siblingRange(editor.nodes(), editor.selection()) !== null;
  const canUnwrap = () => {
    const [id] = [...editor.selection()];
    return editor.selection().size === 1 && id !== undefined && editor.nodes().length > 0 && isRepeat(id);
  };
  const isRepeat = (id: string) => {
    const stack = [...editor.nodes()];
    while (stack.length) {
      const n = stack.pop();
      if (!n) break;
      if (n.id === id) return n.type === "repeat";
      if (n.type === "repeat") stack.push(...n.children);
    }
    return false;
  };

  createKeydown((e) => {
    const mod = e.metaKey || e.ctrlKey;
    const inField = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLSelectElement;
    const key = e.key.toLowerCase();
    if (mod && key === "z") {
      e.preventDefault();
      if (e.shiftKey) editor.actions.redo();
      else editor.actions.undo();
    } else if (mod && key === "y") {
      e.preventDefault();
      editor.actions.redo();
    } else if (mod && key === "g") {
      e.preventDefault();
      if (e.shiftKey) editor.actions.unwrap();
      else editor.actions.wrap();
    } else if (mod && key === "d") {
      e.preventDefault();
      editor.actions.duplicate();
    } else if (inField) {
      return;
    } else if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      editor.actions.remove();
    } else if (e.key === "Escape") {
      if (help()) setHelp(false);
      else if (editor.rewriteReq()) editor.actions.rejectRewrite();
      else editor.actions.clearSelection();
    } else if (e.key === "?") {
      setHelp(!help());
    }
  });

  const rideThis = () => {
    ride.controls.load({ ...editor.doc, nodes: structuredClone([...editor.nodes()]) });
    router.go("ride");
  };

  return (
    <div class="page editor">
      <header class="editor-head">
        <input class="text title-input" value={editor.doc.name} onChange={(e) => editor.actions.rename(e.currentTarget.value)} aria-label="Workout name" />
        <div class="toolbar" role="toolbar" aria-label="Edit">
          <button onClick={() => editor.actions.undo()} disabled={!editor.canUndo()} title="Undo (⌘Z)">
            ↶ Undo
          </button>
          <button onClick={() => editor.actions.redo()} disabled={!editor.canRedo()} title="Redo (⇧⌘Z)">
            ↷ Redo
          </button>
          <span class="tb-sep" />
          <button onClick={() => editor.actions.wrap()} disabled={!siblings()} title="Wrap in repeat (⌘G)">
            ⟦ ⟧ Wrap
          </button>
          <button onClick={() => editor.actions.unwrap()} disabled={!canUnwrap()} title="Unwrap (⇧⌘G)">
            Unwrap
          </button>
          <button onClick={() => editor.actions.duplicate()} disabled={!siblings()} title="Duplicate (⌘D)">
            Duplicate
          </button>
          <button onClick={() => editor.actions.remove()} disabled={!hasSel()} title="Delete (Del)">
            Delete
          </button>
          <span class="tb-sep" />
          <span class="ai-group" aria-label="AI on selection">
            <span class="muted small">AI on selection</span>
            <For each={INTENTS}>
              {(it) => (
                <button class="ai" disabled={!siblings() || editor.rewrite.loading} onClick={() => editor.actions.requestRewrite(it.intent)}>
                  ✦ {it.label}
                </button>
              )}
            </For>
          </span>
          <span class="tb-sep" />
          <button class="primary" onClick={rideThis}>
            ▶ Ride this
          </button>
          <button class="ghost" onClick={() => setHelp(true)} title="Shortcuts (?)">
            ?
          </button>
        </div>
      </header>

      <Switch>
        <Match when={editor.rewrite.loading}>
          <div class="diff-banner loading" role="status">
            ✦ Asking the planner to {editor.rewriteReq()?.intent.replaceAll("_", " ")} the selection…
          </div>
        </Match>
        <Match when={editor.rewrite.error !== undefined && editor.rewriteReq()}>
          <div class="diff-banner" role="alert">
            Rewrite failed. <button onClick={() => editor.actions.rejectRewrite()}>Dismiss</button>
          </div>
        </Match>
        <Match when={editor.diff()}>
          {(d) => (
            <div class="diff-banner" role="region" aria-label="AI proposal">
              <span>
                ✦ AI proposal: <b>{d().req.intent.replaceAll("_", " ")}</b> — old profile ghosted, changed blocks outlined.
              </span>
              <span class="diff-actions">
                <button class="primary" onClick={() => editor.actions.acceptRewrite()}>
                  ✓ Accept
                </button>
                <button onClick={() => editor.actions.rejectRewrite()}>✕ Reject (Esc)</button>
              </span>
            </div>
          )}
        </Match>
      </Switch>

      <div class="card profile-card">
        <h2>
          Profile <span class="muted small">click to select · drag a block's right edge for duration, its top for power</span>
        </h2>
        <ProfileEditor />
      </div>

      <div class="editor-grid">
        <div class="card outline-card">
          <h2>
            Outline <span class="muted small">drag to reorder · drop on a repeat's middle to move inside</span>
          </h2>
          <Show when={editor.diff()} fallback={<Outline nodes={editor.nodes()} />}>
            {(d) => (
              <>
                <p class="muted small">Proposed tree (highlighted = new from AI):</p>
                <Outline nodes={d().after} readOnly changed={d().changed} />
              </>
            )}
          </Show>
          <div class="snippets">
            <h2>Snippets</h2>
            <div class="chips">
              <For each={editor.snippets()}>
                {(s) => (
                  <button class="mini" onClick={() => editor.actions.insertNodes(reId(s.nodes))} title="Insert after selection">
                    + {s.name}
                  </button>
                )}
              </For>
            </div>
            <form
              class="snippet-save"
              onSubmit={(e) => {
                e.preventDefault();
                if (!snippetName().trim()) return;
                editor.actions.saveSnippet(snippetName().trim());
                setSnippetName("");
              }}
            >
              <input class="text" placeholder="Save selection as snippet…" value={snippetName()} onInput={(e) => setSnippetName(e.currentTarget.value)} disabled={!siblings()} />
              <button type="submit" disabled={!siblings() || !snippetName().trim()}>
                Save
              </button>
            </form>
          </div>
        </div>
        <DslPane />
        <Summary compiled={editor.preview()} />
      </div>

      <Show when={help()}>
        <HelpOverlay title="Editor shortcuts" shortcuts={SHORTCUTS} onClose={() => setHelp(false)} />
      </Show>
    </div>
  );
}
