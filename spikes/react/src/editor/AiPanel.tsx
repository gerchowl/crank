import { useTransition } from "react";
import type { RewriteIntent } from "@neo/shared";
import { backend } from "../backend";
import { useEditor } from "./context";
import { allIds, replaceRun, runNodes, siblingRun } from "./tree";

const INTENTS: [RewriteIntent, string][] = [
  ["vary", "Vary"],
  ["harder", "Harder"],
  ["easier", "Easier"],
  ["shorter_same_stimulus", "Shorter, same stimulus"],
];

/** AI on selection → planner.rewrite → diff (ghost vs new on the profile, outline highlight) → accept / reject. */
export function AiPanel() {
  const { state, dispatch } = useEditor();
  const [pending, startTransition] = useTransition();
  const run = siblingRun(state.nodes, state.selection);
  const diff = state.diff;

  const ask = (intent: RewriteIntent) => {
    if (!run) return;
    const before = state.nodes;
    // React 19 async transition: isPending covers the planner's latency.
    startTransition(async () => {
      const out = await backend.planner.rewrite(runNodes(before, run), intent);
      startTransition(() => {
        dispatch({ type: "diff", diff: { before, after: replaceRun(before, run, out), changed: allIds(out), top: out.map((n) => n.id), intent } });
      });
    });
  };

  return (
    <section className="ai">
      <h4>✦ AI on selection</h4>
      {diff ? (
        <div className="ai-diff">
          <p>
            Proposed <b>{diff.intent.replaceAll("_", " ")}</b> rewrite of {diff.top.length} item(s). The profile shows the old version ghosted.
          </p>
          <div className="row">
            <button type="button" className="btn primary" onClick={() => { dispatch({ type: "accept_diff" }); }}>✓ Accept</button>
            <button type="button" className="btn" onClick={() => { dispatch({ type: "diff", diff: null }); }}>✕ Reject</button>
          </div>
        </div>
      ) : (
        <>
          <div className="row wrap">
            {INTENTS.map(([k, label]) => (
              <button type="button" key={k} className="btn" disabled={!run || pending} onClick={() => { ask(k); }}>
                {label}
              </button>
            ))}
          </div>
          <p className="muted small" aria-live="polite">
            {pending ? "Asking the planner…" : run ? `${String(run.to - run.from + 1)} item(s) selected` : "Select one or more sibling items in the outline or profile."}
          </p>
        </>
      )}
    </section>
  );
}
