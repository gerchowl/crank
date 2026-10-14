// The `?` keys dialog scaffold (#88). Modal: it traps Tab and consumes Escape, then returns focus to where it
// came from. The binding list is a scaffold; the five ride actions and their bindings land with DESIGN §7.2.
import { useEffect, useRef } from "react";

const FOCUSABLE =
  'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

export function KeysDialog({ onClose }: { onClose: () => void }) {
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const from =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    box.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    return () => from?.focus();
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation(); // consumed: nothing behind the dialog sees it
      onClose();
    } else if (e.key === "Tab") {
      const items = [
        ...(box.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? []),
      ];
      if (items.length === 0) return e.preventDefault();
      const first = items[0]!;
      const last = items[items.length - 1]!;
      const at = document.activeElement;
      if (e.shiftKey && (at === first || !box.current?.contains(at))) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (at === last || !box.current?.contains(at))) {
        e.preventDefault();
        first.focus();
      }
    }
  };

  return (
    <div className="keys-scrim">
      <div
        ref={box}
        className="keys-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="keys-title"
        onKeyDown={onKeyDown}
      >
        <h2 id="keys-title">Keyboard</h2>
        <dl>
          <dt>
            <kbd>?</kbd>
          </dt>
          <dd>Show this list</dd>
          <dt>
            <kbd>Esc</kbd>
          </dt>
          <dd>Close a dialog</dd>
          <dt>
            <kbd>Tab</kbd>
          </dt>
          <dd>Move between controls</dd>
        </dl>
        <p className="muted">
          Ride controls and their bindings appear here once they exist.
        </p>
        <button type="button" className="primary" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  );
}
