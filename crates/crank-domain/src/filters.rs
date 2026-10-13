//! Display filtering in ride seconds, ported from `shared/src/filters.ts`.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// A display filter. Parameters use seconds or Hz; median uses a sample count.
/// Finite parameters are expected. Negative windows/tau/beta are clamped to zero.
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
    /// Median of the most recent valid samples; even counts average the middle pair.
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
            sum: 0.,
            count: 0.,
            value: None,
            derivative: 0.,
            last_t: 0.,
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
                if self.count > crate::js_round(n).max(1.) {
                    self.values.pop_front();
                    self.count -= 1.;
                }
                let mut sorted: Vec<_> = self.values.iter().map(|p| p.1).collect();
                // JS's comparator treats NaN differences as equal.
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let m = sorted.len() / 2;
                if sorted.len() % 2 == 1 {
                    sorted[m]
                } else {
                    {
                        // Keep TS sum-then-divide overflow behaviour.
                        let sum = sorted[m - 1] + sorted[m];
                        sum / 2.
                    }
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
/// NaN is a value (and may poison state), as in TS; callers use `None` for gaps.
/// Empty chains are identity; backwards time resets every stage before the sample.
#[derive(Debug)]
pub struct FilterChain {
    stages: Vec<Stage>,
    last_t: f64,
}
impl FilterChain {
    /// Construct an initially empty chain state from filter descriptions.
    #[must_use]
    pub fn new(filters: impl IntoIterator<Item = Filter>) -> Self {
        Self {
            stages: filters.into_iter().map(Stage::new).collect(),
            last_t: f64::NEG_INFINITY,
        }
    }
    /// Feed one sample at finite ride time `t` in seconds.
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
