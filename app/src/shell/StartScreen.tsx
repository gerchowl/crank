// Cold-start screen and hand-off (BRAND.md §Start screen, §F · Spin-up; #88, #552). Its launch motion is F · Spin-up:
// the brand spike's pure `frame(t)` (design/brand/src/spin.ts, the same code the prototype paints), driven per
// animation frame from the clock, from the first frame, whatever the boot does. It is never restarted ("Try again"
// keeps the one motion going) and runs on at 1:1 under the hand-off fade. The markup at rest is the settled wordmark
// (A · Slice), so under prefers-reduced-motion nothing is driven and the final frame is what shows.
//
// When the hand-off begins is the shell's (App.tsx): at once when the core is ready, except on the first launch of a
// build, which waits for the motion to end (`onMotionEnd`). The moment it begins (`leaving`) the destination
// (rendered underneath by the shell) is focused, this screen turns inert and fades over HANDOFF_MS, or is removed at
// once under reduced motion. The slow-boot status line is inserted by a timer from the boot attempt, so a fast boot
// never shows it and a screen reader never hears it. The build's version string sits at the bottom right.
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { SPIN, frame, paint } from "../../../design/brand/src/spin.ts";
import {
  CONCEPTS,
  focusShift,
  svg,
} from "../../../design/brand/src/wordmark.ts";
import type { ThemeMode } from "../theme";

/** The boot status line appears only when a boot outlasts this (BRAND.md §Motion budget). */
export const STATUS_AFTER_MS = 1200;
/** Cross-fade to the destination. */
export const HANDOFF_MS = 200;
/** F · Spin-up's length: at rest on the wordmark from here (BRAND.md §F · Spin-up, Timing). */
export const MOTION_MS = SPIN.end;

export type Boot =
  | { phase: "booting"; attempt: number }
  | { phase: "ready" }
  | { phase: "failed"; detail: string };

export const reducedMotion = () =>
  typeof matchMedia === "function" &&
  matchMedia("(prefers-reduced-motion: reduce)").matches;

// The wordmark paints with the start screen's roles (theme.css `.start[data-mode]`), never hex. The SVG is the brand
// spike's generated drawing, so it is markup, not a JSX `<svg>`: a brand graphic (`data-graphic`), not an icon
// (DESIGN §18.7 rule 4); its wrapper names it. The wrapper carries the shift that centres the letters' optical centre,
// so the drawing stays inside the box that lays it out.
const COLOURS = { ink: "var(--start-ink)", accent: "var(--start-accent)" };
const WORDMARK = svg("spin", COLOURS, "start", ' data-graphic=""');
const CENTRE = {
  "--focus-shift": `${focusShift(CONCEPTS.spin.build(COLOURS, "start"))}%`,
} as CSSProperties;

export function StartScreen({
  boot,
  mode,
  build,
  leaving,
  onRetry,
  onGone,
  onMotionEnd,
}: {
  boot: Boot;
  mode: ThemeMode;
  /** This build's version string, once read. */
  build: string | null;
  /** The hand-off has begun: the destination is live underneath. */
  leaving: boolean;
  onRetry: () => void;
  onGone: () => void;
  /** F · Spin-up has reached its rest (never called under reduced motion: there is no motion). */
  onMotionEnd: () => void;
}) {
  const art = useRef<HTMLDivElement>(null);
  const motionEnd = useRef(onMotionEnd);
  useLayoutEffect(() => {
    motionEnd.current = onMotionEnd;
  });
  const [slowAttempt, setSlowAttempt] = useState(0);
  const attempt = boot.phase === "booting" ? boot.attempt : 0;

  // The motion, once per mount: t is the time since the first frame, capped at the rest. `data-t` reports the
  // painted t (tests). Painted before the first paint, so the settled wordmark never flashes first.
  useLayoutEffect(() => {
    const wrap = art.current;
    const drawing = wrap?.querySelector("svg");
    if (!wrap || !drawing || reducedMotion()) return;
    const draw = (t: number) => {
      paint(drawing, frame(t), COLOURS);
      wrap.dataset.t = String(Math.round(t));
    };
    const t0 = performance.now();
    let raf = 0;
    let ended = false;
    const end = () => {
      if (ended) return;
      ended = true;
      cancelAnimationFrame(raf);
      draw(SPIN.end);
      motionEnd.current();
    };
    const tick = () => {
      const t = Math.min(performance.now() - t0, SPIN.end);
      if (t >= SPIN.end) return end();
      draw(t);
      raf = requestAnimationFrame(tick);
    };
    draw(0);
    raf = requestAnimationFrame(tick);
    // A webview that launches hidden may pause animation frames (WKWebView does): the clock, not the frames, bounds
    // the hold, so the motion is at rest and the hand-off free at SPIN.end whether or not a frame was painted.
    const bound = setTimeout(end, SPIN.end);
    return () => {
      ended = true;
      cancelAnimationFrame(raf);
      clearTimeout(bound);
    };
  }, []);

  useEffect(() => {
    if (!attempt) return;
    const t = setTimeout(() => setSlowAttempt(attempt), STATUS_AFTER_MS);
    return () => clearTimeout(t);
  }, [attempt]);

  useEffect(() => {
    if (!leaving) return;
    document.querySelector<HTMLElement>("[data-destination]")?.focus();
    if (reducedMotion()) return onGone();
    const t = setTimeout(onGone, HANDOFF_MS);
    return () => clearTimeout(t);
  }, [leaving, onGone]);

  return (
    <section
      className="start"
      data-mode={mode}
      aria-label="crank is starting"
      inert={leaving}
      data-leaving={leaving ? "" : undefined}
    >
      <div
        ref={art}
        className="start-wordmark"
        role="img"
        aria-label="crank"
        style={CENTRE}
        dangerouslySetInnerHTML={{ __html: WORDMARK }}
      />
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
      {build !== null && <p className="start-build">{build}</p>}
    </section>
  );
}
