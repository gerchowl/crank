// Cold-start screen, concept A · Slice (docs/design/BRAND.md §Start screen, issue #62). Shown only while the
// core boots; the wordmark is the brand spike's outlined SVG (design/brand/src, no font); the launch motion is
// CSS (start.css) and is the final frame under prefers-reduced-motion.
import { useEffect, useRef, useState } from "react";
import { svg } from "../../brand/src/wordmark.ts";
import { retryBoot, type Boot } from "./boot";

/** The boot status line appears only when a boot outlasts the launch motion budget (BRAND.md §Motion). */
export const STATUS_AFTER_MS = 1200;
/** Cross-fade to the destination (BRAND.md §Start screen, hand-off). */
export const HANDOFF_MS = 200;

// Colours stay CSS custom properties (start.css): the wordmark paints with roles, not hex.
const WORDMARK = svg("slice", { ink: "var(--start-ink)", accent: "var(--start-accent)" });

export function StartScreen({ boot, mode, onGone }: { boot: Boot; mode: "dark" | "light"; onGone: () => void }) {
  const el = useRef<HTMLElement>(null);
  const [slowAttempt, setSlowAttempt] = useState(0);
  const attempt = boot.phase === "booting" ? boot.attempt : 0;

  // Timed from the boot attempt, not from the motion; inserted (so announced) only on a slow boot.
  useEffect(() => {
    if (!attempt) return;
    const t = setTimeout(() => setSlowAttempt(attempt), STATUS_AFTER_MS);
    return () => clearTimeout(t);
  }, [attempt]);

  // Core ready: the destination is live and focused at once; this screen stops taking input, freezes its
  // launch motion where it is and fades out (removed at once under reduced motion).
  useEffect(() => {
    if (boot.phase !== "ready" || !el.current) return;
    const start = el.current;
    start.inert = true;
    for (const a of start.getAnimations({ subtree: true })) a.pause();
    document.querySelector<HTMLElement>("[data-destination]")?.focus();
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return onGone();
    const fade = start.animate([{ opacity: 1 }, { opacity: 0 }], { duration: HANDOFF_MS, easing: "linear", fill: "forwards" });
    fade.finished.then(onGone, () => {});
    return () => fade.cancel();
  }, [boot.phase, onGone]);

  return (
    <section ref={el} className="start" data-mode={mode} aria-label="crank is starting">
      <div className="wm" role="img" aria-label="crank" dangerouslySetInnerHTML={{ __html: WORDMARK }} />
      {boot.phase === "booting" && slowAttempt === boot.attempt && (
        <p className="start-status" role="status">
          Starting crank…
        </p>
      )}
      {boot.phase === "failed" && (
        // DESIGN P6: what broke, what the app does, what to do. Replaces the status line at once.
        <div className="start-error" role="alert">
          <p>
            <b>crank can’t start.</b> The ride core didn’t come up: {boot.detail}.
          </p>
          <p>
            Your rides and settings were not touched, and nothing is recording.
          </p>
          <p>
            <button type="button" className="start-retry" onClick={retryBoot}>
              Try again
            </button>
            <span>If it fails again, quit and reopen crank.</span>
          </p>
        </div>
      )}
    </section>
  );
}
