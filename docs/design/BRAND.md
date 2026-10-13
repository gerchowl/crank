# crank — wordmark + start screen

Issue: [#55](https://github.com/gerchowl/crank/issues/55). Date: 2026-10-13. Status: **proposed**, round 1
(an astra + opus UX review picks the concept; see the review guide at the end).
Follow-up: [#62](https://github.com/gerchowl/crank/issues/62) builds the chosen start screen into
`design/prototype` once #49 has merged.

Owner's brief: *"for the start screen — get some fancy design spike, e.g. CRANK slightly italic with lines
through it to make it look fast — or sth."* Constraints from the product: precise, calm under effort, a coach
you trust; not Zwift-cartoon, not gamer-neon (DESIGN.md, PR #49). It must sit with the app mark
(`assets/icon/logo.svg`, [ICON.md](ICON.md)): graphite + brass, crank axis at **50°**.

## Files

| Path | What |
| --- | --- |
| `design/brand/` | the spike: `pnpm -C design/brand dev` (page), `screens`, `export`, `outline`, `typecheck` |
| `design/brand/src/wordmark.ts` | all wordmark geometry, every concept; framework-free (see "Why plain TS") |
| `design/brand/src/glyphs.ts` | GENERATED outlines of C R A N K (`pnpm -C design/brand outline`) |
| `design/brand/fonts/` | Barlow Bold + its licence (`OFL.txt`) — source for the outlines only |
| `assets/brand/wordmark.svg` | recommended wordmark, outlined, mono (`currentColor`) master |
| `assets/brand/wordmark-dark.svg` / `-light.svg` | two-colour variants for dark / light surfaces |
| `docs/design/brand/<concept>.<w>x<h>.<theme>.png` | each concept's start screen, 1440×900 + 1024×768, dark + light |
| `docs/design/brand/overview.*.png` | all concepts side by side (contact sheet) |
| `docs/design/brand/motion.<concept>.<t>ms.png` | launch motion frames at 0 / 150 / 300 / 500 / 750 / 1200 ms |
| `docs/design/brand/assets.png` | the exported assets at 480 / 240 / 140 / 96 / 64 px wide (min-size check) |

`pnpm -C design/brand screens` re-exports the assets and regenerates every PNG (Vite + Playwright). It fails on
page errors, horizontal overflow, a wordmark that leaves the window, or any launch motion longer than 1.2 s.

### Why plain TS + Vite, not React

The brief allowed either. The geometry is the deliverable, and it has three consumers: the spike page, the
SVG exporter (Node) and later the React prototype (#62) / production shell (#28). A framework-free module of
string builders (`svg(concept, paint)`) serves all three without a renderer; motion is CSS keyframes on class
hooks. React would have added a dependency and a second representation of the same SVG for no gain.

## Type

**Barlow Bold** (v1.408) by Jeremy Tribby, **SIL Open Font License 1.1** (`design/brand/fonts/OFL.txt`).
A low-contrast grotesk with straight-sided rounds, drawn after California highway signage: it reads as
engineered and calm at size, which matches the "instrument" tone of the mark, and its squared C, R and K hold
up to slicing. Outlining glyphs into a logo is use of the font, not a modified Font Software, so the OFL's
Reserved Font Name clause does not apply; the font file is vendored only so `pnpm outline` is reproducible.
**Nothing ships the font**: the app and the assets use outlined paths.

The oblique is a geometric **shear of the upright** (not Barlow Italic) so the angle is a design parameter.

## Concepts

All five share the outlined Barlow Bold set, `SLANT = 12°`, `TRACK = 28` units, and the brand roles below.
Screens: `docs/design/brand/<id>.1440x900.dark.png` etc.; motion frames `motion.<id>.*`.

| | Concept (`id`) | Idea | Reads as | Risk |
| --- | --- | --- | --- | --- |
| **A** | **Slice** (`slice`) — *recommended* | The owner's direction, constructed: oblique CRANK, two horizontal cuts through every letter that run on to the left as brass speed lines. Cut ends and line starts sit on the 50° crank axis. | fast, but cut with a ruler, not a brush | cuts fill in below ~160 px wide → min size rule |
| B | Stepped blur (`blur`) | Motion blur as stepped strokes: three brass echoes trail the C, each a set of horizontal strokes that thin with distance (60 / 35 / 15 % of the pitch). | 70s/80s speed type | busiest of the five; "comb" read at small sizes; the most "sporty-retro", least "calm" |
| C | Mark-as-C (`lockup`) | The crankset mark *is* the C; RANK follows. One asset carries icon and name. | clever, unified | reads "C RANK" at a glance; the mark's two colours make the C unlike the other letters |
| D | Kinetic lockup (`kinetic`) | Mark + CRANK side by side; on launch the crank turns one revolution and locks on 50° while the letters settle. | the mark coming alive | static frame is just a conventional lockup; the idea lives in the motion |
| E | Instrument (`instrument`) | Calm oblique CRANK over a measured scale with one brass index, cut on the crank axis. | precision, "the coach you trust" | the least "fast"; doesn't answer the owner's request directly |

### Recommendation: **A · Slice**

- **It answers the owner's request literally and does it with discipline.** "Slightly italic with lines
  through it": 12° oblique, two cuts — not five, not a gradient, not a glow. The lines are derived from the
  mark's 50° axis (construction below), so the wordmark and the icon are one system.
- **Purely typographic.** It works without the mark beside it — the dock icon already shows the mark during
  launch — and it scales from the start screen down to 160 px (About, onboarding header, exported reports).
- **Calm motion.** The launch is the letters settling and the lines drawing out behind them: under a second,
  no rotation, no bounce.
- **Two colours, both roles.** Ink letters + brass lines on both surfaces; mono `currentColor` for single-ink
  use.

If the reviewers prefer the mark on the start screen, **D's crank turn** is the motion to borrow (mark
centred above A's wordmark); C is not recommended as the primary wordmark because of the "C RANK" read.

## Construction (A · Slice)

Units are font units of the outlined glyphs: cap height **H = 700**, baseline y = 0, y down.
All constants live in `design/brand/src/wordmark.ts` (`SLANT`, `TRACK`, `AXIS`, `SLICE`).

```
                       ┌ 50° axis (∥ crank arms)
                      /
          ▬▬▬▬▬▬▬▬▬▬ /  ═ C R A N K   ← cut 2 at y = −570 (0.81 H), g = 40 (0.057 H)
                    /     ╱ 12° shear
  ▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬ /    ═ C R A N K   ← cut 1 at y = −305 (0.44 H)
                         C R A N K   ← baseline
  line start of cut k: x = edge(y_k) − lead − y_k / tan 50°
```

- **Letters:** Barlow Bold outlines, shear `x' = x − y·tan 12°`, kerning from the font + 28 units tracking.
- **Cuts:** two horizontal gaps through the letters, thickness **g = 40** (0.057 H), centred at y = −305 and
  y = −570. The heights are chosen so no cut leaves a sliver: the lower band (−325…−285) swallows R's bowl
  junction (−324…−285), so the bowl ends cleanly on the cut; the upper band (−590…−550) swallows R's counter
  top (−579), and clears A's counter apex (−524) and the C's terminals (−466 / −229) by ≥ 26 units. (Round 1
  first tried −330 / −545, which left hairline slivers in R and a double-cut look above R's counter.)
  Implemented as a `clipPath` of the kept strips, so the asset stays transparent.
- **Speed lines:** each cut continues to the left as a brass bar of the same thickness g, ending g before the
  C's leading edge (so the line visibly *enters* the cut). Both ends of every bar are cut at **50°**.
- **Tie to the 50° axis:** the line starts sit on one line parallel to the crank axis. With cut spacing
  Δy = 265 (0.38 H), each higher line starts Δy / tan 50° = **222 units** further right; with the base lead of
  980 units the lines are 684 (lower) and 462 (upper) long.
- **Colour:** letters `ink`, lines `accent` (roles below).

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

## Usage rules (A · Slice)

- **Minimum size:** 160 px wide (the cut is then ≥ 1.5 px). Below that, use the app mark, not the wordmark
  (see `assets.png`: at 96 / 64 px the cuts close up).
- **Clear space:** 0.5 H on every side, measured from the letters and from the line starts.
- **On dark:** `wordmark-dark.svg` (ivory + brass). **On light:** `wordmark-light.svg` (graphite + dark brass).
  **Single ink** (print, embossing, a coloured surface): `wordmark.svg` with `color:` set; lines and letters
  take the same ink.
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
- **Hand-off:** the moment the core is ready, a **200 ms cross-fade** to the target view, cutting whatever is
  left of the launch motion. Target: Onboarding (first run), the Resume sheet (interrupted ride, DESIGN §6.6),
  otherwise Today.
- **Slow boot:** the status line ("Starting crank…", `--muted`, `role="status"`) is revealed only after
  1.2 s, so a fast boot never shows it. A boot *failure* becomes an error in that line per DESIGN P6 —
  designed in #62.

**Layout.** Wordmark centred in the window at `min(52vw, 820px)` wide (A), status line 48 px above the
bottom edge. Nothing else: no version, no tips, no progress bar.

## Motion

One curve for everything: **settle** = `cubic-bezier(0.16, 1, 0.3, 1)` (fast start, long decelerating tail,
no overshoot). D's crank turn uses **turn** = `cubic-bezier(0.35, 0, 0.15, 1)`. Every keyframe states only
its `from`; the resting state is the element's own style, so **`prefers-reduced-motion: reduce` =
`animation: none` = the final frame** (static screenshots are taken that way). Budget **≤ 1.2 s**, enforced by
`pnpm screens`.

| Concept | Element | From | Duration | Delay | Ends |
| --- | --- | --- | --- | --- | --- |
| **A Slice** | each letter (C→K) | −60 units x, opacity 0 | 520 ms settle | 45 ms × index | 700 ms |
| | lower / upper speed line | scaleX 0 from its right end (drawn out behind the word) | 700 ms settle | 150 / 230 ms | 930 ms |
| B Blur | word | −280 units x, opacity 0 | 800 ms settle | 0 | 800 ms |
| | echo k | collapsed onto the word, opacity 0 | 800 ms settle | 100 ms | 900 ms |
| C Mark-as-C | crank (arms + chainring) | rotate −360° about the hub → locks at 50° | 1000 ms turn | 0 | 1000 ms |
| | R A N K | −60 units x, opacity 0 | 600 ms settle | 200 + 50 ms × index | 950 ms |
| D Kinetic | ring | opacity 0 | 300 ms linear | 0 | 300 ms |
| | crank | rotate −360° (clockwise, as pedalled from the drive side) → 50° | 1100 ms turn | 0 | 1100 ms |
| | letters | −160 units x, opacity 0 | 700 ms settle | 100 + 60 ms × index | 1040 ms |
| E Instrument | letters | opacity 0 | 500 ms linear | 0 | 500 ms |
| | scale | scaleX 0 from the left | 700 ms settle | 0 | 700 ms |
| | index | −600 units x, opacity 0 | 800 ms settle | 300 ms | 1100 ms |
| all | status line | opacity 0 | 300 ms linear | 1200 ms | (only on slow boot) |
| all | hand-off | cross-fade to the target view | 200 ms | when the core is ready | — |

## Review guide

For the astra + opus UX review. Start with `overview.1440x900.dark.png` and `overview.1440x900.light.png`,
then each concept's start screen at both sizes and themes, then the `motion.*` frames, then `assets.png`.

Questions to answer:
1. **Pick one** (A–E) for the start screen and as the wordmark — or say what a round 2 must change.
2. **Fast but calm?** Does A read as "fast" without tipping into gamer/Zwift territory (DESIGN.md tone)?
   Is B too busy? Is E too quiet for the owner's brief?
3. **One system with the icon?** Does the 50° tie (line starts, cut ends) register, and do graphite + brass
   carry over to the light surface (`#94642d` on paper)?
4. **Legibility:** does any cut make a letter ambiguous (A's crossbar, R's bowl, K's junction)? Is 160 px the
   right minimum (`assets.png`)?
5. **Start-screen behaviour:** are the rules above (cold start only, no minimum time, never waits for
   devices, 200 ms hand-off cutting the motion, status line after 1.2 s) complete and right?
6. **Motion:** is ≤ 1.2 s with one settle curve right, and is A's "lines drawn out behind the word" the right
   gesture — or should the start screen borrow D's crank turn?

Out of scope for this PR: building it into `design/prototype` (#62) and the production app (#28).
