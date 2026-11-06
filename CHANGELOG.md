# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Releases are cut as described in [RELEASING.md](RELEASING.md); add entries under `## Unreleased`.

## Unreleased

### Added

- Route model and import, no UI or store yet (ADR-0005 §2.1, §2.2.1–2.2.2, §2.2.4–2.2.5): `crank_domain::route`
  types, a hardened GPX 1.0/1.1 reader (`quick-xml`; a DOCTYPE, any entity but XML's five and nesting deeper than 32
  are refused), the §2.2.4 limits and the missing-elevation rules with an import report, multi-track files listing
  their tracks, and grade-profile text (`2km 3%, 3x(0.4km 12% cobbles, 0.6km 2%)`) with a canonical printer
  ([#539](https://github.com/gerchowl/crank/issues/539))
- ESLint design rules in `pnpm -C app lint` (DESIGN §18.7 rules 1, 2, 4 and 7): Lucide-only icons (`lucide-react` 1.62.0), no raw
  `<svg>` outside `src/icons/`, no literal colours or px in `style={{}}`, `eslint-plugin-jsx-a11y` recommended; a `<Num>`
  component for live numbers; the Feather MIT text in `NOTICE`
  ([#455](https://github.com/gerchowl/crank/issues/455))
- Bug-report log tail (no UI yet): the last 200 of vigil's OTLP records, each reduced to its timestamp, level, module,
  event name and typed allowlisted fields; messages and error text are never included, and identifier fields (names,
  serials, addresses, ids, paths) become placeholders by key; a privacy pinning test salts every leak spelling of #482
  ([#508](https://github.com/gerchowl/crank/issues/508))

### Changed

- CI and releases (ADR-0001 §9.6): the required PR gate is Linux only; macOS runs on `release/*` branches, after
  every merge into `dev` and on request. Release candidates `X.Y.Z-rc.N` (bundle version `X·1000000 + Y·10000 + Z·100 + N`, final
  `… + 99`) are cut into `dev` and published as GitHub pre-releases with the zip, its `.sha256` and a build-provenance
  attestation (`.sigstore.json`); final releases wait for Developer ID signing (#426)
  ([#485](https://github.com/gerchowl/crank/issues/485))
- Rail: 76 px wide with 12 px words (`--text-xs`) and `--space-1` side padding, so "Workouts" at 600 weight keeps 5.6 px inside its button (it spilled 2.75 px); content is 908 px at 1024 and 1300 px at 1440. The design suite asserts ≥ 4 px per rail word; DESIGN §6.1, §16.2, §16.3, §17.6 and §18.2 follow (#506, #467)

### Deprecated

### Removed

### Fixed

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

