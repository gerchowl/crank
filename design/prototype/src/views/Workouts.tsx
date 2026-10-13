// Workouts: library + editor with three synced views (DESIGN §5.7). All edits go through
// editor/model.ts, which refuses them while an AI rewrite is pending or a diff is open (#1).
// An open suggestion is explained before it can be accepted: change summary, stacked
// Current / Proposed profiles, text + outline diff, and a labelled before → after summary.
// Below 1280 px the library is a drawer over the panes (☰ Library), so Outline | Text | Summary stay in one row
// (DESIGN §16.3, #111). The open drawer is modal: focus moves into it, Tab stays inside, and Esc, ✕, a pick or the
// scrim close it and return focus to ☰ Library.
import { useEffect, useRef, useSyncExternalStore, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { demoWorkout, type CompiledWorkout, type RewriteIntent, type WorkoutNode } from "@crank/shared";
import { Profile } from "../charts";
import { editor, editorStore, type EditorState } from "../editor/model";
import { mmss, signed } from "../format";
import { humanToken } from "../mock/ext";
import { backend } from "../session";
import { requestStart } from "../safety";
import { createStore, useStore } from "../store";

/** The library drawer (< 1280 px). At ≥ 1280 px the library is a column and this has no effect. */
export const libraryOpen = createStore(false);

const DRAWER_MEDIA = "(max-width: 1279px)";
// Crossing to ≥ 1280 px closes the drawer. Owned here, with the state, not by the mounted view: it must also close
// while another place is showing, or it would come back as a modal when the window shrinks again (DESIGN §16.3, #189).
const closeOnWide = (e: MediaQueryListEvent) => {
  if (!e.matches) libraryOpen.set(false);
};
const drawerQuery = matchMedia(DRAWER_MEDIA);
drawerQuery.addEventListener("change", closeOnWide);
// a hot reload re-runs this module: drop the old listener so they do not accumulate
import.meta.hot?.dispose(() => drawerQuery.removeEventListener("change", closeOnWide));
/** True while the library is a drawer (the window is below 1280 px). */
function useDrawerMode() {
  return useSyncExternalStore(
    (cb) => {
      const m = matchMedia(DRAWER_MEDIA);
      m.addEventListener("change", cb);
      return () => m.removeEventListener("change", cb);
    },
    () => matchMedia(DRAWER_MEDIA).matches,
  );
}

const INTENTS: [RewriteIntent, string][] = [
  ["vary", "Vary"],
  ["harder", "Harder"],
  ["easier", "Easier"],
  ["shorter_same_stimulus", "Shorter, same stimulus"],
];
const body = (n: WorkoutNode) => backend.printDsl([n]).replace(/^"(?:[^"\\]|\\.)*"\s*/, "");
const label = (n: WorkoutNode) => n.label ?? (n.type === "repeat" ? `${n.count}× set` : "step");

function Outline({ s, locked }: { s: EditorState; locked: boolean }) {
  const diff = s.lock?.kind === "diff" ? s.lock : null;
  return (
    <ol className="outline" aria-label="Outline">
      {s.workout.nodes.map((n, i) => (
        <li key={n.id}>
          <button className={`orow${s.selection === i ? " sel" : ""}${diff?.path === i ? " old" : ""}`} disabled={locked} onClick={() => editor.select(s.selection === i ? null : i)}>
            <span className="olabel">
              {label(n)} {diff?.path === i && <span className="tag">current</span>}
            </span>
            <code>{body(n)}</code>
          </button>
          {diff?.path === i &&
            diff.proposal.map((p) => (
              <div key={p.id} className="orow new">
                <span className="olabel">
                  {label(p)} <span className="tag">proposed</span>
                </span>
                <code>{body(p)}</code>
              </div>
            ))}
        </li>
      ))}
    </ol>
  );
}

