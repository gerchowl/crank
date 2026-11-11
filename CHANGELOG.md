# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Releases are cut as described in [RELEASING.md](RELEASING.md); add entries under `## Unreleased`.

## Unreleased

### Added

- Route model and import, no UI or store yet (ADR-0005 §2.1, §2.2.1–2.2.2, §2.2.4–2.2.5): `crank_domain::route`
  types, a hardened GPX 1.0/1.1 reader (`quick-xml`; a DOCTYPE, any entity but XML's five and nesting deeper than 32
  are refused, every element must stand where the GPX XSD content model puts it, and everything inside `<extensions>`
  is skipped whatever its namespace, so Strava's unprefixed `<power>` imports), the §2.2.4 limits and the
  missing-elevation rules with an import report, multi-track files listing their tracks, and grade-profile text
  (`2km 3%, 3x(0.4km 12% cobbles, 0.6km 2%)`) with a canonical printer
  ([#539](https://github.com/gerchowl/crank/issues/539))
- ESLint design rules in `pnpm -C app lint` (DESIGN §18.7 rules 1, 2, 4 and 7): Lucide-only icons (`lucide-react` 1.62.0), no raw
  `<svg>` outside `src/icons/`, no literal colours or px in `style={{}}`, `eslint-plugin-jsx-a11y` recommended; a `<Num>`
  component for live numbers; the Feather MIT text in `NOTICE`
  ([#455](https://github.com/gerchowl/crank/issues/455))
- Bug-report log tail (no UI yet): the last 200 of vigil's OTLP records, each reduced to its timestamp, level, module,
  event name and typed allowlisted fields; messages and error text are never included, and identifier fields (names,
  serials, addresses, ids, paths) become placeholders by key; a privacy pinning test salts every leak spelling of #482
  ([#508](https://github.com/gerchowl/crank/issues/508))
- Ready sheet in direction A (DESIGN §5.4): before Start the ride screen's traces column states whether the ride can
  start (loading, connecting, ready, trainer missing, strap missing, a refused Start) with one row per device, the mode
  and how the ride starts; Start is disabled while the trainer link is connecting or lost, and focused once it can be
  pressed. Six design fixtures (`ready*`) ([#526](https://github.com/gerchowl/crank/issues/526))
- Link keeper (ADR-0002 §5.1, no UI yet): `Core::links_intent` warms the paired trainer and strap up in parallel while a
  ride is likely and lets them go after 5 min idle, 60 s in the background or at once on hide, sleep, Devices or
  "Release trainer" (which latches the trainer); nothing is written while warm. `ride_load` takes warm links with no
  Bluetooth work and awaits running attempts; a loaded ride that is not started is discarded after 10 min without
  input or on sleep, and "Release trainer" on it unloads it (refused during a started ride); a `not_found` device is
  retried on its advertisement in a capped scan (30 s continuous, then 4 s every 30 s); quitting releases the keeper's
  links within the cap; the simulator gets a "not advertising" latency
  ([#469](https://github.com/gerchowl/crank/issues/469))
- Settings and Devices in direction A (DESIGN §5.9): the section list and one panel on the 12-column grid, 32 px controls;
  Devices as two tables (Paired, Nearby) with a hint before the first scan, and Forget asks first, in place of the row
  (Cancel focused, Esc cancels). Pair buttons' names start with their visible words (WCAG 2.5.3). Three new design
  fixtures (`devices-found`, `devices-pairing`, `devices-forget-confirm`) and a whole-screen no-clip check for Settings
  and Devices ([#527](https://github.com/gerchowl/crank/issues/527))
- Workouts in direction A (DESIGN §5.7): the library (7 columns) and the picked workout's panel (5) on the 12-column grid;
  the library is a listbox (↑ / ↓ / Home / End pick, one tab stop) whose rows wrap a long name; the panel shows the
  profile, Ride this (32 px, "Loading ride…" while its load is in flight), the compiled summary with time-in-zone bars, and every
  block. Six new design fixtures (`workouts-empty`, `workouts-list-error`, `workouts-blocked`, `workouts-loading`,
  `workouts-load-error`, `workouts-long`) and a whole-screen no-clip check for Workouts
  ([#528](https://github.com/gerchowl/crank/issues/528))
- Review and History in direction A (DESIGN §5.5, §5.6): Review shows the ride just ridden's power, heart-rate
  (80–200 bpm) and cadence traces on one time axis beside its figures, then planned against ridden; History is a table
  of rides, newest first, each with a 32 px Review button, marking the ride last opened. Eight new design fixtures
  (`review-*`, `history-*`) ([#529](https://github.com/gerchowl/crank/issues/529))
- Today in direction A (DESIGN §5.1): the placeholder becomes four panels on the 12-column grid. Today's ride shows
  the load started in Workouts, with Start ⏎ once it is loaded, or "Nothing planned" with Pick a workout. Devices shows
  who is paired and the loaded ride's links, and leads to pairing when no trainer is paired. Last 7 days shows the
  minutes ridden each local day (TSS once the summary has it, [#570](https://github.com/gerchowl/crank/issues/570)),
  and Last ride the newest ride with Review. The ride-load notice leads the main region, styled with tokens and 32 px
  buttons. Eight new design fixtures (`today-*`, `settings-ride-loaded`)
  ([#530](https://github.com/gerchowl/crank/issues/530))
- Link contract (ADR-0002 §5.1.4, no UI yet): the `links_intent` command answers the keeper's state with its number
  (`LinkFrame`: `seq` is +1 per change, shared by every frame and answer, and a frame or answer is applied only if its `seq`
  is at least the highest applied so far, from either source);
  `links_open` opens the window's `Channel<LinkFrame>` (each frame the complete state: every paired device's phase and
  a warm device's latest sample), acked by `links_ack` with one frame in flight, changes coalesced into the next frame
  and at most 10 Hz; `device_links` reports the keeper's `phase`
  ([#470](https://github.com/gerchowl/crank/issues/470))

### Changed

- Rail item height (DESIGN §18.4): the labelled rail item has its own token, `--control-h-rail` (55 px, derived from its
  border, inset, icon, gap and word), instead of breaking the 40 px rule; the layout audit holds it to that token, and
  every railed design fixture drops its rail-only `button.height` and `button.inconsistentHeight` counts
  ([#555](https://github.com/gerchowl/crank/issues/555))
- `ride_status`: `loading: bool` becomes `load: LoadProgress | null`, the trainer's and the strap's phase while a load
  is under way (ADR-0002 §5.1.4) ([#470](https://github.com/gerchowl/crank/issues/470))
- CI and releases (ADR-0001 §9.6): the required PR gate is Linux only; macOS runs on `release/*` branches, after
  every merge into `dev` and on request. Release candidates `X.Y.Z-rc.N` (bundle version `X·1000000 + Y·10000 + Z·100 + N`, final
  `… + 99`) are cut into `dev` and published as GitHub pre-releases with the zip, its `.sha256` and a build-provenance
  attestation (`.sigstore.json`); final releases wait for Developer ID signing (#426)
  ([#485](https://github.com/gerchowl/crank/issues/485))
- Rail: 76 px wide with 12 px words (`--text-xs`) and `--space-1` side padding, so "Workouts" at 600 weight keeps 5.6 px inside its button (it spilled 2.75 px); content is 908 px at 1024 and 1300 px at 1440. The design suite asserts ≥ 4 px per rail word; DESIGN §6.1, §16.2, §16.3, §17.6 and §18.2 follow (#506, #467)

### Deprecated

### Removed

### Fixed

- Ride screen metric column on one grid (DESIGN §18.3 "Metric grid"): the same label-to-value gap and slot-to-slot gap
  in every slot, T2 values right-aligned so W, bpm and rpm share one column, and the target written "250 W" like the
  plan; the block strip's labels sit `--space-1` under the bar, flush left with their block, and shed the name, then
  the watts, then to a leader on a block too short for them. The layout audit checks both (`metrics.*`,
  `strip.label`), with a 30 s-block fixture, and the §18.2 minimum is re-counted to 453 px with the column's borders
  (465 px once #97 brings the power band back) ([#581](https://github.com/gerchowl/crank/issues/581))
- The workout DSL printer escapes a newline inside a label (`\` then the newline), so every label it prints reads
  back as the same label ([#539](https://github.com/gerchowl/crank/issues/539))
- serde_json parses floats exactly (`float_roundtrip`, workspace-wide), so a value read back from JSON has the bits
  it was written with; its default parsing can be one ULP off ([#539](https://github.com/gerchowl/crank/issues/539))
- `hil_fec analyse` (Part B) reads a capture once into one timeline (ADR-0007): stamps must not go back in file
  order and no line may be malformed (else no result), ties count in file order, and checks read typed events only
  (samples, writes, requests, command statuses, measurements, stamps); power and cadence come from updates (the
  page-25 count changed); checks 6, 9 and 12 must write exactly their procedure's control writes, and a check 6
  that does not measures no power lag; check 8 needs 3 fresh speeds per segment; the live silence guard and the
  analyser share one threshold; the reply accounting no longer panics on a missing line. The live writer stamps
  each line under the capture lock (`Capture::record_now`, `record_aux_now`), so file order is time order
  ([#516](https://github.com/gerchowl/crank/issues/516))

### Security

## [0.1.0](https://github.com/gerchowl/crank/releases/tag/0.1.0) - 2026-11-03

### Added

- stylelint (17) in `pnpm -C app lint`: token-only colours and sizes, no unknown `var(--…)`, no `outline: none`
  (DESIGN §18.7 rules 1 to 3, [#454](https://github.com/gerchowl/crank/issues/454)); the shell and ride stylesheets
  now use the theme tokens, so a few sizes moved to the nearest token
- Release workflow adopted from devkit: `prepare-release`, `release` and `abandon-release`
  ([#428](https://github.com/gerchowl/crank/issues/428))
- Direction A design tokens in `theme.css` (space, radius, type, control height, graphite palette in both themes)
  and locally bundled Inter and JetBrains Mono ([#453](https://github.com/gerchowl/crank/issues/453))
- `ride` and `device` spans in `traces.jsonl`, ended before vigil shuts down; the `coach` span follows with the first coach call
  ([#465](https://github.com/gerchowl/crank/issues/465), [#474](https://github.com/gerchowl/crank/issues/474))

### Changed

- Desktop logs go through vigil: OTLP/JSON Lines `logs.jsonl` and `traces.jsonl` in the log directory, `CRANK_LOG` filter,
  7-day retention and size caps; a `CRANK_CAPTURE_DIR` capture keeps everything. The old `crank.log` is left in place
  ([#414](https://github.com/gerchowl/crank/issues/414))
- Loading a ride connects the trainer and the strap at the same time, so a cold load takes the longer of the two
  connects (at most 15 s) instead of their sum, and each connect phase is logged with its duration
  ([#468](https://github.com/gerchowl/crank/issues/468))

### Fixed

- Devices: a nearby device that is already paired shows "Paired as trainer/strap" with Forget instead of "Pair as"
  for that role ([#436](https://github.com/gerchowl/crank/issues/436))

