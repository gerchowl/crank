// Spike page. ?concept=<id>&theme=dark|light → one start screen; no concept → overview of all concepts.
// ?ready=<ms> simulates the core reporting ready after <ms> and runs the hand-off (BRAND.md §Start screen);
// without it the core never becomes ready, which is what the static screenshots show.
import "./style.css";
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
  // Timed from boot, not from the motion: inserted (and so announced) only on a slow boot.
  const status = setTimeout(() => {
    start.insertAdjacentHTML("beforeend", `<p class="status" role="status">Starting crank…</p>`);
  }, STATUS_AFTER_MS);
  const ready = q.get("ready");
  if (ready !== null) setTimeout(() => handOff(start, status), Number(ready));
} else {
  root.innerHTML =
    `<main class="overview">` +
    Object.entries(CONCEPTS)
      .map(([cid, c]) => `<figure><div class="wm">${svg(cid as ConceptId, paint, `ov-${cid}`)}</div><figcaption>${c.name}</figcaption></figure>`)
      .join("") +
    `</main>`;
}

/** Core ready: the destination is live and focused at once; the start screen stops taking input, freezes
 *  its motion where it is and fades out over 200 ms (removed at once under reduced motion). */
function handOff(start: HTMLElement, status: ReturnType<typeof setTimeout>) {
  clearTimeout(status);
  root.insertAdjacentHTML(
    "afterbegin",
    `<section class="dest" aria-label="Today"><p>Destination — stands in for Today / Onboarding / Resume</p>` +
      `<button type="button">Start ride</button></section>`,
  );
  root.querySelector<HTMLButtonElement>(".dest button")!.focus();
  start.inert = true;
  for (const a of start.getAnimations({ subtree: true })) a.pause();
  if (reduced) return start.remove();
  start.animate([{ opacity: 1 }, { opacity: 0 }], { duration: HANDOFF_MS, easing: "linear" }).finished.then(() => start.remove());
}
