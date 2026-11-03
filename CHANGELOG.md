# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Releases are cut as described in [RELEASING.md](RELEASING.md); add entries under `## Unreleased`.

## Unreleased

### Added

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

### Deprecated

### Removed

### Fixed

- Devices: a nearby device that is already paired shows "Paired as trainer/strap" with Forget instead of "Pair as"
  for that role ([#436](https://github.com/gerchowl/crank/issues/436))

### Security
