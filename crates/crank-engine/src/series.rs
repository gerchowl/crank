//! The loaded ride's display series (ADR-0001 §4.3.3): one append-only series per metric, on
//! session time, and the latest tick's values for the big numbers.
//!
//! The core owns the store; the engine appends to it and publishes its status under the
//! store's lock ([`crate::Engine::with_series`]), so a reader holding the lock sees one tick's
//! status and points. It is display data: the 1 Hz record stays the truth.
use std::sync::{Arc, Mutex};

/// The store of one ride, shared by the engine (appending) and the core (reading).
pub type SeriesStore = Arc<Mutex<Series>>;

/// One metric's points, oldest first: `t[i]`, `w[i]` and `v[i]` are point `i`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    /// Session time, s.
    pub t: Vec<f64>,
    /// Workout time, s.
    pub w: Vec<f64>,
    /// The value: W, rpm or bpm.
    pub v: Vec<u16>,
}
impl Track {
    /// Number of points.
    #[must_use]
    pub fn len(&self) -> usize {
        self.v.len()
    }
    /// No point yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.v.is_empty()
    }
    fn push(&mut self, t: f64, w: f64, v: u16) {
        self.t.push(t);
        self.w.push(w);
        self.v.push(v);
    }
    fn append(&mut self, other: &mut Self) {
        self.t.append(&mut other.t);
        self.w.append(&mut other.w);
        self.v.append(&mut other.v);
    }
}

/// The latest tick's values; each `None` while paused, lost, in a gap or not riding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Live {
    /// Power, W.
    pub power_w: Option<u16>,
    /// Cadence, rpm.
    pub cadence_rpm: Option<u16>,
    /// Heart rate, bpm.
    pub hr_bpm: Option<u16>,
}

/// A ride's series: power every tick (4 Hz), cadence and heart rate every second tick (2 Hz).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Series {
    /// Power, W.
    pub power: Track,
    /// Cadence, rpm.
    pub cadence: Track,
    /// Heart rate, bpm.
    pub hr: Track,
    /// The latest tick's values.
    pub live: Live,
}
impl Series {
    /// Take one riding tick's values at session time `t` and workout time `w`: each present,
    /// finite value appends a point (power on every tick, cadence and heart rate when `even`),
    /// and all of them become the live values.
    pub(crate) fn tick(&mut self, t: f64, w: f64, even: bool, values: [Option<f64>; 3]) {
        let [power, cadence, hr] = values.map(|v| v.and_then(display));
        self.live = Live {
            power_w: power,
            cadence_rpm: cadence,
            hr_bpm: hr,
        };
        for (track, v, due) in [
            (&mut self.power, power, true),
            (&mut self.cadence, cadence, even),
            (&mut self.hr, hr, even),
        ] {
            if let Some(v) = v.filter(|_| due) {
                track.push(t, w, v);
            }
        }
    }
    /// Move `pending`'s points onto these and take its live values.
    pub(crate) fn take(&mut self, pending: &mut Self) {
        self.power.append(&mut pending.power);
        self.cadence.append(&mut pending.cadence);
        self.hr.append(&mut pending.hr);
        self.live = pending.live;
    }
}

/// A value as a display point: `None` when it is not finite or outside `u16`.
fn display(v: f64) -> Option<u16> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked to be finite and within u16 first"
    )]
    (v.is_finite() && (0.0..=f64::from(u16::MAX)).contains(&v)).then(|| v.round() as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-0001 §4.3.9 (FM-STEADY-24): a non-finite or absent value appends no point and
    /// shows as no live value; cadence and heart rate are taken only on even ticks.
    #[test]
    fn a_non_finite_or_absent_value_appends_no_point() {
        let mut s = Series::default();
        s.tick(0.25, 0.25, false, [Some(200.0), Some(90.0), Some(140.0)]);
        s.tick(0.5, 0.5, true, [Some(f64::NAN), Some(f64::INFINITY), None]);
        s.tick(0.75, 0.75, true, [Some(-1.0), Some(91.4), Some(141.6)]);
        assert_eq!(s.power.v, [200]);
        assert_eq!(
            (s.cadence.v.as_slice(), s.cadence.t.as_slice()),
            (&[91][..], &[0.75][..])
        );
        assert_eq!(s.hr.v, [142]);
        assert_eq!(
            s.live,
            Live {
                power_w: None,
                cadence_rpm: Some(91),
                hr_bpm: Some(142)
            }
        );
    }

    /// ADR-0001 §4.3.3: the engine appends power every tick and cadence and heart rate every
    /// second tick, on session and workout time, while riding; a pause appends nothing and
    /// clears the live values.
    #[test]
    fn the_engine_appends_at_four_and_two_hertz_only_while_riding() {
        use crate::{EngineCmd, tests::rig};
        let store = SeriesStore::default();
        let mut r = rig();
        r.engine.series = Some((store.clone(), Series::default()));
        r.load(vec![]);
        r.start();
        for k in 0..8 {
            r.sample(k, 150, 90);
            r.heart(k, 120);
        }
        r.through(7);
        r.engine.publish();
        {
            let s = store.lock().unwrap();
            assert_eq!(s.power.t, [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0]);
            assert_eq!(s.power.w, s.power.t);
            assert_eq!(s.cadence.t, [0.5, 1.0, 1.5, 2.0]);
            assert_eq!((s.cadence.v[0], s.hr.v[0], s.hr.len()), (90, 120, 4));
            let live = (s.live.power_w, s.live.cadence_rpm, s.live.hr_bpm);
            assert_eq!(live, (Some(150), Some(90), Some(120)));
        }
        r.engine.command(EngineCmd::Pause).unwrap();
        for k in 8..16 {
            r.sample(k, 150, 90);
        }
        r.through(15);
        r.engine.publish();
        let s = store.lock().unwrap();
        assert_eq!((s.power.len(), s.cadence.len()), (8, 4));
        assert_eq!(s.live, Live::default());
    }
}
