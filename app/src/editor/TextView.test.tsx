// @vitest-environment jsdom
// The text view (ADR-0011 §2.2, §5 "App (Vitest)"; #759): a controlled view of the model's draft. It sends no change
// when the draft equals the document and none during a composition, routes `historyUndo` / `historyRedo` to the model,
// follows a model change with one external replacement that keeps a cursor outside it in place and is never sent back,
// and the lock makes it read-only.
import { EditorSelection } from "@codemirror/state";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import fc from "fast-check";
import { afterEach, expect, test, vi } from "vitest";
import { TextView, externalChange, setText, textOf, viewOf } from "./TextView";

afterEach(cleanup);

/** A text view owned by a stand-in for the editor: `draft` is what the model holds. */
function mount(draft: string, locked = false) {
  const onEdit = vi.fn();
  const onUndo = vi.fn();
  const onRedo = vi.fn();
  const view = (d: string, l: boolean) => (
    <TextView
      draft={d}
      locked={l}
      attributes={{ id: "dsl", "aria-label": "Text" }}
      onEdit={onEdit}
      onUndo={onUndo}
      onRedo={onRedo}
    />
  );
  const r = render(view(draft, locked));
  const content = r.container.querySelector<HTMLElement>("#dsl")!;
  return {
    content,
    onEdit,
    onUndo,
    onRedo,
    /** The model's draft is now `d` (an undo, an outline edit, the lock). */
    model: (d: string, l = locked) => r.rerender(view(d, l)),
  };
}
const task = () => act(() => new Promise((r) => setTimeout(r)));

test("the rider's change reaches the model; none is sent when the document equals the draft", () => {
  const t = mount("5m 50%");
  act(() => setText(t.content, "5m 60%"));
  expect(t.onEdit).toHaveBeenCalledExactlyOnceWith("5m 60%");
  // The model took it: the draft now equals the document, and following it changes nothing and sends nothing.
  t.model("5m 60%");
  expect(textOf(t.content)).toBe("5m 60%");
  // The rider types the draft's own text again (a paste over it): nothing new for the model.
  act(() => setText(t.content, "5m 60%"));
  expect(t.onEdit).toHaveBeenCalledTimes(1);
});

test("a key that brings the document back to the last draft before the model answered is still sent", () => {
  const t = mount("5m 50%");
  act(() => {
    setText(t.content, "5m 50%x");
    setText(t.content, "5m 50%");
  });
  expect(t.onEdit.mock.calls).toEqual([["5m 50%x"], ["5m 50%"]]);
});

test("a model change is dispatched into the view and never sent back as an edit", () => {
  const t = mount("5m 50%\n10m 90%");
  const v = viewOf(t.content);
  const listener = vi.fn();
  // Every transaction the view applies from now on.
  const dispatch = v.dispatch.bind(v);
  v.dispatch = ((...a: Parameters<typeof v.dispatch>) => {
    listener(...a);
    dispatch(...a);
  }) as typeof v.dispatch;
  t.model("5m 50%\n12m 90%");
  expect(textOf(t.content)).toBe("5m 50%\n12m 90%");
  expect(listener).toHaveBeenCalledTimes(1);
  expect(t.onEdit).not.toHaveBeenCalled();
});

test("the controlled view keeps a cursor outside the model's change where it was", () => {
  const t = mount('"Warm-up" 10m 50%\n3x(12m 90%)\n"Cool-down" 5m 40%');
  const v = viewOf(t.content);
  // The cursor in the first line, before the change; then in the last, after it.
  act(() => v.dispatch({ selection: EditorSelection.cursor(3) }));
  t.model('"Warm-up" 10m 50%\n4x(12m 90%)\n"Cool-down" 5m 40%');
  expect(v.state.selection.main.head).toBe(3);
  const after = v.state.doc.toString().indexOf("Cool");
  act(() => v.dispatch({ selection: EditorSelection.cursor(after) }));
  t.model('"Warm-up" 10m 50%\n4x(15m 90%, 4m 50%)\n"Cool-down" 5m 40%');
  expect(v.state.doc.sliceString(v.state.selection.main.head)).toBe(
    'Cool-down" 5m 40%',
  );
  expect(t.onEdit).not.toHaveBeenCalled();
});

