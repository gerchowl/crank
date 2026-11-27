// The editor's text view (ADR-0011 §2.1, §2.2, §2.7; #759): CodeMirror 6 as a controlled view of the model's draft.
//
// - One history, the model's (§2.2): no `history()` extension and no undo key here. The editor root's Cmd/Ctrl+Z
//   listener stays the only undo key; `beforeinput` `historyUndo` / `historyRedo` (how WebKit delivers Edit ▸ Undo and
//   the menu's Cmd+Z to a contenteditable) are prevented and routed to `onUndo` / `onRedo`, so the DOM's own undo
//   never runs.
// - The draft is the truth. A change the rider makes here reaches the model as `onEdit(text)` (the editor's
//   `edit(text, at)`). A change that comes from the model (undo, redo, an outline edit, load, an AI rewrite) is
//   dispatched in as one replacement of the changed region (`externalChange`: the common prefix and suffix are kept,
//   so a cursor outside the change stays put), annotated `external`; the update listener never calls `onEdit` for it.
// - Nothing is dispatched when the draft equals the document (the change came from here), and nothing in either
//   direction while a composition (IME, dead key) is in progress: the composed text reaches the model when it ends,
//   then the view follows the model's draft again, so a composition is never cut.
// - The AI lock (#112) makes the view read-only (`EditorState.readOnly`, `EditorView.editable`, in a compartment).
// - Tab is not bound, so Tab and Shift+Tab move focus as from any text box (§2.7, WCAG 2.1.2); the box is named by its
//   heading (`aria-labelledby`: a `<label for>` cannot name a `div`) and described by the problem under it.
import { Annotation, Compartment, EditorState } from "@codemirror/state";
import { EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { useEffect, useRef } from "react";

/** Marks a transaction that follows the model: the view never reports it back as an edit. */
const external = Annotation.define<boolean>();

/**
 * The one replacement that turns `from` into `to`: the longest common prefix, then the longest common suffix of what
 * is left, are kept. Splitting a surrogate pair is avoided, so the inserted text is always whole characters.
 */
export function externalChange(
  from: string,
  to: string,
): { from: number; to: number; insert: string } {
  const max = Math.min(from.length, to.length);
  let start = 0;
  while (start < max && from.charCodeAt(start) === to.charCodeAt(start))
    start++;
  if (start > 0 && isHigh(from.charCodeAt(start - 1))) start--;
  let end = 0;
  while (
    end < max - start &&
    from.charCodeAt(from.length - 1 - end) ===
      to.charCodeAt(to.length - 1 - end)
  )
    end++;
  if (end > 0 && isLow(from.charCodeAt(from.length - end))) end--;
  return {
    from: start,
    to: from.length - end,
    insert: to.slice(start, to.length - end),
  };
}
const isHigh = (c: number) => c >= 0xd800 && c <= 0xdbff;
const isLow = (c: number) => c >= 0xdc00 && c <= 0xdfff;

/** What the view is told by its owner on every render. */
interface Props {
  /** The model's draft: the text the view shows. */
  draft: string;
  /** The rider changed the text here (never called for a change that came from `draft`). */
  onEdit: (text: string) => void;
  onUndo: () => void;
  onRedo: () => void;
  /** The AI rewrite's lock: the text can be selected and copied, not changed. */
  locked: boolean;
  /** The content's attributes beyond the fixed ones: its id, name, description and validity. */
  attributes: Record<string, string>;
}

/** The view's link to its owner's latest props: the plugin reads them when the rider types. */
interface Owner {
  current: Props;
}

/**
 * Keeps the view and the model's draft in step (header comment). `follow` is called on every render with the draft;
 * the update listener reports the rider's changes.
 */
class Follow {
  /** A rider's change held back while a composition is in progress. */
  held = false;
  /** The text the model was last given or last showed: a document equal to it is not sent again. */
  known: string;
  constructor(
    readonly view: EditorView,
    readonly owner: Owner,
  ) {
    this.known = view.state.doc.toString();
  }
  update(u: ViewUpdate) {
    const mine = u.transactions.some(
      (tr) => tr.docChanged && !tr.annotation(external),
    );
    if (!mine) return;
    if (u.view.compositionStarted) this.held = true;
    else this.send();
  }
  /** Report the document to the model, unless the model has it already. */
  send() {
    this.held = false;
    const text = this.view.state.doc.toString();
    if (text === this.known) return;
    this.known = text;
    this.owner.current.onEdit(text);
  }
  /** A composition ended: what it composed reaches the model, else the view follows the model again. */
  settle() {
    if (this.view.compositionStarted) return;
    if (this.held) this.send();
    else this.follow();
  }
  /** Show the model's draft: one external replacement, never while composing, nothing if it is already shown. */
  follow() {
    const { view } = this;
    const draft = this.owner.current.draft;
    this.known = draft;
    if (view.compositionStarted) return;
    const doc = view.state.doc.toString();
    if (doc === draft) return;
    view.dispatch({
      changes: externalChange(doc, draft),
      annotations: external.of(true),
    });
  }
}

const followPlugin = (owner: Owner) =>
  ViewPlugin.define((view) => new Follow(view, owner), {
    eventObservers: {
      // CodeMirror reads the composition's last DOM change in a microtask after `compositionend` (and WebKit can drop
      // the event: CodeMirror then ends the composition itself from the next `beforeinput`, whose change the update
      // listener sees with the composition over). A task later, every change of the composition has been read.
      compositionend() {
        setTimeout(() => this.settle());
      },
    },
    eventHandlers: {
      beforeinput(e) {
        if (e.inputType !== "historyUndo" && e.inputType !== "historyRedo")
          return false;
        e.preventDefault();
        if (e.inputType === "historyUndo") this.owner.current.onUndo();
        else this.owner.current.onRedo();
        return true;
      },
    },
  });

const lockOf = (locked: boolean) => [
  EditorState.readOnly.of(locked),
  EditorView.editable.of(!locked),
];
const attributesOf = (p: Props) =>
  EditorView.contentAttributes.of({
    spellcheck: "false",
    autocorrect: "off",
    autocapitalize: "off",
    ...(p.locked ? { "aria-readonly": "true" } : {}),
    ...p.attributes,
  });

/** The DSL text box (header comment). */
export function TextView(props: Props) {
  const host = useRef<HTMLDivElement>(null);
  const owner = useRef<Props>(props);
  const view = useRef<{ view: EditorView; follow: Follow } | null>(null);
  const lock = useRef(new Compartment());
  const attrs = useRef(new Compartment());
  useEffect(() => {
    owner.current = props;
  });
  useEffect(() => {
    const plugin = followPlugin(owner);
    const v = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: owner.current.draft,
        extensions: [
          EditorView.lineWrapping,
          lock.current.of(lockOf(owner.current.locked)),
          attrs.current.of(attributesOf(owner.current)),
          plugin,
        ],
      }),
    });
    view.current = { view: v, follow: v.plugin(plugin)! };
    return () => {
      view.current = null;
      v.destroy();
    };
  }, []);
  const { draft, locked, attributes } = props;
  const attrKey = JSON.stringify(attributes);
  useEffect(() => {
    view.current?.view.dispatch({
      effects: [
        lock.current.reconfigure(lockOf(owner.current.locked)),
        attrs.current.reconfigure(attributesOf(owner.current)),
      ],
    });
  }, [locked, attrKey]);
  useEffect(() => {
    view.current?.follow.follow();
  }, [draft]);
  return <div ref={host} className="editor-dsl" />;
}

/**
 * The test seam (ADR-0011 §2.1): replace the text of the view that `el` is in (or is) by `text`, as one transaction
 * from the rider, the path a keystroke takes once CodeMirror has read it from the DOM. jsdom cannot type into a
 * contenteditable; real typing is covered by the design harness and the real-app e2e.
 */
export function setText(el: Element, text: string) {
  const v = viewOf(el);
  v.dispatch({
    changes: { from: 0, to: v.state.doc.length, insert: text },
    userEvent: "input.paste",
  });
}
/** The text the view that `el` is in (or is) shows. */
export function textOf(el: Element): string {
  return viewOf(el).state.doc.toString();
}
/** The CodeMirror view that `el` is in (or is). */
export function viewOf(el: Element): EditorView {
  const v = EditorView.findFromDOM(el as HTMLElement);
  if (!v) throw new Error("not inside a text view");
  return v;
}
