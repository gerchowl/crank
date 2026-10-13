//! Fixed-capacity multi-resolution metric history, in ride seconds.
use crate::scales::{TimeScale, time_ago_edges};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Slot widths in seconds and ring capacities (20 min, 2 h, 4 h, 24 h).
pub const HISTORY_LEVELS: [(u32, u32); 4] = [(1, 1200), (5, 1440), (30, 480), (300, 288)];

#[derive(Clone, Debug)]
struct Acc {
    sum: f64,
    count: f64,
    min: f64,
    max: f64,
}
impl Default for Acc {
    fn default() -> Self {
        Self {
            sum: 0.,
            count: 0.,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
}
impl Acc {
    fn merge(&mut self, other: &Self) {
        self.sum += other.sum;
        self.count += other.count;
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }
}
#[derive(Debug)]
struct Level {
    res: f64,
    cap: f64,
    // Integer-valued doubles preserve the TS slot arithmetic without narrowing
    // timestamps to machine integers. Remainders identify fixed ring positions.
    slots: HashMap<u64, (u64, Acc)>,
    latest: f64,
}
impl Level {
    fn add(&mut self, t: f64, v: f64) {
        let s = (t / self.res).floor() + 0.;
        let entry = self
            .slots
            .entry((s % self.cap).to_bits())
            .or_insert_with(|| (s.to_bits(), Acc::default()));
        if entry.0 != s.to_bits() {
            *entry = (s.to_bits(), Acc::default());
        }
        entry.1.sum += v;
        entry.1.count += 1.;
        entry.1.min = entry.1.min.min(v);
        entry.1.max = entry.1.max.max(v);
        self.latest = self.latest.max(s);
    }
    fn oldest_t(&self) -> f64 {
        if self.latest < 0. {
            f64::INFINITY
        } else {
            (self.latest - self.cap + 1.).max(0.) * self.res
        }
    }
    fn accumulate(&self, s: f64, acc: &mut Acc) {
        if s < 0. {
            return;
        }
        if let Some((slot, data)) = self.slots.get(&(s % self.cap).to_bits())
            && *slot == s.to_bits()
        {
            acc.merge(data);
        }
    }
}
/// A populated history bucket. Empty buckets are omitted, exposing gaps.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    /// Seconds ago of the newest edge, relative to the end of the current second.
    pub t_ago: f64,
    /// Width in seconds; adjacent populated buckets tile unless there is a gap.
    pub dur: f64,
    /// Mean of source samples, retaining subsecond sample weighting.
    pub mean: f64,
    /// Lowest source sample.
    pub min: f64,
    /// Highest source sample.
    pub max: f64,
}
/// One metric's four bounded rings. Each sample updates every level.
#[derive(Debug)]
pub struct MultiResSeries {
    levels: Vec<Level>,
}
impl Default for MultiResSeries {
    fn default() -> Self {
        Self {
            levels: HISTORY_LEVELS
                .into_iter()
                .map(|(res, cap)| Level {
                    res: res.into(),
                    cap: cap.into(),
                    slots: HashMap::new(),
                    latest: -1.,
                })
                .collect(),
        }
    }
}
impl MultiResSeries {
    /// Add a source sample in ride seconds. Missing/nonfinite values and negative
    /// times are ignored. Time must be finite and nonnegative; gaps are not filled.
    pub fn add(&mut self, t: f64, v: Option<f64>) {
        if t < 0. {
            return;
        }
        if let Some(v) = v.filter(|v| v.is_finite()) {
            for level in &mut self.levels {
                level.add(t, v);
            }
        }
    }
    /// Clear all rings for a new ride.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    fn coarse(&self, lo: f64, hi: f64, level: usize, acc: &mut Acc) {
        if hi <= lo {
            return;
        }
        let lv = &self.levels[level];
        let s = (lo / lv.res).floor();
        let mid = (s + 0.5) * lv.res;
        if mid >= lo && mid < hi {
            lv.accumulate(s, acc);
        }
    }
    fn ragged(&self, lo: f64, hi: f64, level: usize, acc: &mut Acc) {
        if hi <= lo {
            return;
        }
        let f0 = self.levels[level - 1].oldest_t();
        if lo >= f0 {
            self.agg(lo, hi, level - 1, acc);
        } else if hi <= f0 {
            self.coarse(lo, hi, level, acc);
        } else {
            self.coarse(lo, f0, level, acc);
            self.agg(f0, hi, level - 1, acc);
        }
    }
    fn agg(&self, lo: f64, hi: f64, level: usize, acc: &mut Acc) {
        if hi <= lo {
            return;
        }
        let lv = &self.levels[level];
        if level == 0 {
            let mut s = lo;
            while s < hi {
                lv.accumulate(s, acc);
                s += 1.;
            }
            return;
        }
        let a = (lo / lv.res).ceil();
        let b = (hi / lv.res).floor();
        if a < b {
            let mut s = a;
            while s < b {
                lv.accumulate(s, acc);
                s += 1.;
            }
            self.ragged(lo, a * lv.res, level, acc);
            self.ragged(b * lv.res, hi, level, acc);
        } else {
            self.ragged(lo, hi, level, acc);
        }
    }
    /// Query oldest-to-newest buckets over a positive finite span in seconds.
    /// Bucket count is clamped to 1..2000. Edges snap to whole seconds and then
    /// to the finest retained grid; narrow buckets merge. Expired ragged edges
    /// use the coarse slot when its midpoint lies inside the edge range (TS parity).
    #[must_use]
    pub fn query(&self, now: f64, span_s: f64, buckets: u32, scale: TimeScale) -> Vec<Bucket> {
        let now_edge = now.floor() + 1.;
        let mut hi = now_edge;
        let mut out = Vec::new();
        for edge in time_ago_edges(span_s, buckets.clamp(1, 2000), scale)
            .into_iter()
            .skip(1)
        {
            let mut lo = crate::js_round(now_edge - edge).max(0.);
            if let Some(cover) = self.levels.iter().find(|lv| lv.oldest_t() <= lo) {
                lo = (lo / cover.res).floor() * cover.res;
            }
            if lo >= hi {
                continue;
            }
            let width = hi - lo;
            let mut level = self
                .levels
                .iter()
                .rposition(|lv| lv.res <= width)
                .unwrap_or(0);
            while level < self.levels.len() - 1 && lo < self.levels[level].oldest_t() {
                level += 1;
            }
            let mut acc = Acc::default();
            self.agg(lo, hi, level, &mut acc);
            if acc.count > 0. {
                out.push(Bucket {
                    t_ago: now_edge - hi,
                    dur: width,
                    mean: acc.sum / acc.count,
                    min: acc.min,
                    max: acc.max,
                });
            }
            hi = lo;
            if lo <= 0. {
                break;
            }
        }
        out.reverse();
        out
    }
}
/// Independent metric histories. Metric identifiers are supplied by the caller.
#[derive(Debug, Default)]
pub struct HistoryStore {
    series: HashMap<String, MultiResSeries>,
}
impl HistoryStore {
    /// Feed a sample, creating its bounded metric rings on first use.
    pub fn add(&mut self, metric: &str, t: f64, v: Option<f64>) {
        self.series.entry(metric.into()).or_default().add(t, v);
    }
    /// Query one metric; unknown metrics return an empty result.
    #[must_use]
    pub fn query(
        &self,
        metric: &str,
        now: f64,
        span_s: f64,
        buckets: u32,
        scale: TimeScale,
    ) -> Vec<Bucket> {
        self.series
            .get(metric)
            .map_or_else(Vec::new, |s| s.query(now, span_s, buckets, scale))
    }
    /// Forget all metrics and their samples.
    pub fn reset(&mut self) {
        self.series.clear();
    }
}
