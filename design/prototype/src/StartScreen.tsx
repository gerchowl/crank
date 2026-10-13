// Cold-start screen: F · Spin-up (docs/design/BRAND.md §F · Spin-up, issues #83 / #138). Shown only while the
// core boots; the wordmark is the brand spike's outlined SVG (design/brand/src, no font). The launch motion is
// F's pure `frame(t)` painted per animation frame from the clock; the markup at rest is the settled wordmark
// (A · Slice), so under prefers-reduced-motion nothing is driven and the final frame is what shows.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { SPIN, frame, paint } from "../../brand/src/spin.ts";
import { svg } from "../../brand/src/wordmark.ts";
import { retryBoot, type Boot } from "./boot";

/** The boot status line appears only when a boot outlasts the launch motion budget (BRAND.md §Motion). */
export const STATUS_AFTER_MS = 1200;
/** Cross-fade to the destination (BRAND.md §Start screen, hand-off). */
export const HANDOFF_MS = 200;

// Colours stay CSS custom properties (start.css): the wordmark paints with roles, not hex.
const COLOURS = { ink: "var(--start-ink)", accent: "var(--start-accent)" };
const WORDMARK = svg("spin", COLOURS, "start");

export function StartScreen({ boot, mode, onGone }: { boot: Boot; mode: "dark" | "light"; onGone: () => void }) {
  const el = useRef<HTMLElement>(null);
  const [slowAttempt, setSlowAttempt] = useState(0);
  const attempt = boot.phase === "booting" ? boot.attempt : 0;

  // The launch motion plays once from the first frame and rests on the wordmark, whatever the boot does: it is
  // never gated on readiness, held or restarted (BRAND.md §Boot behaviours (a), (b), failure). A hand-off lets
  // it run on at 1:1 under the fade. `data-t` reports the painted t (tests, screenshots).
  useLayoutEffect(() => {
    const start = el.current;
    const art = start?.querySelector("svg");
    if (!start || !art || matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const draw = (t: number) => {
      paint(art, frame(t), COLOURS);
      start.dataset.t = String(Math.round(t));
    };
    const t0 = performance.now();
    let raf = 0;
    const tick = () => {
      const t = Math.min(performance.now() - t0, SPIN.end);
      draw(t);
      if (t < SPIN.end) raf = requestAnimationFrame(tick);
    };
    draw(0); // before the first paint: never a flash of the settled wordmark
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  // Timed from the boot attempt, not from the motion; inserted (so announced) only on a slow boot.
  useEffect(() => {
    if (!attempt) return;
    const t = setTimeout(() => setSlowAttempt(attempt), STATUS_AFTER_MS);
    return () => clearTimeout(t);
  }, [attempt]);

  // Core ready: the destination is live and focused at once; this screen stops taking input and fades out,
  // its launch motion running on underneath (removed at once under reduced motion).
  useEffect(() => {
    if (boot.phase !== "ready" || !el.current) return;
    const start = el.current;
    start.inert = true;
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
