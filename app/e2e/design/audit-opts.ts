// The layout audit's options (layout-audit.js), shared by the Chromium harness (design.pw.ts) and the WKWebView run
// (webkit.ts, #630), so both engines are held to the same rules and the same declared exemptions.

/** The generic layout invariants (#630): they hold on every fixture at every size, in every engine. */
export const INVARIANTS = [
  "layout.contained",
  "layout.landmarks",
  "layout.textOverflow",
] as const;

/**
 * The declared exemptions from the invariants (layout-audit.js `opts.exempt`): each names the rules it lifts, the
 * elements (an element or anything inside it), and why. Nothing else is skipped.
 */
const EXEMPT = [
  {
    rules: ["layout.contained", "layout.textOverflow"],
    selector: ".sr-only",
    reason:
      "accessible text only: a 1 px box that clips its words by design (shell.css), never drawn",
  },
  {
    rules: ["layout.contained"],
    selector: "svg *",
    reason:
      "a graphic's marks sit at their data's coordinates in the SVG's own viewBox, which clips them; the SVG's box is checked",
  },
  {
    rules: ["layout.contained"],
    selector: ".skip-link",
    reason:
      "the skip link waits above the window until it is focused (WCAG 2.4.1); the focus walk checks it on screen",
  },
  {
    rules: ["layout.textOverflow"],
    selector: ".hero-t1 .colon",
    reason:
      "the countdown's colon is narrower than its glyph's advance on purpose (ride.css, §18.2), its ink inside the box",
  },
  {
    rules: ["layout.contained"],
    selector: ".review-trace svg",
    reason:
      "temporary, #633: in WKWebView the trace's `height: 100%` resolves against the whole trace, not its row",
  },
];

/** Direction A's allowances (§18.4): the rail's items are `--control-h-rail`; every other control is `--control-h`. */
export const AUDIT_OPTS = {
  controls: [{ selector: ".rail button", token: "--control-h-rail" }],
  exempt: EXEMPT,
};
