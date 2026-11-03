# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Releases are cut as described in [RELEASING.md](RELEASING.md); add entries under `## Unreleased`.

## Unreleased

### Added

- ESLint design rules in `pnpm -C app lint` (DESIGN §18.7 rules 1, 2, 4 and 7): Lucide-only icons (`lucide-react` 1.62.0), no raw
  `<svg>` outside `src/icons/`, no literal colours or px in `style={{}}`, `eslint-plugin-jsx-a11y` recommended; a `<Num>`
  component for live numbers; the Feather MIT text in `NOTICE`
  ([#455](https://github.com/gerchowl/crank/issues/455))
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

### Deprecated

### Removed

### Fixed

- Devices: a nearby device that is already paired shows "Paired as trainer/strap" with Forget instead of "Pair as"
  for that role ([#436](https://github.com/gerchowl/crank/issues/436))

### Security
