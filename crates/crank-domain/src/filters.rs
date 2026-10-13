//! Display filtering in ride seconds, ported from `shared/src/filters.ts`.
use crate::metric_error::MetricError;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// A display filter. Parameters use seconds or Hz; median uses a sample count.
/// Non-finite parameters are rejected by `FilterChain::new`. Finite negative
/// windows/tau/beta are clamped to zero, preserving the TS normalization.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Filter {
    /// Sliding mean; samples exactly on the old boundary are removed.
    Sma {
        /// Window in ride seconds.
        #[serde(rename = "windowS")]
        window_s: f64,
    },
    /// Exponential mean, independent of source sample rate.
    Ema {
        /// Time constant in seconds; zero passes values directly.
        #[serde(rename = "tauS")]
        tau_s: f64,
    },
    /// Median of the most recent `n` non-missing samples. A NaN sample occupies a window
    /// slot (it is a received sample, unlike `None`) but is never selected as the median;
    /// a window holding only NaN yields NaN. TS sorts NaN with an inconsistent comparator, so
    /// there is no reference result; this definition is the deterministic one (#155).
    Median {
        /// Sample count, rounded like JS and clamped to at least one.
        n: f64,
    },
    /// Casiez 1€ adaptive low-pass filter (derivative cutoff is 1 Hz).
    OneEuro {
        /// Minimum cutoff in Hz, clamped to at least 0.001.
        #[serde(rename = "minCutoff")]
        min_cutoff: f64,
        /// Nonnegative speed coefficient.
        beta: f64,
    },
    /// Replace missing samples with the last value, initially missing.
    Hold,
}

#[derive(Debug)]
struct Stage {
    filter: Filter,
    values: VecDeque<(f64, f64)>,
    sorted: Vec<f64>,
    sum: f64,
    count: f64,
    value: Option<f64>,
    derivative: f64,
    last_t: f64,
}
impl Stage {
    fn new(filter: Filter) -> Self {
        Self {
            filter,
            values: VecDeque::new(),
            sorted: Vec::new(),
            sum: 0.,
            count: 0.,
            value: None,
            derivative: 0.,
            last_t: 0.,
        }
    }
    /// Keep `sorted` ordered by `total_cmp`, excluding NaN: a binary search finds the slot, so a
    /// push costs O(log n) comparisons plus one memmove instead of a full re-sort.
    fn sorted_insert(sorted: &mut Vec<f64>, v: f64) {
        if !v.is_nan() {
            let at = sorted.partition_point(|x| x.total_cmp(&v).is_lt());
            sorted.insert(at, v);
        }
    }
    fn sorted_remove(sorted: &mut Vec<f64>, v: f64) {
        if !v.is_nan() {
            let at = sorted.partition_point(|x| x.total_cmp(&v).is_lt());
            debug_assert!(sorted.get(at).is_some_and(|x| x.total_cmp(&v).is_eq()));
            if at < sorted.len() {
                sorted.remove(at);
            }
        }
    }
    fn push(&mut self, t: f64, v: Option<f64>) -> Option<f64> {
        if matches!(self.filter, Filter::Hold) {
            if v.is_some() {
                self.value = v;
            }
            return self.value;
        }
        let v = v?;
        let y = match self.filter {
            Filter::Sma { window_s } => {
                self.values.push_back((t, v));
                self.sum += v;
                self.count += 1.;
                while self.values.len() > 1
                    && self
                        .values
                        .front()
                        .is_some_and(|p| p.0 <= t - window_s.max(0.))
                {
                    if let Some((_, old)) = self.values.pop_front() {
                        self.sum -= old;
                        self.count -= 1.;
                    }
                }
                self.sum / self.count
            }
            Filter::Ema { tau_s } => self.value.map_or(v, |y| {
                if tau_s <= 0. {
                    v
                } else {
                    y + (1. - (-(t - self.last_t).max(0.) / tau_s).exp()) * (v - y)
                }
            }),
            Filter::Median { n } => {
                self.values.push_back((t, v));
                self.count += 1.;
                Self::sorted_insert(&mut self.sorted, v);
                if self.count > crate::js_round(n).max(1.) {
                    if let Some((_, evicted)) = self.values.pop_front() {
                        Self::sorted_remove(&mut self.sorted, evicted);
                    }
                    self.count -= 1.;
                }
                let m = self.sorted.len() / 2;
                if self.sorted.is_empty() {
                    f64::NAN
                } else if self.sorted.len() % 2 == 1 {
                    self.sorted[m]
                } else {
                    // Preserve TS sum-then-divide overflow for finite samples.
                    let sum = self.sorted[m - 1] + self.sorted[m];
                    sum / 2.
                }
            }
            Filter::OneEuro { min_cutoff, beta } => {
                let dt = t - self.last_t;
                self.value.map_or(v, |x| {
                    if dt <= 0. {
                        v
                    } else {
                        let alpha =
                            |cutoff| 1. / (1. + 1. / (2. * std::f64::consts::PI * cutoff) / dt);
                        self.derivative += alpha(1.) * ((v - x) / dt - self.derivative);
                        x + alpha(min_cutoff.max(1e-3) + beta.max(0.) * self.derivative.abs())
                            * (v - x)
                    }
                })
            }
            Filter::Hold => v,
        };
        self.value = Some(y);
        self.last_t = t;
        Some(y)
    }
}

