# crank — wordmark + start screen

Issue: [#55](https://github.com/gerchowl/crank/issues/55). Date: 2026-10-13. Status: **round 2**. Both UX
reviews of round 1 (astra, opus) ranked **A · Slice** first, so it is the chosen direction; round 2 fixes its
construction and size rules (see "Review resolution"). Follow-up: [#62](https://github.com/gerchowl/crank/issues/62)
builds the start screen into `design/prototype` once #49 has merged.

Owner's brief: *"for the start screen — get some fancy design spike, e.g. CRANK slightly italic with lines
through it to make it look fast — or sth."* Constraints from the product: precise, calm under effort, a coach
you trust; not Zwift-cartoon, not gamer-neon (DESIGN.md, PR #49). It must sit with the app mark
(`assets/icon/logo.svg`, [ICON.md](ICON.md)): graphite + brass, crank axis at **50°**.

## Files

| Path | What |
| --- | --- |
| `design/brand/` | the spike: `pnpm -C design/brand dev` (page), `screens`, `export`, `outline`, `typecheck` |
| `design/brand/src/geometry.ts` | construction constants (`AXIS`, `SLANT`, `TRACK`, `KERN`, `SLICE`) + layout |
| `design/brand/src/wordmark.ts` | the five concepts as framework-free SVG builders (see "Why plain TS") |
| `design/brand/scripts/slice.ts` | builds A as true outlines with boolean ops (paper.js, MIT) → `src/slice.ts` (GENERATED) |
| `design/brand/src/glyphs.ts` | GENERATED outlines of C R A N K from the font (`scripts/outline.ts`) |
| `design/brand/fonts/` | Barlow Bold + its licence (`OFL.txt`) — source for the outlines only |
| `assets/brand/wordmark{,-dark,-light}.svg` | **full form**: mono `currentColor` master + two-colour variants for dark / light surfaces |
| `assets/brand/wordmark-small{,-dark,-light}.svg` | **small form** (uncut letters, no lines), same three variants |
| `docs/design/brand/slice.<w>x<h>.<theme>.png` | A's start screen, 1440×900 + 1024×768, dark + light |
| `docs/design/brand/slice.slow-boot.*.png` / `slice.handoff-100ms.*.png` | A on a boot over 1.2 s; A half-way through the hand-off |
| `docs/design/brand/sizes.{dark,light}.png` | size proof: every asset at its stated size, measured labels, title-bar example |
| `docs/design/brand/motion.<concept>.<t>ms.png` | launch motion frames at 0 / 150 / 300 / 500 / 750 / 1200 ms |
| `docs/design/brand/<b–e concept>.*.png`, `overview.*.png` | the rejected concepts, kept as the record |

`pnpm -C design/brand screens` regenerates the outlines, re-exports the assets and renders every PNG (Vite +
Playwright, timers on Playwright's fake clock). It fails on: page errors; overflow; a wordmark leaving the
window; launch motion over 1.2 s; the boot status showing before 1.2 s or missing after it (with and without
reduced motion); a destination that is not on top and focused as the hand-off begins; the start screen not
removed at once under reduced motion; any size-sheet item not rendered at its stated size. `pnpm export`
refuses to write an asset containing `clipPath`, `mask` or an `id`.

### Why plain TS + Vite, not React

The geometry is the deliverable, and it has three consumers: the spike page, the SVG exporter (Node) and later
the React prototype (#62) / production shell (#28). A framework-free module of string builders serves all
three without a renderer; motion is CSS keyframes on class hooks. React would add a dependency and a second
representation of the same SVG for no gain. paper.js runs only at build time (`pnpm outline`).

## Type

**Barlow Bold** (v1.408) by Jeremy Tribby, **SIL Open Font License 1.1** (`design/brand/fonts/OFL.txt`).
A low-contrast grotesk with straight-sided rounds, drawn after California highway signage: engineered and calm
at size, which matches the "instrument" tone of the mark, and its squared C, R and K hold up to slicing.
Outlining glyphs into a logo is use of the font, not a modified Font Software, so the OFL's Reserved Font
Name clause does not apply; the font file is vendored only so `pnpm outline` is reproducible. **Nothing ships
the font**: the app and the assets use outlined paths. The oblique is a geometric **shear of the upright**
(not Barlow Italic) so the angle is a design parameter.

## Chosen: A · Slice

Oblique CRANK, two horizontal cuts through every letter that run on to the left as brass speed lines whose
starts sit on the mark's 50° crank axis. Why (both reviews concur):

- **It answers the owner literally, with discipline.** 12° oblique, two cuts — not five, no gradient, no glow.
- **One system with the icon.** Graphite + brass on both surfaces; the line starts are on a true 50° line.
- **Purely typographic.** Works without the mark (the dock icon shows it during launch); a small form covers
  sizes where the cuts would close up.
- **Calm motion.** Letters settle and the lines draw out behind them in 930 ms; no rotation, no bounce.

### Rejected concepts (kept in `wordmark.ts` and the screenshots as the record)

| Concept | Rank (astra / opus) | Why not |
| --- | --- | --- |
| B · Stepped blur | 4 / 4 | the brass comb is busy and reads retro-sports; furthest from "calm under effort"; worst at 1024×768 |
| C · Mark-as-C | 5 / 5 | reads "C RANK": the upright two-tone round C breaks a word of oblique ink letters |
| D · Kinetic lockup | 2 / 3 | strongest cycling identity, but the static lockup is conventional and the motion is more than a start screen needs; the arm crosses the ring mid-turn. **Use its lockup for About / marketing** if one is needed |
| E · Instrument | 3 / 2 | calm and precise, but barely answers the request for speed; the ruler reads decorative. **Its uncut oblique letters became A's small form** |

## Construction

Units are font units of the outlined glyphs: cap height **H = 700**, baseline y = 0, y down. Constants:
`design/brand/src/geometry.ts`. The outlines are flattened at build time (`scripts/slice.ts`): plain filled
paths with the shear baked in, no `clipPath`, no `mask`, no ids, so a file can be inlined any number of times
and survives Figma / print / VectorDrawable pipelines.

```
            line starts on x = left − lead − y / tan 50°   (a true 50° line, Δx/Δy = cot 50° = 0.839)
                         /
             ▰▰▰▰▰▰▰▰▰▰ /  ) C R A N K    ← cut 2, centre y = −570 (0.81 H), g = 40 (0.057 H)
                       /     ╱ letters sheared 12°
     ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰ /    ) C R A N K    ← cut 1, centre y = −305 (0.44 H)
                            C R A N K    ← baseline
     bar right end = the C's own outline moved left by g  →  gap g at every height of the bar
```

- **Letters:** Barlow Bold outlines, shear `x' = x − y·tan 12°`; font kerning + 28 units tracking, with one
  hand-kerned pair: **R→A −12** (the font's +15 read looser than N→K).
- **Cuts:** two horizontal gaps, thickness **g = 40** (0.057 H), centred at y = −305 and y = −570, subtracted
  from the letters (paper.js boolean ops). The heights leave no slivers: the lower band (−325…−285) swallows
  R's bowl junction (−324…−285), so the bowl ends cleanly on the cut; the upper band (−590…−550) swallows R's
  counter top (−579) and clears A's counter apex (−524) and the C's terminals (−466 / −229) by ≥ 26 units.
  The strip above cut 2 is ~0.16 H of ink — the C, A and K tops read as thin caps; that sets the minimum size.
- **Speed lines, left end:** a **50° chamfer**; its bottom corner lies on `x = left − lead − y / tan 50°` with
  `left` = the C's leftmost ink (47) and **lead = 940**. That line does **not** depend on the letter shear, so
  the two starts (x = −653.9 and −431.5, Δy = 265) step by exactly 265 / tan 50° = **222.4** units: a true 50°
  line, parallel to the crank arms. (Round 1 added the shear term and landed on 43.6°.)
- **Speed lines, right end:** the C's own outline moved left by g, subtracted from the bar. The gap between
  line and letter is **g at every height** of the bar — straight where the C's side is straight (lower cut),
  curved with the C's shoulder (upper cut). The line visibly enters the cut.
- **Optical centre:** the letters, not the whole drawing, are centred in the window; the lines trail into the
  left margin (`focus` in `src/slice.ts`, applied as `--focus-shift`).
- **Colour:** letters `ink`, lines `accent`.

## Colour (start screen roles)

The start screen uses the brand pair of the mark, not the app's theme preset (THEMES.md): it shows before
the rider's theme is loaded. The light pair is chosen for contrast, not inverted. Contrast (WCAG 2.x):

| Role | Dark | on `#1a1d22` | Light | on `#f4f1ea` |
| --- | --- | --- | --- | --- |
| `--bg` (surface) | `#1a1d22` graphite | — | `#f4f1ea` paper | — |
| `--ink` (letters) | `#ece6dc` | 13.6 : 1 | `#1a1d22` | 15.0 : 1 |
| `--accent` (lines, mark) | `#c08a4a` brass | 5.6 : 1 | `#94642d` dark brass | 4.5 : 1 |
| `--muted` (status text) | `#9ba0a8` | 6.4 : 1 | `#5b6068` | 5.6 : 1 |

Mode follows the OS (`prefers-color-scheme`) until the rider's own choice is readable; the native window
background must equal `--bg` so there is no flash before first paint (#62, #28).

## Sizes and usage

Proof: `sizes.dark.png`, `sizes.light.png` (every item at its stated size, shrinking disabled, labels measured
in the page; the run fails if any item renders at another size).

| Rendered size | Use | Why |
| --- | --- | --- |
| full form **≥ 160 px wide** (cap ≥ 27 px) | `wordmark*.svg` | the cut is ≥ 1.53 px; below that it closes up |
| below 160 px wide, **cap height ≥ 9 px** | `wordmark-small*.svg` (uncut, no lines) | the oblique letters alone stay legible down to ~44 × 11 px |
| cap height < 9 px | the app mark alone (`assets/icon/logo.svg`) | letters fill in |
| **title bar, 24 px high** | mark 16 px + small form 12 px high (cap ≈ 10 px) | the full form there is 118 × 24 px with 1.12 px cuts — shown as "don't" on the sheet |

- **Clear space:** 0.5 H on every side, measured from the letters and from the line starts.
- **On dark:** `-dark` (ivory + brass). **On light:** `-light` (graphite + dark brass). **Single ink**
  (print, embossing, a coloured surface): the unsuffixed file with `color:` set.
- **Don't:** re-set it in live text, change the slant or the cut heights, add lines, recolour the letters
  brass, put it on photos or gradients, add shadows or glows, animate it outside the launch.

## Start screen

**When it shows.** Only on a **cold start** (process launch), from the first window paint until the core
reports ready — backend handshake done and persisted state loaded (athlete model, today's proposal, or an
interrupted ride). Never on window re-open, wake from sleep, navigation, or after a ride.

**It never delays the rider.**
- No minimum display time. If the core is ready before the first frame, there is no start screen.
- It does **not wait for devices.** Trainer and HR reconnect in the background; their state is shown where it
  matters (the Ready check — DESIGN.md §4).
- **Hand-off, the moment the core is ready:** the destination (Onboarding on first run, the Resume sheet after
  an interrupted ride — DESIGN §6.6 — otherwise Today) is rendered **underneath, live and focused at once**;
  the start screen becomes `inert` (no input), freezes its launch motion where it is, and fades out over
  **200 ms**. Under `prefers-reduced-motion` it is **removed at once** (no fade).
- **Slow boot:** the status line ("Starting crank…", `--muted`, `role="status"`) is **inserted by a boot
  timer at 1.2 s** — timed from boot, not from the motion, and the same under reduced motion. A fast boot
  never shows it, and a screen reader never announces it. A boot *failure* becomes an error in that line per
  DESIGN P6 — designed in #62.

**Layout.** Letters centred in the window (lines trailing left), wordmark at `min(52vw, 820px)` wide, status
line 48 px above the bottom edge. Nothing else: no version, no tips, no progress bar.

The spike demonstrates all of this: `?concept=slice&ready=<ms>` simulates the core becoming ready
(`src/main.ts`); `pnpm screens` checks it on a fake clock and captures `slice.slow-boot.*` and
`slice.handoff-100ms.*`.

## Motion

One curve: **settle** = `cubic-bezier(0.16, 1, 0.3, 1)` (fast start, long decelerating tail, no overshoot).
Every keyframe states only its `from`; the resting state is the element's own style, so
**`prefers-reduced-motion: reduce` = `animation: none` = the final frame**. Budget **≤ 1.2 s**, enforced by
`pnpm screens`.

| Element (A · Slice) | From | Duration | Delay | Ends |
| --- | --- | --- | --- | --- |
| each letter (C→K) | −60 units x, opacity 0 | 520 ms settle | 45 ms × index | 700 ms |
| lower / upper speed line | hidden by `clip-path: inset(0 0 0 100%)`, revealed right → left (drawn out behind the word) | 700 ms settle | 150 / 230 ms | 930 ms |
| status line (slow boot only) | opacity 0, on insertion at 1.2 s | 300 ms linear | — | — |
| hand-off | start screen opacity 1 → 0, over the live destination | 200 ms linear | when the core is ready | — |

The lines are revealed with a clip, not `scaleX`, so the 50° chamfer never changes angle mid-draw. The other
concepts' motions (B–E) are recorded in `src/style.css` and their `motion.*` frames.

## Review resolution (round 2)

| # | Finding (astra / opus) | Resolution |
| --- | --- | --- |
| 1 | Line starts on 43.6°, not 50° (both) | starts on `x = left − lead − y/tan 50°`, no shear term; Δx/Δy = cot 50° exactly (printed by `pnpm outline`) |
| 2 | Uneven wedge gaps where the 50° bar ends meet the C (both) | bar right end = the C's outline moved left by g: gap g at every height |
| 3 | Masters use a live `clipPath` with a fixed id (opus) | boolean-flattened outlines; `export` refuses `clipPath`/`mask`/`id` |
| 4 | No small form; size proof unreliable (both) | `wordmark-small*.svg` + switching rule; `sizes.*.png` with shrinking disabled, measured labels, 160 px, title bar, both themes |
| 5 | Composition right-weighted (both) | letters centred; lines trail into the margin |
| 6 | Reduced motion shows the status at once; status announced on fast boots (both) | status inserted by a 1.2 s boot timer, identical under reduced motion; checked on a fake clock |
| 7 | Hand-off under reduced motion / destination interactivity (astra) | destination live + focused as the hand-off begins; start screen `inert`; removed at once under reduced motion; checked |
| 8 | `scaleX` steepens the chamfers mid-draw (opus) | lines revealed with `clip-path: inset()` |
| 9 | R→A looser than N→K (both) | hand-kerned R→A −12 |
| — | D's arm crosses the ring mid-turn (opus) | D rejected; noted in its row |

## Review guide (round 2)

Start with `slice.1440x900.dark.png` / `.light.png`, then `sizes.dark.png` / `sizes.light.png`, then
`slice.slow-boot.*`, `slice.handoff-100ms.*` and `motion.slice.*`. Check: the 50° line through the two line
starts; the equal gap between each line and the C; the small-form switch and the title-bar example; the
start-screen rules above.

Out of scope for this PR: building it into `design/prototype` (#62) and the production app (#28).
