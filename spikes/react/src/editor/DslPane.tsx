import { useRef } from "react";
import { useEditor } from "./context";

/** Text DSL view: valid text updates the tree on every keystroke; errors are highlighted in place. */
export function DslPane() {
  const { state, dispatch } = useEditor();
  const back = useRef<HTMLPreElement>(null);
  const err = state.error;
  const text = state.text;
  let before = text;
  let bad = "";
  let after = "";
  let where = "";
  if (err) {
    before = text.slice(0, err.offset);
    bad = text.slice(err.offset, err.offset + Math.max(err.length, 0)) || " ";
    after = text.slice(err.offset + err.length);
    const lines = text.slice(0, err.offset).split("\n");
    where = `line ${String(lines.length)}, col ${String((lines[lines.length - 1]?.length ?? 0) + 1)}`;
  }
  return (
    <div className="dsl">
      <div className="dsl-box">
        <pre className="dsl-back" ref={back} aria-hidden>
          {before}
          {err && <mark>{bad}</mark>}
          {after}
          {"\n"}
        </pre>
        <textarea
          className="dsl-text"
          spellCheck={false}
          value={text}
          aria-label="Workout DSL"
          aria-invalid={!!err}
          onChange={(e) => { dispatch({ type: "dsl", text: e.target.value }); }}
          onScroll={(e) => {
            if (back.current) back.current.scrollTop = e.currentTarget.scrollTop;
          }}
        />
      </div>
      <p className={`dsl-status ${err ? "bad" : ""}`} role="status">
        {err ? (
          <>
            <span className="status-tag critical">✕ {where}</span> {err.message}
          </>
        ) : (
          <span className="muted">✓ parsed · tree and profile follow as you type</span>
        )}
      </p>
    </div>
  );
}