/// Left-to-right display filters, fed at source rate, with independent state.
/// `None` passes through all stages except hold, without advancing their state.
/// Median excludes NaN from selection and returns NaN if its window contains
/// only NaN. Other filters propagate IEEE arithmetic (NaN may poison state);
/// hold retains any value, including NaN/infinity. Use `None` for dropouts.
/// Timestamps may be any f64; NaN does not trigger a backwards-time reset.
/// Median keeps its window sorted incrementally (binary search plus one memmove per push, not a
/// re-sort); a strictly O(log n) update needs an order-statistic tree, which window sizes of a
/// few dozen samples do not justify.
/// Empty chains are identity; backwards time resets every stage before the sample.
#[derive(Debug)]
pub struct FilterChain {
    stages: Vec<Stage>,
    last_t: f64,
}
impl FilterChain {
    /// Construct an initially empty chain state from filter descriptions.
    ///
    /// # Errors
    /// Returns `MetricError::InvalidFilterParameter` for any non-finite numeric
    /// parameter, consistently across SMA, EMA, median and one-euro stages.
    pub fn new(filters: impl IntoIterator<Item = Filter>) -> Result<Self, MetricError> {
        let filters: Vec<_> = filters.into_iter().collect();
        for filter in &filters {
            let parameters: &[(&str, f64)] = match *filter {
                Filter::Sma { window_s } => &[("window_s", window_s)],
                Filter::Ema { tau_s } => &[("tau_s", tau_s)],
                Filter::Median { n } => &[("n", n)],
                Filter::OneEuro { min_cutoff, beta } => {
                    &[("min_cutoff", min_cutoff), ("beta", beta)]
                }
                Filter::Hold => &[],
            };
            for &(name, value) in parameters {
                if !value.is_finite() {
                    return Err(MetricError::InvalidFilterParameter(name));
                }
            }
        }
        Ok(Self {
            stages: filters.into_iter().map(Stage::new).collect(),
            last_t: f64::NEG_INFINITY,
        })
    }
    /// Feed one sample at ride time `t` in seconds, including IEEE non-finite values.
    pub fn push(&mut self, t: f64, mut value: Option<f64>) -> Option<f64> {
        if t < self.last_t {
            self.reset();
        }
        self.last_t = t;
        for stage in &mut self.stages {
            value = stage.push(t, value);
        }
        value
    }
    /// Clear all state, including held values and derivative estimates.
    pub fn reset(&mut self) {
        for stage in &mut self.stages {
            *stage = Stage::new(stage.filter.clone());
        }
        self.last_t = f64::NEG_INFINITY;
    }
}
