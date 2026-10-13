// Workouts: library + editor with three synced views (DESIGN §5.7). All edits go through
// editor/model.ts, which refuses them while an AI rewrite is pending or a diff is open (#1).
import { useEffect } from "react";
import { demoWorkout, type RewriteIntent, type WorkoutNode } from "@crank/shared";
import { Profile } from "../charts";
import { editor, editorStore } from "../editor/model";
import { mmss } from "../format";
import { backend, loadRide, navigate } from "../session";
import { useStore } from "../store";

const INTENTS: [RewriteIntent, string][] = [
  ["vary", "Vary"],
  ["harder", "Harder"],
  ["easier", "Easier"],
  ["shorter_same_stimulus", "Shorter, same stimulus"],
];

function Outline({ nodes, selected, locked, changed }: { nodes: WorkoutNode[]; selected: number | null; locked: boolean; changed: number | null }) {
  return (
    <ol className="outline" aria-label="Outline">
      {nodes.map((n, i) => (
        <li key={n.id}>
          <button className={`orow${selected === i ? " sel" : ""}${changed === i ? " changed" : ""}`} disabled={locked} onClick={() => editor.select(selected === i ? null : i)}>
            <span className="olabel">{n.label ?? (n.type === "repeat" ? `${n.count}× set` : "step")}</span>
            <code>{backend.printDsl([n]).replace(/^"(?:[^"\\]|\\.)*"\s*/, "")}</code>
          </button>
        </li>
      ))}
    </ol>
  );
}

export function Workouts() {
  const s = useStore(editorStore);
  const locked = s.lock !== null;
  const compiled = backend.compileWorkout(s.workout);
  const proposed = editor.proposed(s);
  const after = proposed ? backend.compileWorkout(proposed) : null;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "z") return;
      e.preventDefault();
      if (e.shiftKey) editor.redo();
      else editor.undo();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  return (
    <div className="page workouts">
      <aside className="library" aria-label="Library">
        <h2>Library</h2>
        <button className="lib-item" disabled={locked} onClick={() => editor.load(demoWorkout(backend.athlete.ftp))}>
          VO2 sets + threshold
        </button>
        {backend.snippets().map((sn) => (
          <button key={sn.name} className="lib-item" disabled={locked} onClick={() => editor.load({ id: `snip-${sn.name}`, name: sn.name, ftp: backend.athlete.ftp, nodes: sn.nodes })}>
            {sn.name}
          </button>
        ))}
      </aside>
      <div className="editor">
        <header className="editor-head">
          <div>
          <h1>{s.workout.name}</h1>
          <span className="big-meta">
            {after ? (
              <>
                {mmss(compiled.totalS)} → <b>{mmss(after.totalS)}</b> · TSS {Math.round(compiled.tss)} → <b>{Math.round(after.tss)}</b>
              </>
            ) : (
              `${mmss(compiled.totalS)} · TSS ${Math.round(compiled.tss)} · IF ${compiled.if.toFixed(2)}`
            )}
          </span>
          </div>
          <span className="spacer" />
          <button disabled={locked || !s.past.length} onClick={editor.undo} aria-label="Undo">
            ↶ <kbd>⌘Z</kbd>
          </button>
          <button disabled={locked || !s.future.length} onClick={editor.redo} aria-label="Redo">
            ↷ <kbd>⇧⌘Z</kbd>
          </button>
          <button className="primary" disabled={locked || !!s.error} onClick={() => (loadRide(s.workout), navigate("ready"))}>
            Ride this
          </button>
        </header>
        {s.lock ? (
          <div className="lock-banner" role="status">
            <span aria-hidden>🔒</span>
            {s.lock.kind === "pending" ? (
              <span>
                Asking the coach to make item {s.lock.path + 1} <b>{s.lock.intent.replace(/_/g, " ")}</b>… editing is locked until you decide.
              </span>
            ) : (
              <>
                <span>
                  Coach suggests a <b>{s.lock.intent.replace(/_/g, " ")}</b> version of item {s.lock.path + 1}: solid = proposed, dashed = current. Editing is locked.
                </span>
                <button className="primary" onClick={editor.accept}>
                  Accept
                </button>
                <button onClick={editor.reject}>Reject</button>
              </>
            )}
          </div>
        ) : (
          <div className="ai-bar">
            <span className="muted">{s.selection === null ? "Select an item (outline or profile) to ask the coach:" : `Item ${s.selection + 1} selected — ask the coach:`}</span>
            {INTENTS.map(([intent, label]) => (
              <button key={intent} disabled={s.selection === null || !!s.error} onClick={() => editor.requestRewrite(intent)}>
                {label}
              </button>
            ))}
            {s.notice && <span className="status-warning">{s.notice}</span>}
          </div>
        )}
        <div className="editor-profile">
          <Profile
            blocks={after?.blocks ?? compiled.blocks}
            ghost={s.lock?.kind === "diff" ? compiled.blocks.filter((b) => b.path[0] === s.lock?.path) : undefined}
            zones={backend.zones.power}
            height={150}
            selected={s.lock?.path ?? s.selection}
            onPick={locked ? undefined : (i) => editor.select(i)}
            label="Workout profile"
          />
        </div>
        <div className="editor-panes">
          <section aria-label="Outline">
            <h2>Outline</h2>
            <Outline nodes={s.workout.nodes} selected={s.selection} locked={locked} changed={s.lock?.kind === "diff" ? s.lock.path : null} />
          </section>
          <section aria-label="Text">
            <h2>Text</h2>
            <textarea className="dsl" spellCheck={false} value={s.text} readOnly={locked} aria-invalid={!!s.error} onChange={(e) => editor.setText(e.target.value)} />
            {s.error ? (
              <p className="status-serious small" role="alert">
                ⚠ {s.error.message} (at {s.error.offset}) — the profile shows the last valid version
              </p>
            ) : (
              <p className="muted small">Grammar: shared/DSL.md</p>
            )}
          </section>
          <section aria-label="Summary">
            <h2>Summary</h2>
            <p>
              {mmss(compiled.totalRangeS[0])}–{mmss(compiled.totalRangeS[1])} · TSS {Math.round(compiled.tss)} · IF {compiled.if.toFixed(2)}
            </p>
            <div className="zones">
              {compiled.timeInZoneS.map((t, i) => (
                <div key={i} className="zone-row">
                  <span>Z{i + 1}</span>
                  <i style={{ width: `${(t / Math.max(1, compiled.totalS)) * 100}%`, background: `var(--zone-${i + 1})` }} />
                  <span>{mmss(t)}</span>
                </div>
              ))}
            </div>
            {compiled.issues.map((iss, i) => (
              <p key={i} className={iss.severity === "error" ? "status-serious small" : "status-warning small"}>
                ⚠ {iss.message}
              </p>
            ))}
          </section>
        </div>
      </div>
    </div>
  );
}
