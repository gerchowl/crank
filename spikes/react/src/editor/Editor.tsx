import { useEffect, useEffectEvent, useMemo, useReducer } from "react";
import { demoWorkout, type Workout } from "@neo/shared";
import { backend } from "../backend";
import { editorSeed } from "../lib/session";
import { go } from "../lib/router";
import { forceRefreshCurrent } from "../lib/live";
import { EditorContext } from "./context";
import { editorReducer, initEditor } from "./state";
import { duplicate, removeIds, siblingRun, unwrap, wrap, nodeAt, pathOf } from "./tree";
import { Profile } from "./Profile";
import { Outline } from "./Outline";
import { DslPane } from "./DslPane";
import { Summary } from "./Summary";
import { AiPanel } from "./AiPanel";
import { Snippets } from "./Snippets";

function initialWorkout(): Workout {
  // read-only here: initializers run twice under StrictMode; the seed is consumed in an effect
  return editorSeed.get() ?? backend.ride.current()?.workout ?? demoWorkout(backend.athlete.ftp);
}

let wrapSeq = 0;

/** V4 workout editor: profile + outline + DSL in sync, one reducer, undo/redo across all views. */
export function Editor() {
  const [state, dispatch] = useReducer(editorReducer, null, () => initEditor(initialWorkout()));
  const compiled = useMemo(() => backend.compileWorkout({ ...state.workout, nodes: state.nodes }), [state.workout, state.nodes]);
  const ctx = useMemo(() => ({ state, dispatch, compiled }), [state, compiled]);

  const sel = state.selection;
  const run = siblingRun(state.nodes, sel);
  const selPath = sel.length === 1 && sel[0] ? pathOf(state.nodes, sel[0]) : null;
  const selNode = selPath ? nodeAt(state.nodes, selPath) : undefined;
  const busy = !!state.diff;

  const act = {
    wrap: () => {
      const out = wrap(state.nodes, sel, `wrap-${String(++wrapSeq)}-${String(Date.now())}`);
      if (out) dispatch({ type: "edit", nodes: out });
    },
    unwrap: () => {
      if (selNode?.type !== "repeat") return;
      const out = unwrap(state.nodes, selNode.id);
      if (out) dispatch({ type: "edit", nodes: out, selection: selNode.children.map((c) => c.id) });
    },
    duplicate: () => {
      const out = duplicate(state.nodes, sel);
      if (out) dispatch({ type: "edit", nodes: out.nodes, selection: out.ids });
    },
    remove: () => {
      if (sel.length) dispatch({ type: "edit", nodes: removeIds(state.nodes, new Set(sel)), selection: [] });
    },
  };

  const onKey = useEffectEvent((e: KeyboardEvent) => {
    const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;
    const mod = e.metaKey || e.ctrlKey;
    const k = e.key.toLowerCase();
    if (mod && k === "z") {
      // undo/redo work everywhere except while editing a single field's native text
      if (typing && !(e.target instanceof HTMLTextAreaElement)) return;
      e.preventDefault();
      dispatch({ type: e.shiftKey ? "redo" : "undo" });
    } else if (mod && k === "y") {
      e.preventDefault();
      dispatch({ type: "redo" });
    } else if (typing || busy) {
      return;
    } else if (mod && k === "g") {
      e.preventDefault();
      if (e.shiftKey) act.unwrap();
      else act.wrap();
    } else if (mod && k === "d") {
      e.preventDefault();
      act.duplicate();
    } else if (k === "backspace" || k === "delete") {
      e.preventDefault();
      act.remove();
    } else if (k === "escape") {
      dispatch({ type: "select", ids: [] });
    }
  });
  useEffect(() => {
    editorSeed.set(null);
  }, []);
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      onKey(e);
    };
    addEventListener("keydown", h);
    return () => {
      removeEventListener("keydown", h);
    };
  }, []);

  return (
    <EditorContext value={ctx}>
      <div className="editor">
        <header className="ed-toolbar">
          <input className="ed-name" value={state.workout.name} aria-label="workout name" onChange={(e) => { dispatch({ type: "rename", name: e.target.value }); }} />
          <div className="btn-group">
            <button type="button" className="btn" disabled={!state.past.length} onClick={() => { dispatch({ type: "undo" }); }} title="Undo (⌘Z)">↶ Undo</button>
            <button type="button" className="btn" disabled={!state.future.length} onClick={() => { dispatch({ type: "redo" }); }} title="Redo (⇧⌘Z)">↷ Redo</button>
          </div>
          <div className="btn-group">
            <button type="button" className="btn" disabled={!run || busy} onClick={act.wrap} title="Wrap in repeat (⌘G)">⟦ Wrap</button>
            <button type="button" className="btn" disabled={selNode?.type !== "repeat" || busy} onClick={act.unwrap} title="Unwrap (⇧⌘G)">⟧ Unwrap</button>
            <button type="button" className="btn" disabled={!run || busy} onClick={act.duplicate} title="Duplicate (⌘D)">⧉ Duplicate</button>
            <button type="button" className="btn" disabled={!sel.length || busy} onClick={act.remove} title="Delete (⌫)">✕ Delete</button>
          </div>
          <span className="spacer" />
          <button
            type="button"
            className="btn primary"
            disabled={busy || compiled.issues.some((i) => i.severity === "error")}
            onClick={() => {
              backend.ride.load({ ...state.workout, nodes: state.nodes });
              backend.ride.start();
              forceRefreshCurrent();
              go("ride");
            }}
          >
            ▶ Ride this
          </button>
        </header>
        <Profile />
        <div className="ed-cols">
          <section className="ed-pane">
            <h4>Outline</h4>
            <Outline />
          </section>
          <section className="ed-pane">
            <h4>Text</h4>
            <DslPane />
          </section>
          <section className="ed-pane side">
            <Summary />
            <AiPanel />
            <Snippets />
          </section>
        </div>
      </div>
    </EditorContext>
  );
}
