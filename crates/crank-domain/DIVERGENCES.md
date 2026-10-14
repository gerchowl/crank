# Intentional divergences from the frozen TypeScript reference

`shared/` is frozen and is the parity reference (ADR-0001 §3). Where `crank-domain`
deliberately behaves differently, the difference is listed here with the TS behaviour, the Rust
behaviour, the reason, and the test that pins it. Every other behaviour is covered by the
golden vectors in `tests/vectors/` (see its README). Add a row here in the same PR that
introduces a divergence; a divergence without a row is a bug.

All divergences are limited to inputs for which the reference is unbounded, non-terminating
or yields `NaN`/`Infinity`, plus the one contract change in #148.

| # | Area | TS reference | Rust | Why | Pinned by |
|---|------|--------------|------|-----|-----------|
| #148 | `histogram_from`, `heatmap_from` | A finite zero or negative bin width is normalised to `1`. | `MetricError::InvalidBinWidth` for every nonpositive or non-finite width, including on empty input. | Round 2 of #66: silently substituting a width hides caller bugs. Callers pass a finite positive width. | `tests/metrics_safety.rs`: `nonpositive_bin_widths_are_typed_errors_not_normalised`, `histogram_arbitrary_f64`, `heatmap_arbitrary_f64`; width-zero vectors in `tests/metrics_vectors.rs::distribution_vectors`. |
| #150 | `MultiResSeries::query` | Aggregates by incrementing an `f64` slot counter (`s++`); at timestamps around 1e308 adding one does not advance it and the loop never terminates. | Visits the retained slots in ascending timestamp order, so work is bounded by ring capacity and at most 2000 buckets. Non-finite or negative `now` and non-finite or nonpositive spans return an empty result. Summation order for normal finite input is unchanged. | Round 2 of #66 requires bounded queries for arbitrary `f64`. | `tests/metrics_safety.rs`: `history_query_terminates_at_extreme_finite_timestamps`, `history_query_arbitrary_f64`; `tests/metrics.rs::history_tiles_and_matches_brute_force`. |
| #155, #186 | `typical_distribution` (power metrics only) | No FTP check; a zero, negative or overflowing FTP propagates `NaN` into the distribution. | `InvalidMetricParameter("ftp")` unless FTP is finite, positive, and its power components are finite with nonzero deviations. `hr`, `cadence` and `pct_ftp` do not read FTP and never reject it, as in TS. | The callback must never yield `NaN`. Scoped to power in #186 item 3, so the divergence is exactly the power case. | `tests/metrics_safety.rs`: `typical_distribution_rejects_invalid_ftp_for_power_metrics`, `typical_distribution_ignores_ftp_for_ftp_free_metrics`, `typical_distribution_error_kind_per_ftp_class`. |
| #155 | `histogram_from` / `heatmap_from` ranges | No bound on the grid; an unfittable range can allocate without limit. | Histogram: `TooManyBins` / `NonFiniteRange`. Heatmap clamps each axis to 400 bins (this clamp matches TS and is not a divergence; only the overflow errors are). | Bounded memory for arbitrary `f64`. | `tests/metrics_safety.rs::histogram_overflow_range_is_typed_error`, `histogram_arbitrary_f64`, `heatmap_arbitrary_f64`. |
| #169 | `fit_mono_exp`, recovery state, `dfa_alpha1`, HRV | Finite inputs whose arithmetic overflows return `NaN` weights/fits; a zero DFA box size does not terminate; unbounded work. | `MetricError::NonFiniteCalculation`, `InvalidMetricParameter(..)`, or a computation-limit error, chosen per input class. TS's length guard still runs before box-size validation (`dfa_length_guard_precedes_box_size_validation`); an overflowed grid-point cost still gets zero weight (`overflowed_cost_grid_points_get_zero_weight_as_in_ts`). | Nonfinite output and non-termination are not representable results. | `tests/metrics_group_b_safety.rs`: `all_overflowed_costs_are_a_nonfinite_calculation`, `squares_that_overflow_are_a_nonfinite_calculation`, `nonfinite_observation_is_a_nonfinite_calculation`, `finite_invalid_parameters_keep_ts_results`, `dfa_error_kind_per_class`, `horizon_above_one_day_is_a_computation_limit`. |
| #160 | `compile_workout` and helpers | No bounds; may hang, allocate without limit, or emit `NaN`/`Infinity` durations from finite inputs. | Typed `CompileError`s: `NestingTooDeep` (32 repeats), `ExpansionLimit` (100,000 visits), `TraceLimit`, `InvalidArithmetic` (nonfinite arithmetic needed for output), `TimelinePrecision`, `NonFinite` (supplied NaN/Infinity). Invalid prescriptions stay diagnostics, as in TS. | Safety bounds; no frozen vector is changed. Details: rustdoc of `compile_workout`. | `tests/compile.rs`: `bounded_expansion_and_depth`, `explicit_nonfinite_and_helper_edges`, `finite_arithmetic_failure_is_not_nonfinite_input`, `zero_ftp_hard_step_then_hr_recovery_is_invalid_arithmetic`, `adversarial_numbers_are_values`. |

## Numerical parity that is observed, not guaranteed

These are not behavioural divergences, but they are the places where parity depends on the
platform's maths library.

- `hr_recovery.rs` computes the τ grid with `f64::powf`, which delegates to the platform libm.
  Agreement within the vectors tolerance with the TS reference is **observed on the linux and macOS CI
  matrix, not guaranteed**: a one-ulp change at one grid point moves a weight by about 1e-11
  relative, past the vectors' 1e-12 tolerance. The CI matrix (`recovery_vectors`) is the guard;
  a libm update that breaks it fails CI with no code change. Exponentials and logarithms use
  pure-Rust `libm`, which does not have this exposure (#169); `libm::pow` is not used for the
  grid because it differs from the reference by an ulp there.
