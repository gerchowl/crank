// A modal question with its answers (DESIGN §5.7 "Unsaved changes", #110): the WAI-ARIA dialog pattern, as the keys
// dialog (KeysDialog.tsx). It is rendered on `document.body` and everything else there is `inert` while it is open, so
// neither focus, a click nor assistive technology reaches the page behind it. Focus moves to the answer named `focus`
// (the safe one: Save, or Cancel before a delete), Tab and Shift+Tab wrap inside it, Escape is Cancel, and focus
// returns to where it came from when it closes.
import { useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

const FOCUSABLE =
  'button:not(:disabled), [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

export interface Answer {
  label: string;
  onClick: () => void;
  /** `primary` for the answer the dialog recommends. */
  kind?: "primary";
  disabled?: boolean;
  /** Says why the answer is disabled; shown under the answers. */
  reason?: string;
}

export function Dialog({
  title,
  children,
  answers,
  focus,
  onCancel,
}: {
  title: string;
  children?: ReactNode;
  /** The answers in their order, Cancel included. */
  answers: readonly Answer[];
  /** The label of the answer focused first. */
  focus: string;
  /** Escape. */
  onCancel: () => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const scrim = useRef<HTMLDivElement>(null);
  const first = useRef(focus);

  useEffect(() => {
    const from =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    // The page behind is inert; what was inert already stays so when the dialog closes.
    const behind = [...document.body.children].filter(
      (e) => e !== scrim.current && !e.hasAttribute("inert"),
    );
    for (const e of behind) e.setAttribute("inert", "");
    const buttons = [
      ...(box.current?.querySelectorAll<HTMLButtonElement>("button") ?? []),
    ];
    (
      buttons.find((b) => b.textContent === first.current && !b.disabled) ??
      box.current?.querySelector<HTMLElement>(FOCUSABLE)
    )?.focus();
    return () => {
      for (const e of behind) e.removeAttribute("inert");
      from?.focus();
    };
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    } else if (e.key === "Tab") {
      const items = [
        ...(box.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? []),
      ];
      if (items.length === 0) return e.preventDefault();
      const head = items[0]!;
      const tail = items[items.length - 1]!;
      const at = document.activeElement;
      if (e.shiftKey && (at === head || !box.current?.contains(at))) {
        e.preventDefault();
        tail.focus();
      } else if (!e.shiftKey && (at === tail || !box.current?.contains(at))) {
        e.preventDefault();
        head.focus();
      }
    }
  };
  const reasons = answers.filter((a) => a.disabled && a.reason);

  return createPortal(
    <div ref={scrim} className="dialog-scrim">
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions -- the WAI-ARIA dialog pattern traps Tab inside; the rule keys on element names, not roles, so it cannot allow a role=dialog div. */}
      <div
        ref={box}
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="dialog-title"
        aria-describedby={children ? "dialog-body" : undefined}
        onKeyDown={onKeyDown}
      >
        <h2 id="dialog-title">{title}</h2>
        {children && <div id="dialog-body">{children}</div>}
        <div className="dialog-answers">
          {answers.map((a) => (
            <button
              key={a.label}
              type="button"
              className={a.kind}
              disabled={a.disabled}
              onClick={a.onClick}
            >
              {a.label}
            </button>
          ))}
        </div>
        {reasons.map((a) => (
          <p key={a.label} className="dialog-reason">
            {a.reason}
          </p>
        ))}
      </div>
    </div>,
    document.body,
  );
}
