// Spike page. ?concept=<id>&theme=dark|light → one start screen; no concept → overview of all concepts.
import "./style.css";
import { CONCEPTS, svg, type ConceptId, type Paint } from "./wordmark.ts";

const q = new URLSearchParams(location.search);
const theme = q.get("theme") === "light" ? "light" : "dark";
document.documentElement.dataset.theme = theme;
// Colours stay CSS custom properties (spec/THEMES.md): the page paints with roles, not hex.
const paint: Paint = { ink: "var(--ink)", accent: "var(--accent)" };
const root = document.getElementById("root")!;
const id = q.get("concept");

if (id && id in CONCEPTS) {
  const concept = id as ConceptId;
  root.innerHTML =
    `<main class="start" data-concept="${concept}" aria-label="crank is starting">` +
    `<div class="wm" role="img" aria-label="crank">${svg(concept, paint)}</div>` +
    // Boot status: rendered from the start, revealed only if boot outlasts the launch motion (BRAND.md §Start screen).
    `<p class="status" role="status">Starting crank…</p></main>`;
} else {
  root.innerHTML =
    `<main class="overview">` +
    Object.entries(CONCEPTS)
      .map(([cid, c]) => `<figure><div class="wm">${svg(cid as ConceptId, paint, `ov-${cid}`)}</div><figcaption>${c.name}</figcaption></figure>`)
      .join("") +
    `</main>`;
}
