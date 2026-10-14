// Cold-start screen and hand-off (BRAND.md §Start screen). Shown only while the core boots; no minimum display
// time, never waits for devices. The moment the core is ready the destination (rendered underneath by the
// shell) is focused, this screen turns inert and fades over HANDOFF_MS, or is removed at once under
// prefers-reduced-motion. The slow-boot status line is inserted by a timer from the boot attempt, so a fast
// boot never shows it and a screen reader never hears it. The wordmark is the vendored outlined SVG (no font);
// the launch motion is the brand spike's and is not part of this shell.
import { useEffect, useState } from "react";
import wordmarkDark from "../../../assets/brand/wordmark-dark.svg";
import wordmarkLight from "../../../assets/brand/wordmark-light.svg";
import type { ThemeMode } from "../theme";

/** The boot status line appears only when a boot outlasts this (BRAND.md §Motion budget). */
export const STATUS_AFTER_MS = 1200;
/** Cross-fade to the destination. */
export const HANDOFF_MS = 200;

export type Boot =
  | { phase: "booting"; attempt: number }
  | { phase: "ready" }
  | { phase: "failed"; detail: string };

const reducedMotion = () =>
  typeof matchMedia === "function" &&
  matchMedia("(prefers-reduced-motion: reduce)").matches;

export function StartScreen({
  boot,
  mode,
  onRetry,
  onGone,
}: {
  boot: Boot;
  mode: ThemeMode;
  onRetry: () => void;
  onGone: () => void;
}) {
  const [slowAttempt, setSlowAttempt] = useState(0);
  const attempt = boot.phase === "booting" ? boot.attempt : 0;
  const ready = boot.phase === "ready";

  useEffect(() => {
    if (!attempt) return;
    const t = setTimeout(() => setSlowAttempt(attempt), STATUS_AFTER_MS);
    return () => clearTimeout(t);
  }, [attempt]);

  useEffect(() => {
    if (!ready) return;
    document.querySelector<HTMLElement>("[data-destination]")?.focus();
    if (reducedMotion()) return onGone();
    const t = setTimeout(onGone, HANDOFF_MS);
    return () => clearTimeout(t);
  }, [ready, onGone]);

  return (
    <section
      className="start"
      data-mode={mode}
      aria-label="crank is starting"
      inert={ready}
      data-leaving={ready ? "" : undefined}
    >
      <img src={mode === "dark" ? wordmarkDark : wordmarkLight} alt="crank" />
      {boot.phase === "booting" && slowAttempt === boot.attempt && (
        <p className="start-status" role="status">
          Starting crank…
        </p>
      )}
      {boot.phase === "failed" && (
        // DESIGN P6: what broke, what the app did, what to do. Replaces the status line at once.
        <div className="start-error" role="alert">
          <p>
            <b>crank can’t start.</b> The ride core didn’t come up:{" "}
            {boot.detail}.
          </p>
          <p>
            Your rides and settings were not touched, and nothing is recording.
          </p>
          <p>
            <button type="button" className="start-retry" onClick={onRetry}>
              Try again
            </button>{" "}
            <span>If it fails again, quit and reopen crank.</span>
          </p>
        </div>
      )}
    </section>
  );
}
