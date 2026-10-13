import { createMemo, Show } from "solid-js";
import { editor } from "./state";

/** Text DSL: editable; parse errors inline with line:col and the offending token marked. */
export function DslPane() {
  const errorView = createMemo(() => {
    const e = editor.textError();
    if (!e) return null;
    const text = editor.text();
    const before = text.slice(0, e.offset);
    const line = before.split("\n").length;
    const lineStart = before.lastIndexOf("\n") + 1;
    const lineEnd = text.indexOf("\n", e.offset);
    const lineText = text.slice(lineStart, lineEnd < 0 ? undefined : lineEnd);
    const col = e.offset - lineStart;
    return {
      ...e,
      line,
      col: col + 1,
      pre: lineText.slice(0, col),
      tok: lineText.slice(col, col + Math.max(e.length, 1)) || "␣",
      post: lineText.slice(col + Math.max(e.length, 1)),
    };
  });
  let ta: HTMLTextAreaElement | undefined;
  const jump = () => {
    const e = editor.textError();
    if (!ta || !e) return;
    ta.focus();
    ta.setSelectionRange(e.offset, e.offset + e.length);
  };

  return (
    <div class="dsl-pane card">
      <h2>
        Text <span class="muted small">DSL — edits sync both ways</span>
      </h2>
      <textarea
        ref={(el) => (ta = el)}
        class="dsl"
        classList={{ invalid: editor.textError() !== null }}
        spellcheck={false}
        value={editor.text()}
        onInput={(e) => editor.actions.setText(e.currentTarget.value)}
        aria-invalid={editor.textError() !== null}
        aria-describedby="dsl-error"
      />
      <Show when={errorView()} fallback={<div class="dsl-ok muted small">✓ parsed</div>}>
        {(e) => (
          <div class="dsl-error" id="dsl-error" role="alert" onClick={jump}>
            <div>
              <b>
                Line {e().line}:{e().col}
              </b>{" "}
              {e().message}
            </div>
            <code>
              {e().pre}
              <mark>{e().tok}</mark>
              {e().post}
            </code>
          </div>
        )}
      </Show>
    </div>
  );
}