function Summary({ c, title }: { c: CompiledWorkout; title: string }) {
  return (
    <div className="summary-col">
      <h3>{title}</h3>
      <p>
        {mmss(c.totalRangeS[0])}–{mmss(c.totalRangeS[1])}
        <br />
        TSS {Math.round(c.tss)} · IF {c.if.toFixed(2)}
      </p>
      <div className="zones">
        {c.timeInZoneS.map((t, i) => (
          <div key={i} className="zone-row">
            <span>Z{i + 1}</span>
            <i style={{ width: `${(t / Math.max(1, c.totalS)) * 100}%`, background: `var(--zone-${i + 1})` }} />
            <span>{mmss(t)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

/**
 * Short summary of the open suggestion: the tokens that changed, the time and the load
 * (production: the planner's rewrite rationale, #29).
 */
function changeSummary(s: EditorState, before: CompiledWorkout, after: CompiledWorkout) {
  if (s.lock?.kind !== "diff") return "";
  const old = s.workout.nodes[s.lock.path];
  const tok = (t: string) => t.split(/[\s,()]+/).filter(Boolean);
  const a = tok(old ? body(old) : "");
  const b = tok(s.lock.proposal.map(body).join(" "));
  const changed = a.length === b.length ? a.flatMap((x, i) => (x !== b[i] ? [`${humanToken(x)} → ${humanToken(b[i] ?? "")}`] : [])) : [];
  const dT = after.totalS - before.totalS;
  return [
    `${old ? label(old) : "Item"}${changed.length ? `: ${changed.join(", ")}` : " rewritten"}`,
    `${dT >= 0 ? "+" : "−"}${mmss(Math.abs(dT))}`,
    `load ${Math.round(before.tss)} → ${Math.round(after.tss)} TSS (${signed(after.tss - before.tss)})`,
  ].join(" · ");
}

export function Workouts() {
  const s = useStore(editorStore);
  const locked = s.lock !== null;
  const compiled = backend.compileWorkout(s.workout);
  const proposed = editor.proposed(s);
  const after = proposed ? backend.compileWorkout(proposed) : null;
  const diffPath = s.lock?.kind === "diff" ? s.lock.path : null;
  const span = Math.max(compiled.totalS, after?.totalS ?? 0);
  const drawer = useStore(libraryOpen);
  const drawerMode = useDrawerMode();
  const modal = drawer && drawerMode;
  const libRef = useRef<HTMLElement>(null);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const closeLibrary = () => libraryOpen.set(false);
  const load = (w: Parameters<typeof editor.load>[0]) => (editor.load(w), closeLibrary());
  const focusables = () => [...(libRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
  // the open drawer takes focus (its first item, else ✕); once closed (and the editor no longer inert), ☰ Library has it
  const wasModal = useRef(false);
  useEffect(() => {
    if (modal) {
      const f = focusables();
      (f.find((b) => b.classList.contains("lib-item")) ?? f[0])?.focus();
    } else if (wasModal.current) toggleRef.current?.focus();
    wasModal.current = modal;
  }, [modal]);
  // Tab and Shift+Tab wrap inside the drawer while it overlays the editor
  const trapTab = (e: ReactKeyboardEvent) => {
    if (!modal || e.key !== "Tab") return;
    const f = focusables();
    if (!f.length) return;
    const i = f.indexOf(document.activeElement as HTMLButtonElement);
    const next = e.shiftKey ? (i <= 0 ? f.length - 1 : i - 1) : i === f.length - 1 ? 0 : i + 1;
    e.preventDefault();
    f[next]?.focus();
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && libraryOpen.get()) return libraryOpen.set(false);
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "z") return;
      e.preventDefault();
      if (e.shiftKey) editor.redo();
      else editor.undo();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="page workouts">
      {modal && <div className="lib-scrim" aria-hidden onClick={closeLibrary} />}
      <aside ref={libRef} className="library" aria-label="Library" data-open={drawer || undefined} role={modal ? "dialog" : undefined} aria-modal={modal || undefined} onKeyDown={trapTab}>
        <h2>
          Library{" "}
          <button className="lib-close" onClick={closeLibrary} aria-label="Close the library">
            ✕
          </button>
        </h2>
        <button className="lib-item" disabled={locked} onClick={() => load(demoWorkout(backend.athlete.ftp))}>
          VO2 sets + threshold
        </button>
        {backend.snippets().map((sn) => (
          <button key={sn.name} className="lib-item" disabled={locked} onClick={() => load({ id: `snip-${sn.name}`, name: sn.name, ftp: backend.athlete.ftp, nodes: sn.nodes })}>
            {sn.name}
          </button>
        ))}
      </aside>
      <div className="editor" inert={modal || undefined}>
        <header className="editor-head">
          <button ref={toggleRef} className="lib-toggle" aria-expanded={drawer} onClick={() => libraryOpen.set(!drawer)}>
            ☰ Library
          </button>
          <div>
            <h1>{s.workout.name}</h1>
            <span className="big-meta">
              current {mmss(compiled.totalRangeS[0])}–{mmss(compiled.totalRangeS[1])} · TSS {Math.round(compiled.tss)}
              {after && (
                <>
                  {" "}
                  → proposed <b>
                    {mmss(after.totalRangeS[0])}–{mmss(after.totalRangeS[1])} · TSS {Math.round(after.tss)}
                  </b>
                </>
              )}
            </span>
          </div>
          <span className="spacer" />
          <button disabled={locked || !s.past.length} onClick={editor.undo} aria-label="Undo">
            Undo <kbd>⌘Z</kbd>
          </button>
          <button disabled={locked || !s.future.length} onClick={editor.redo} aria-label="Redo">
            Redo <kbd>⇧⌘Z</kbd>
          </button>
          <button className="primary" disabled={locked || !!s.error} onClick={() => requestStart(s.workout, "workouts")}>
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
                  <b>Coach suggests ({s.lock.intent.replace(/_/g, " ")}):</b> {after && changeSummary(s, compiled, after)}. Exact text below. Editing is locked.
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
            {INTENTS.map(([intent, l]) => (
              <button key={intent} disabled={s.selection === null || !!s.error} onClick={() => editor.requestRewrite(intent)}>
                {l}
              </button>
            ))}
            {s.notice && <span className="status-warning">{s.notice}</span>}
          </div>
        )}
        <div className="editor-profile">
          {after && <span className="profile-tag">Current</span>}
          <Profile blocks={compiled.blocks} toS={span} zones={backend.zones.power} height={after ? 90 : 150} selected={diffPath ?? s.selection} onPick={locked ? undefined : (i) => editor.select(i)} label="Current workout profile" />
          {after && (
            <>
              <span className="profile-tag">Proposed</span>
              <Profile blocks={after.blocks} toS={span} zones={backend.zones.power} height={90} selected={diffPath} label="Proposed workout profile" />
            </>
          )}
        </div>
        <div className="editor-panes">
          <section aria-label="Outline">
            <h2>Outline</h2>
            <Outline s={s} locked={locked} />
          </section>
          <section aria-label="Text">
            <h2>Text</h2>
            {s.lock?.kind === "diff" ? (
              <pre className="dsl diff" aria-label="Text diff">
                {s.workout.nodes.map((n, i) =>
                  i === diffPath ? (
                    <span key={n.id}>
                      <del>{backend.printDsl([n])}</del>
                      {"\n"}
                      <ins>{backend.printDsl(s.lock?.kind === "diff" ? s.lock.proposal : [])}</ins>
                      {"\n"}
                    </span>
                  ) : (
                    <span key={n.id}>{backend.printDsl([n])}{"\n"}</span>
                  ),
                )}
              </pre>
            ) : (
              <textarea className="dsl" spellCheck={false} value={s.text} readOnly={locked} aria-invalid={!!s.error} onChange={(e) => editor.setText(e.target.value)} />
            )}
            {s.error ? (
              <p className="status-serious small" role="alert">
                ⚠ {s.error.message} (at {s.error.offset}) — the profile shows the last valid version
              </p>
            ) : (
              <p className="muted small">{s.lock?.kind === "diff" ? "struck = current · highlighted = proposed" : "Grammar: shared/DSL.md"}</p>
            )}
          </section>
          <section aria-label="Summary" className="summary">
            <h2>Summary</h2>
            <div className="summary-cols">
              <Summary c={compiled} title={after ? "Current" : "This workout"} />
              {after && <Summary c={after} title="Proposed" />}
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
