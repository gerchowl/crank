# Backend / contract issues surfaced by the spikes

- [ ] **B1 Per-lap stats** (actual start/end, avg/NP power, avg HR, cadence, compliance by the backend's definition, τ if
  recovery) so no UI computes laps. (X2)
- [ ] **B2 Explicit thresholds and flags** in the contract: target band, τ "slower than usual" flag, W′bal floor, delta_target already
  exists (use it). (X2)
- [ ] **B3 Live cursor in actual time**: current block's actual start and elapsed, so the strip marker doesn't drift (Solid L8).
- [ ] **B4 planner.propose is deterministic per check-in**, so "Regenerate" returns the same proposal (Solid report). Add a seed/variant param.
- [ ] **B5 Gaps worked around in mock v0**: `decisions()` lacks ratings; no snippet-save API; `Bucket` can't express "no data";
  `MmpCurve.today` uses 0 beyond ride length; coach changes don't update the `ride.current()` tree.
- [ ] **B6 Mock realism**: 1-component fit vs 2-component simulated heart (ready-in wobbles ±20–30 s early); the default seed never
  emits push/hold nudges; the unreachable HR-recovery path is never triggered by the demo.
- [ ] **B7 Pending additions** (spec): CP/W′ anchor + athlete model with uncertainty (ONBOARDING.md), AEPF/CPV/torque, OmPD fit,
  durability, stamina, carbs/fuel, trends.
- [x] B0 Full-ride streams + finished recoveries (`streams()`, `recoveries()`), done in e1c1d14.
- [ ] B8 Root `pnpm-workspace.yaml`: Solid added a `minimumReleaseAgeExclude` for typescript-eslint 8.71.2 (removable after
  2026-10-13T17:10Z); Svelte instead pinned 8.71.1. Unify when merging tooling.
