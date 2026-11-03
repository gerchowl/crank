# crank — wordmark + start screen

Issue: [#55](https://github.com/gerchowl/crank/issues/55). Date: 2026-10-13. Status: **round 2**. Both UX
reviews of round 1 (astra, opus) ranked **A · Slice** first, so it is the chosen direction; round 2 fixes its
construction and size rules (see "Review resolution"). Follow-up: [#62](https://github.com/gerchowl/crank/issues/62)
builds the start screen into `design/prototype` once #49 has merged. **Launch motion:** F · Spin-up (#83) is
adopted in place of A's letter settle — see "F · Spin-up"; A's static form is unchanged.

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
| `design/brand/src/spin.ts`, `scripts/spin.ts` → `src/spin-art.ts` | **F · Spin-up** (#83): motion as a pure function of time + its build-time geometry (GENERATED) — see "F · Spin-up" |
| `docs/design/brand/spin/spin.<w>x<h>.<theme>.<t>ms.png`, `spin.storyboard.*.png` | F every 100 ms, 0–1200 ms, both sizes and themes; the storyboard sheets |
| `docs/design/brand/compare.slice-vs-spin.{dark,light}.png` | **A vs F side by side**, 1440×900, every 100 ms |
| `docs/design/brand/spin.{1440x900,1024x768}.*.png` | F under reduced motion (= A at rest, checked pixel for pixel) |
| `docs/design/brand/spin.handoff-100ms.*`, `spin.hold.*`, `spin.small.*` | F 100 ms into a hand-off; the `?hold` variant at 1.9 s; F at a 160 px wordmark |
| `docs/design/brand/spin.early-handoff.<w>x<h>.<theme>.png` | F handed off at readiness 149 / 151 / 300 / 450 ms, 0–150 ms into the fade |
| `docs/design/brand/spin*.mp4` | F in real time (60 fps, both themes) and at 4× slow motion (both themes; `pnpm video`, needs ffmpeg) |

`pnpm -C design/brand screens` regenerates the outlines, re-exports the assets and renders every PNG (Vite +
Playwright, timers on Playwright's fake clock). It fails on: page errors; overflow; a wordmark leaving the
window; launch motion over 1.2 s; the boot status showing before 1.2 s or missing after it (with and without
reduced motion); a destination that is not on top and focused as the hand-off begins; the start screen not
removed at once under reduced motion; any size-sheet item not rendered at its stated size. `pnpm export`
refuses to write an asset containing `clipPath`, `mask` or an `id`. Known gap: A–E's `motion.*` frames are not byte-stable
across runs (CSS on the real-time document timeline) — [#127](https://github.com/gerchowl/crank/issues/127); F's
frames are.

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
  DESIGN P6 (see "Boot failure" below).

**Boot failure (designed in #62).** If the core does not come up, the status line is replaced **at once** (it
does not wait for 1.2 s) by a `role="alert"` block in `--ink`, in the order of DESIGN P6: **what broke** ("crank
can't start. The ride core didn't come up: <reason>."), **what the app did** ("Your rides and settings were not
touched, and nothing is recording."), **what to do** (a *Try again* button, and "If it fails again, quit and reopen
crank."). The launch motion is not cut and the screen is not inert. *Try again* is a fresh boot attempt with its
own 1.2 s status clock. The reason text comes from the core's error; the prototype simulates it.

**In the prototype (#62).** `design/prototype/src/StartScreen.tsx` + `start.css`, wired in `App.tsx`; the wordmark is
imported from `design/brand/src/wordmark.ts` (the outlined SVG, no font). The window background is painted in the
start screen's `--bg` from `index.html` before any script runs (the web analogue of the native window background,
#28). The mock core boots instantly, so the screen shows only when `?boot=<ms>` (ready after ms) or
`?boot=fail[:<ms>]` simulates a cold start (`src/boot.ts`). `pnpm -C design/prototype test:start` checks the rules
above; `pnpm -C design/prototype screens` renders `docs/design/screens/start-*.png`.

**Layout.** Letters centred in the window (lines trailing left), wordmark at `min(52vw, 820px)` wide, status
line 48 px above the bottom edge. Nothing else: no version, no tips, no progress bar.

The spike demonstrates all of this: `?concept=slice&ready=<ms>` simulates the core becoming ready
(`src/main.ts`); `pnpm screens` checks it on a fake clock and captures `slice.slow-boot.*` and
`slice.handoff-100ms.*`.

## Motion

A's launch motion, kept as the record; the adopted launch motion is **F · Spin-up** (below). One curve: **settle** = `cubic-bezier(0.16, 1, 0.3, 1)` (fast start, long decelerating tail, no overshoot).
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

## F · Spin-up — adopted launch motion (#83)

Owner's idea (2026-10-13): *the app mark rotates, spinning faster and faster until it becomes a circle, then
forms into "crank"*. F is a **launch motion only**: its settled frame and every static asset are A · Slice,
unchanged. **Decision (round 2): F is the start screen's launch motion**, replacing A's letter settle. Both
independent reviews of the A-vs-F round chose F (astra, opus; summary below). Integration into
`design/prototype` is [#138](https://github.com/gerchowl/crank/issues/138) (done: `src/StartScreen.tsx` paints
`frame(t)` per animation frame, `pnpm -C design/prototype test:start` covers it; `?hold` is not ported); the
production shell follows in the UI waves (#88+).

**Sequence.** The mark (`logo.svg` without the squircle, ring outer diameter 1.5 H ≈ 190 px at 1440×900,
centred on the letters) spins up on its hub; the arms' sweep closes the C's gap into a solid brass ring
(**O**); the ring glides left into the C's place and opens into the wordmark's C, turning from brass to
ink; R A N K come out of the C left → right, still **uncut**; then **one slice** on the 50° crank axis runs
right → left through the word, opening the two cuts behind it, and on along the speed lines until it stops
on their chamfers. It lands on A at rest — `pnpm screens` fails unless F's final frame **and** F under
reduced motion are pixel-identical to A's start screen (both sizes, both themes).

### Reviews (round 1 → 2)

Both independent reviews of PR #131 (astra, opus) chose **F**: the dock icon's mark continues into
the window and turns into the name in one gesture; it reads as a crank winding up (rest at the 50° pose,
visible acceleration, ~2 turns, a definite end), not a spinner; tone stays precise and calm (one curve
family, no bounce, no glow). Both confirmed the slow-boot default (b) and keeping `?hold` (b′) as a record
only. Findings and their resolution:

| # | Finding | Resolution (round 2) |
| --- | --- | --- |
| 1 | MAJOR (opus): the cuts crossfade (830–980 ms) left grey translucent bars across the C; (astra) the uncut C looked heavier than the already-cut RANK at 700–800 ms | No crossfade, no translucent ink. R A N K come out **uncut** like the C, so the word has one weight; one slice front on the 50° axis opens the cuts geometrically (A's cut letters shown behind the front, the uncut ones ahead of it) and continues as the speed lines, ending exactly on their chamfers. Each letter's cut + uncut paths fade in as one group, so cut bands never show through. |
| 2 | MAJOR (astra): early hand-offs need visual proof — no snap or rushed morph over the destination | Round 1 cut to the wordmark at ≤ 150 ms and fast-forwarded later (a snap at 149 ms, a rush at 151 ms). Now the motion **runs on at 1:1 under the 200 ms fade** for every readiness. `spin.early-handoff.*` shows ready at 149 / 151 / 300 / 450 ms, both sizes and themes; `screens` checks t runs at 1:1, the fade is linear, the screen is gone after 200 ms and 149 vs 151 ms differ visibly in < 1 % of pixels |
| 3 | MINOR (opus): 1–2 px step on the O at 3 o'clock, 450–500 ms (the overlapped faces crossed) | The ring's two faces start **collinear** on one radius and part without crossing; the zero-width slit is closed by a stroke in the shape's colour (6 units → 0 over the first 10 % of the glide) — ~~superseded by [#142](https://github.com/gerchowl/crank/issues/142): the stroke ended before the faces were a pixel apart; see "Ring → C"~~ |
| 4 | MINOR (both): ghost arms + pedal eyes beyond the ring; 3 ghosts read as fan blades / a ship's wheel | Ghosts are clipped to the ring's outer radius, **2** of them (opacity 0.35 / 0.15, lag ≤ 60°); the crank's own pedal end beyond the ring blurs out first (gone by 60 % speed), so no translucent bead |
| 5 | MINOR (opus): 20 ms headroom under the budget | At rest by **1080 ms**; `screens` now fails above **1100 ms** |

### Timing (ms from the first frame; `SPIN` in `src/spin.ts`)

| Phase | Start → end | Curve | What moves |
| --- | --- | --- | --- |
| spin-up | 0 → 420 | constant angular acceleration | crank turns clockwise (forward pedalling, drive side); trails grow; tip blurs out by 60 % speed, arms from 45 % |
| ring closed | 420 | — | trails (2 × 186°) cover the gap: a solid O; arms gone |
| glide + open | 420 → 760 | `glide` cubic-bezier(0.3, 0, 0, 1) | ring → C outline, centre → C's place, 1.5 H → cap height, brass → ink (Oklab) |
| R A N K | 635 / 670 / 705 / 740, 260 ms each | `settle` (A's) | uncut, from −60 units, opacity 0 — R only once the ring has cleared it |
| slice | 780 → 1080 | `settle` | front x = f − y/tan 50° from right of all ink to the chamfer line (f = left − lead): cuts open behind it, then the speed lines draw out behind it |
| at rest | **1080** (target ≤ 1100, checked) | — | the DOM is A's paths only |

### Physics and rendering

- **Angular acceleration:** θ(t) = ½ α t² from rest, α = ω_max / 420 ms = 7 380 °/s² (20.5 rev/s²);
  ω_max = 3 100 °/s (8.6 rev/s). About **1.8 revolutions** in the spin-up — enough turns to read as a
  crank wound up under constant torque, not a dial flicking once. Speed s = ω/ω_max rises linearly.
- **Motion blur = arc trails, no raster blur.** Each arm drags a trail through the ring band (radii
  287.5–412.5, the C's stroke) spanning ω·τ behind it, τ = **60 ms** persistence, capped at **186°**: two
  arms 180° apart close the ring exactly at ω_max; the 6° overlap is between the two *trails* (abutting sector
  edges would conflate to a hairline of their own) — the ring→C faces are a separate seam, closed below. Each
  trail is **4 nested sectors** (the nearest lag covered by all four, the oldest by one) of opacity
  0.3 + 0.7 s², so it is stepped and fades with lag at low speed and saturates to solid brass at ω_max.
  τ is 3.6 frames at 60 Hz and every frame's step (≤ 52°) lies inside the trail: **no wagon-wheel strobe**.
- **Arms:** two ghost copies of the crank inside the ring (lags up to 60°, opacity 0.35 / 0.15 × s) blur
  the arms; arms and ghosts fade with 1 − ((s − 0.45)/0.55)², gone at ω_max: at full speed a crank *is* a
  steady blur. The crank is split at the ring's outer radius; its tip (pedal end + eye), the fastest point,
  fades out between 30 % and 60 % speed. Until ω_max each trail stops short of the next arm's by 180° − sweep
  (a dark wedge ahead of every arm that narrows to nothing at 420 ms): it rotates, so it crosses 3 o'clock for
  a millisecond or two at ~400 ms. That is the blur closing, not a seam, and is by design.
- **Ring → C:** the ring (as placed at 420 ms) and the C are each a 616-vertex outline in four matched
  runs — outer arc (320), top terminal face (48), inner arc (200), bottom terminal face (48). Both ring
  faces start on the 3 o'clock radius and part into the C's opening. **The slit between them** is filled in the
  shape's colour (`morph.seam`, stroked `SEAM_PX + 1` = 2 px so it overlaps each face rather than abutting it)
  until even its narrowest point is `SEAM_PX` = 1 px apart on screen; then it is dropped and the gap only
  widens. (Round 2 closed it with a 6-unit stroke tapering over the first 10 % of the glide, which ended while
  the faces were still under a pixel apart: a sub-pixel hairline at 435–444 ms that read as a crack, #142.)
  Every C vertex lies **on** Barlow's outline; `pnpm outline` fails if a chord strays by more than 0.5 units (actual 0.34 ≈ 0.06 px at
  1440×900), so the shape that lands is the C, and A's (uncut) C outline takes over under the same pixels.
- **Slice:** two `clipPath`s (page-only; the exported assets stay id-free) hold the half-planes ahead of and
  behind the front: the uncut letters are clipped to the half-plane ahead, the speed lines to the one
  behind; A's cut letters are always drawn. The front is parallel to the chamfers and ends on them, so at
  rest both clips are removed with no visible change.
- **Deterministic:** `frame(t)` is pure; the page paints it per `requestAnimationFrame` from
  `performance.now()` (a paused fake clock drives it in `pnpm screens`), and `?t=<ms>` renders any instant.
  No CSS animation, no WAAPI: path morphing by CSS `d` does not run in WKWebView (the production webview).
- **Why plain TS here too:** the same constraints as A (see "Why plain TS + Vite"); integration only
  needs `frame` + `paint` and a clock.

### Boot behaviours

| Boot | Behaviour (checked on the fake clock) |
| --- | --- |
| (a) ready during the motion — any time, including the first frames | hand-off as A: destination live + focused at once, start screen `inert`, 200 ms linear fade; F's motion **runs on at its own pace** under the fade — no freeze (a frozen blur reads as a glitch), no cut, no fast-forward (round 1's snap at ≤ 150 ms and rush after it were conspicuous over the live destination) |
| (b) slow boot — **default** | the sequence plays through and **rests** on the wordmark (as A); status line from the boot timer at 1.2 s; fade on ready |
| (b′) slow boot, `?hold` (record only) | the spin **sustains at ω_max** (no further acceleration) until ready — at full speed that is the steady closed ring; status at 1.2 s; on ready it runs on under the fade. Not ported (#138) |
| reduced motion | no motion, no script: the markup at rest is the wordmark; status at 1.2 s; removed at once on ready |
| boot failure | as "Boot failure" above: the alert replaces the status at once; F's motion is not cut — it plays through and rests on the wordmark; "Try again" does not restart it. Wired in `design/prototype` (#138) |

**Why (b) and not (b′) as default** (confirmed by both reviews). A held spin is a progress spinner, which
the start screen rules out ("no progress bar"); and since the hand-off begins the moment the core is
ready, a morph gated on readiness could only ever play inside the 200 ms fade — on every boot slower than
420 ms the rider would never see the C form at its designed pace.

### Small sizes

F only ever runs as the start screen, whose wordmark is ≥ 532 px wide at the smallest supported window
(1024×768, DESIGN §8). `spin.small.*.png` shows it at the full form's 160 px floor at 2×: the ring band is
≈ 6 px, the stepped trails and the O still read, and the settled frame is the full form. Below 160 px the
start screen would use A's small form **without motion** (cut-free letters cannot carry the slice; a ring
band under ~6 px closes into a blob). Static size rules are A's, unchanged.

### Review guide (round 2)

1. `compare.slice-vs-spin.dark.png` / `.light.png` — the two launches side by side every 100 ms.
2. `spin.slowmo-4x.1440x900.dark.mp4` and `.light.mp4` (the light theme was not reviewed in round 2, #142), then
   `spin.1440x900.dark.mp4` — the slice through the uncut word at 780–1080 ms (finding 1), the clean O and the
   slit at 420–480 ms (3; closed until the faces are 1 px apart, then only widening, #142), the two ghosts at
   200–400 ms (4).
3. `spin.early-handoff.{1440x900,1024x768}.{dark,light}.png` — readiness at 149 / 151 / 300 / 450 ms (2).
4. `spin.storyboard.*`, `spin.small.*`, `spin.handoff-100ms.*`, `spin.hold.*`.

Reproduce: `pnpm -C design/brand screens` (checks + all PNGs); videos with
`nix develop -c nix shell nixpkgs#ffmpeg-headless -c pnpm -C design/brand video`.