test("the external change is one replacement: the old text with its region replaced is the new text", () => {
  // Over a small alphabet (with a surrogate pair), so prefixes and suffixes overlap often.
  const text = fc
    .array(fc.constantFrom("a", "b", "\n", "5", "😀"), { maxLength: 24 })
    .map((cs) => cs.join(""));
  fc.assert(
    fc.property(text, text, (from, to) => {
      const c = externalChange(from, to);
      expect(from.slice(0, c.from) + c.insert + from.slice(c.to)).toBe(to);
      expect(c.from).toBeLessThanOrEqual(c.to);
      // The kept prefix and suffix are the longest common ones (the suffix taken after the prefix), so a cursor outside
      // the change is not moved; with a surrogate pair, at most one unit less on each side, to keep the pair whole.
      const prefix = commonPrefix(from, to);
      const suffix = commonPrefix(
        [...from.slice(prefix)].reverse().join(""),
        [...to.slice(prefix)].reverse().join(""),
      );
      const exact = from.length - prefix - suffix;
      if (!/\uD83D/.test(from + to)) {
        expect(c.from).toBe(prefix);
        expect(c.to - c.from).toBe(exact);
      } else expect(c.to - c.from).toBeLessThanOrEqual(exact + 2);
      // Never inside a surrogate pair, in either text.
      const splits = (s: string, at: number) =>
        /[\uD800-\uDBFF]/.test(s.charAt(at - 1)) &&
        /[\uDC00-\uDFFF]/.test(s.charAt(at));
      expect(splits(from, c.from) || splits(from, c.to)).toBe(false);
      expect(splits(to, c.from)).toBe(false);
      // Nothing to replace when nothing changed.
      if (from === to) expect([c.to - c.from, c.insert]).toEqual([0, ""]);
    }),
    { numRuns: 512 },
  );
});
function commonPrefix(a: string, b: string) {
  let i = 0;
  while (i < a.length && i < b.length && a[i] === b[i]) i++;
  return i;
}

test("nothing is sent during a composition: the composed text reaches the model when it ends", async () => {
  const t = mount('"Cte" 5m 50%');
  fireEvent.compositionStart(t.content);
  // The dead key's mark, then the composed letter, as the DOM reports them while composing.
  act(() => setText(t.content, '"C^te" 5m 50%'));
  act(() => setText(t.content, '"Côte" 5m 50%'));
  expect(t.onEdit).not.toHaveBeenCalled();
  fireEvent.compositionEnd(t.content);
  await task();
  expect(t.onEdit).toHaveBeenCalledExactlyOnceWith('"Côte" 5m 50%');
});

test("a model change during a composition is not dispatched until it ends, so the composition is never cut", async () => {
  const t = mount("5m 50%");
  fireEvent.compositionStart(t.content);
  t.model("6m 50%");
  expect(textOf(t.content)).toBe("5m 50%");
  fireEvent.compositionEnd(t.content);
  await task();
  expect(textOf(t.content)).toBe("6m 50%");
  expect(t.onEdit).not.toHaveBeenCalled();
});

test("Edit ▸ Undo and Redo (beforeinput historyUndo / historyRedo) reach the model, not the DOM's own undo", () => {
  const t = mount("5m 50%");
  for (const inputType of ["historyUndo", "historyRedo"]) {
    const e = new InputEvent("beforeinput", {
      inputType,
      bubbles: true,
      cancelable: true,
    });
    t.content.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
  }
  expect(t.onUndo).toHaveBeenCalledTimes(1);
  expect(t.onRedo).toHaveBeenCalledTimes(1);
  expect(t.onEdit).not.toHaveBeenCalled();
});

test("the lock makes the view read-only, and lifting it makes it editable again", () => {
  const t = mount("5m 50%");
  expect(t.content.getAttribute("contenteditable")).toBe("true");
  t.model("5m 50%", true);
  const v = viewOf(t.content);
  expect(v.state.readOnly).toBe(true);
  expect(t.content.getAttribute("contenteditable")).toBe("false");
  expect(t.content.getAttribute("aria-readonly")).toBe("true");
  t.model("5m 50%", false);
  expect(v.state.readOnly).toBe(false);
  expect(t.content.getAttribute("contenteditable")).toBe("true");
  expect(t.content.hasAttribute("aria-readonly")).toBe(false);
});

test("the content is a multi-line text box with the attributes it was given", () => {
  const t = mount("5m 50%");
  expect(t.content.getAttribute("role")).toBe("textbox");
  expect(t.content.getAttribute("aria-multiline")).toBe("true");
  expect(t.content.getAttribute("aria-label")).toBe("Text");
  expect(t.content.getAttribute("spellcheck")).toBe("false");
});
