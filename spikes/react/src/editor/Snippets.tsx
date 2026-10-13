import { useState, useSyncExternalStore } from "react";
import { reId, type WorkoutNode } from "@neo/shared";
import { backend } from "../backend";
import { createStore } from "../lib/store";
import { useEditor } from "./context";
import { insertAfter, runNodes, siblingRun } from "./tree";

interface Snippet { name: string; nodes: WorkoutNode[] }
const KEY = "neo-trainer.snippets.v1";

function load(): Snippet[] {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "[]") as Snippet[];
  } catch {
    return [];
  }
}
const userSnippets = createStore<Snippet[]>(load());
userSnippets.subscribe(() => {
  localStorage.setItem(KEY, JSON.stringify(userSnippets.get()));
});
const seed = backend.snippets();

/** Snippet library: seed (30/15s, 4×4, over-unders, Tabata) + saved selections. */
export function Snippets() {
  const { state, dispatch } = useEditor();
  const mine = useSyncExternalStore(userSnippets.subscribe, userSnippets.get);
  const [name, setName] = useState("");
  const run = siblingRun(state.nodes, state.selection);
  const last = state.selection[state.selection.length - 1] ?? null;
  const insert = (s: Snippet) => {
    const ins = reId(s.nodes);
    dispatch({ type: "edit", nodes: insertAfter(state.nodes, last, ins), selection: ins.map((n) => n.id) });
  };
  return (
    <section className="snippets">
      <h4>Snippets</h4>
      <ul>
        {[...seed, ...mine].map((s, i) => (
          <li key={`${s.name}-${String(i)}`}>
            <button type="button" className="btn small" disabled={!!state.diff} onClick={() => { insert(s); }} title={backend.printDsl(s.nodes)}>
              + {s.name}
            </button>
          </li>
        ))}
      </ul>
      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          if (!run || !name.trim()) return;
          userSnippets.set((xs) => [...xs, { name: name.trim(), nodes: structuredClone(runNodes(state.nodes, run)) }]);
          setName("");
        }}
      >
        <input type="text" value={name} placeholder="name" aria-label="snippet name" onChange={(e) => { setName(e.target.value); }} />
        <button type="submit" className="btn small" disabled={!run || !name.trim()}>Save selection</button>
      </form>
    </section>
  );
}
