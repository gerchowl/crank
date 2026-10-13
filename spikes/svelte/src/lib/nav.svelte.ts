// Hash routing: #/ride (default), #/checkin, #/editor, #/review.
export type View = "ride" | "checkin" | "editor" | "review";

const VIEWS: readonly View[] = ["ride", "checkin", "editor", "review"];

function fromHash(): View {
  const h = location.hash.replace(/^#\/?/, "").split("?")[0] ?? "";
  return (VIEWS as readonly string[]).includes(h) ? (h as View) : "ride";
}

class Nav {
  view = $state<View>(fromHash());
  /** Overlay showing the keyboard shortcuts (?). */
  help = $state(false);

  constructor() {
    window.addEventListener("hashchange", () => {
      this.view = fromHash();
    });
  }

  go(v: View): void {
    location.hash = `/${v}`;
    this.view = v;
  }
}

export const nav = new Nav();

/** True when a key event targets a text field (shortcuts must not fire while typing). */
export function isTyping(e: KeyboardEvent): boolean {
  const t = e.target;
  if (!(t instanceof HTMLElement)) return false;
  return t.isContentEditable || t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || t instanceof HTMLSelectElement;
}
