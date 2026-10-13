// Spike page. ?concept=<id>&theme=dark|light → one start screen; no concept → overview of the static concepts.
// ?ready=<ms> simulates the core reporting ready after <ms> and runs the hand-off (BRAND.md §Start screen);
// without it the core never becomes ready, which is what the static screenshots show.
// F · Spin-up only (BRAND.md §F · Spin-up): ?t=<ms> renders the frozen frame at t (storyboard, deterministic);
// ?hold holds the closed ring until ready (boot behaviour b′); ?w=<px> sets the wordmark width (small sizes).
import "./style.css";
import { SPIN, frame, paint as paintSpin } from "./spin.ts";
import { CONCEPTS, svg, type ConceptId, type Paint } from "./wordmark.ts";

/** Boot status appears only when boot outlasts the launch motion budget — also under reduced motion. */
export const STATUS_AFTER_MS = 1200;
export const HANDOFF_MS = 200;

const q = new URLSearchParams(location.search);
const theme = q.get("theme") === "light" ? "light" : "dark";
document.documentElement.dataset.theme = theme;
// Colours stay CSS custom properties (spec/THEMES.md): the page paints with roles, not hex.
const paint: Paint = { ink: "var(--ink)", accent: "var(--accent)" };
const root = document.getElementById("root")!;
const id = q.get("concept");
const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

if (id && id in CONCEPTS) {
  const concept = id as ConceptId;
  root.innerHTML =
    `<main class="start" data-concept="${concept}" aria-label="crank is starting">` +
    `<div class="wm" role="img" aria-label="crank">${svg(concept, paint)}</div></main>`;
  const start = root.querySelector<HTMLElement>(".start")!;
  const w = q.get("w");
  if (w !== null) start.style.cssText = `--w: ${Number(w)}px; --max: ${Number(w)}px`;
  // Under reduced motion F is never driven: its markup at rest is the settled wordmark.
  const spin = concept === "spin" && !reduced ? spinUp(start) : null;
  // Timed from boot, not from the motion: inserted (and so announced) only on a slow boot.
  const status = setTimeout(() => {
    start.insertAdjacentHTML("beforeend", `<p class="status" role="status">Starting crank…</p>`);
  }, STATUS_AFTER_MS);
  const ready = q.get("ready");
  if (ready !== null) setTimeout(() => handOff(start, status, spin), Number(ready));
} else {
  root.innerHTML =
    `<main class="overview">` +
    Object.entries(CONCEPTS)
      .filter(([cid]) => cid !== "spin") // F is motion; at rest it is A
      .map(([cid, c]) => `<figure><div class="wm">${svg(cid as ConceptId, paint, `ov-${cid}`)}</div><figcaption>${c.name}</figcaption></figure>`)
      .join("") +
    `</main>`;
}

/** F's clock. Timeline time t follows wall time from the first frame (or, with ?hold, stops at the closed ring
 *  until ready); on ready it keeps running at 1:1 under the hand-off fade — no cut, no fast-forward, so a boot
 *  at 149 ms and one at 151 ms look the same and nothing rushes over the live destination. Everything runs on requestAnimationFrame + performance.now,
 *  so a fake clock drives it deterministically; `data-t` on the start screen reports the painted t. */
function spinUp(start: HTMLElement) {
  const art = start.querySelector("svg")!;
  const draw = (t: number) => {
    paintSpin(art, frame(t), paint);
    start.dataset.t = String(Math.round(t));
  };
  const frozen = q.get("t");
  if (frozen !== null) {
    draw(Number(frozen));
    return null;
  }
  const t0 = performance.now();
  const hold = q.has("hold");
  let at = (w: number) => Math.min(hold ? Math.min(w, SPIN.up) : w, SPIN.end);
  let fadeFrom: number | null = null;
  let running = false;
  const tick = () => {
    const w = performance.now() - t0;
    const t = at(w);
    draw(t);
    if (fadeFrom !== null) {
      const p = (w - fadeFrom) / HANDOFF_MS;
      if (p >= 1) return start.remove();
      start.style.opacity = String(1 - p);
    }
    // Stop once nothing changes: settled, or holding the closed ring (a steady blur paints the same frame).
    running = fadeFrom !== null || t < (hold ? SPIN.up : SPIN.end);
    if (running) requestAnimationFrame(tick);
  };
  draw(0); // before the first paint: never a flash of the settled wordmark
  running = true;
  requestAnimationFrame(tick);
  return {
    handOff() {
      const w = performance.now() - t0;
      const from = at(w);
      at = (x) => Math.min(from + (x - w), SPIN.end);
      fadeFrom = w;
      if (!running) {
        running = true;
        requestAnimationFrame(tick);
      }
    },
  };
}

/** Core ready: the destination is live and focused at once; the start screen stops taking input and fades out
 *  over 200 ms (removed at once under reduced motion). A freezes its motion where it is; F lets it run on at
 *  its own pace under the fade (a frozen blur reads as a glitch; a snap or a rush is conspicuous). */
function handOff(start: HTMLElement, status: ReturnType<typeof setTimeout>, spin: ReturnType<typeof spinUp>) {
  clearTimeout(status);
  root.insertAdjacentHTML(
    "afterbegin",
    `<section class="dest" aria-label="Today"><p>Destination — stands in for Today / Onboarding / Resume</p>` +
      `<button type="button">Start ride</button></section>`,
  );
  root.querySelector<HTMLButtonElement>(".dest button")!.focus();
  start.inert = true;
  if (reduced) return start.remove();
  if (spin) return spin.handOff();
  for (const a of start.getAnimations({ subtree: true })) a.pause();
  start.animate([{ opacity: 1 }, { opacity: 0 }], { duration: HANDOFF_MS, easing: "linear" }).finished.then(() => start.remove());
}
