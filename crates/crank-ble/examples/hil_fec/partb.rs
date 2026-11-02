//! Offline analysis of the Part B control checks (#44 checks 6, 7, 8, 9, 12; #412).
//!
//! Every result is a pure function of the `.fecap` records and the capture's outcome line, so
//! `analyse` re-derives it from a committed capture: the procedure's own timing is read back
//! from its tx writes, the rider's from page 25 (power, cadence) and page 16 (speed). Only a run whose procedure
//! completed gives a result; any other run (or a capture without an outcome line) reports
//! "no result" and no verdict.
use crate::frames::{Record, page_of};
use crank_engine::erg::policy::{REENGAGE_RPM, RELEASE_RPM};
use crank_fec::{Page, decode};
use serde_json::{Value, json};

const SEC: u64 = 1_000_000;

/// Prefix of the comment line `hil_fec` appends to a Part B capture once the link is closed.
pub const OUTCOME: &str = "# hil_fec procedure: ";
/// The outcome of a procedure that ran to its end.
pub const COMPLETED: &str = "completed";

/// The outcome line of a capture's text, if it has one.
pub fn outcome_in(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .find_map(|l| l.strip_prefix(OUTCOME))
        .map(|o| o.trim().to_string())
}

/// Append the outcome line; call it after the link is closed, when nothing else writes.
pub fn append_outcome(path: &std::path::Path, outcome: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(path)?;
    writeln!(f, "{OUTCOME}{outcome}")
}

/// One page-25 sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Arrival, µs from capture start.
    pub at_us: u64,
    /// Instantaneous cadence; `None` when the trainer reports it unavailable.
    pub cadence: Option<u8>,
    /// Instantaneous power; `None` when unavailable.
    pub power: Option<u16>,
    /// Target limit code (0 at target, 1 speed low, 2 speed high, 3 undetermined).
    pub target_limit: u8,
}

/// Page-25 samples of a capture, in arrival order.
pub fn samples(records: &[Record]) -> Vec<Sample> {
    let mut out: Vec<Sample> = records
        .iter()
        .filter(|r| r.rx)
        .filter_map(|r| match decode(&r.bytes).ok()?.page {
            Page::Trainer(t) => Some(Sample {
                at_us: r.at_us,
                cadence: t.cadence_rpm,
                power: t.power_w,
                target_limit: t.target_limit,
            }),
            _ => None,
        })
        .collect();
    out.sort_by_key(|s| s.at_us);
    out
}

/// Page-16 speeds `(at_us, m/s)`, in arrival order.
fn speeds(records: &[Record]) -> Vec<(u64, f64)> {
    let mut out: Vec<(u64, f64)> = records
        .iter()
        .filter(|r| r.rx)
        .filter_map(|r| match decode(&r.bytes).ok()?.page {
            Page::GeneralFe(g) => g.speed_mps.map(|v| (r.at_us, f64::from(v))),
            _ => None,
        })
        .collect();
    out.sort_by_key(|s| s.0);
    out
}

/// A control page the harness wrote.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    /// Page 48.
    Resistance(f64),
    /// Page 49.
    Erg(f64),
    /// Page 50 (wind coefficient kg/m, wind speed km/h, drafting).
    Wind(f64, f64, f64),
    /// Page 51 (grade %, Crr).
    Track(f64, f64),
    /// Page 55 (rider kg, bike kg).
    User(f64, f64),
}

/// Control writes `(at_us, control)` in write order (pages 48–51 and 55).
pub fn controls(records: &[Record]) -> Vec<(u64, Control)> {
    let mut out: Vec<(u64, Control)> = records
        .iter()
        .filter(|r| !r.rx)
        .filter_map(|r| {
            let c = match decode(&r.bytes).ok()?.page {
                Page::BasicResistance { pct } => Control::Resistance(f64::from(pct?)),
                Page::TargetPower { watts } => Control::Erg(f64::from(watts)),
                Page::Wind {
                    cw_kg_per_m,
                    speed_kmh,
                    drafting,
                } => Control::Wind(
                    f64::from(cw_kg_per_m?),
                    f64::from(speed_kmh?),
                    f64::from(drafting?),
                ),
                Page::Track { grade_pct, crr } => {
                    Control::Track(f64::from(grade_pct?), f64::from(crr))
                }
                Page::UserConfig {
                    rider_kg, bike_kg, ..
                } => Control::User(f64::from(rider_kg?), f64::from(bike_kg?)),
                _ => return None,
            };
            Some((r.at_us, c))
        })
        .collect();
    out.sort_by_key(|c| c.0);
    out
}

#[allow(clippy::cast_precision_loss)]
fn mean(v: impl IntoIterator<Item = f64>) -> Option<f64> {
    let (n, sum) = v
        .into_iter()
        .fold((0_usize, 0.0), |(n, s), x| (n + 1, s + x));
    (n > 0).then(|| sum / n as f64)
}

fn powers(s: &[Sample], from: u64, to: u64) -> Vec<f64> {
    s.iter()
        .filter(|x| x.at_us >= from && x.at_us < to)
        .filter_map(|x| x.power.map(f64::from))
        .collect()
}

fn cadences(s: &[Sample], from: u64, to: u64) -> Vec<f64> {
    s.iter()
        .filter(|x| x.at_us >= from && x.at_us < to)
        .filter_map(|x| x.cadence.map(f64::from))
        .collect()
}

/// Microseconds as seconds.
#[allow(clippy::cast_precision_loss)]
pub fn secs(us: u64) -> f64 {
    us as f64 / 1e6
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[allow(clippy::cast_precision_loss)]
fn share_within(p: &[f64], target: f64, tol: f64) -> Option<f64> {
    (!p.is_empty()).then(|| {
        p.iter()
            .filter(|x| (*x - target).abs() <= target * tol)
            .count() as f64
            / p.len() as f64
    })
}

/// End of the procedure: the release write (page 48 0 %), or the last record.
fn procedure_end(records: &[Record], ctl: &[(u64, Control)]) -> u64 {
    ctl.iter()
        .rev()
        .find(|(_, c)| *c == Control::Resistance(0.0))
        .filter(|(at, _)| ctl.iter().any(|(b, _)| b < at))
        .map_or_else(
            || records.iter().map(|r| r.at_us).max().unwrap_or(0),
            |(at, _)| *at,
        )
}

/// The release at the end of a capture: page 48 0 % written after the last other control
/// write, and whether a page-71 reply echoed it.
pub fn release(records: &[Record]) -> Value {
    let ctl = controls(records);
    let last = ctl.last().filter(|(_, c)| *c == Control::Resistance(0.0));
    let echoed = last.is_some_and(|(at, _)| {
        records.iter().any(|r| {
            r.rx && r.at_us >= *at
                && page_of(&r.bytes) == Some(71)
                && r.bytes[5] == 48
                && r.bytes[11] == 0
        })
    });
    json!({
        "page48_0pct_last": last.is_some(),
        "at_s": last.map(|(at, _)| round1(secs(*at))),
        "page71_echoed": echoed,
    })
}

/// A page-49 target held for at least `HOLD_MIN` with no other control write.
#[derive(Clone, Copy, Debug)]
struct Hold {
    at: u64,
    end: u64,
    watts: f64,
}

const HOLD_MIN: u64 = 10 * SEC;
/// Check 7: the start of the silent window that is still settling.
const SETTLE: u64 = 10 * SEC;

fn erg_holds(ctl: &[(u64, Control)], end: u64) -> Vec<(bool, Hold)> {
    ctl.iter()
        .enumerate()
        .filter_map(|(i, (at, c))| {
            let Control::Erg(watts) = *c else { return None };
            let next = ctl.get(i + 1).map_or(end, |n| n.0.min(end));
            Some((
                next.saturating_sub(*at) >= HOLD_MIN,
                Hold {
                    at: *at,
                    end: next,
                    watts,
                },
            ))
        })
        .collect()
}

/// Time from the write to the first sample past 90 % of the step, µs.
fn t90(s: &[Sample], from_w: f64, h: &Hold) -> Option<u64> {
    let mark = from_w + 0.9 * (h.watts - from_w);
    s.iter()
        .filter(|x| x.at_us >= h.at && x.at_us < h.end)
        .find(|x| {
            x.power.is_some_and(|p| {
                let p = f64::from(p);
                if h.watts >= from_w {
                    p >= mark
                } else {
                    p <= mark
                }
            })
        })
        .map(|x| x.at_us - h.at)
}

/// Time from `from` until power stays within `tol` of `target` for 3 s, µs.
fn settle(s: &[Sample], from: u64, end: u64, target: f64) -> Option<u64> {
    let win: Vec<&Sample> = s
        .iter()
        .filter(|x| x.at_us >= from && x.at_us < end)
        .collect();
    let inside = |x: &Sample| {
        x.power
            .is_some_and(|p| (f64::from(p) - target).abs() <= target * 0.05)
    };
    (0..win.len()).find_map(|i| {
        let start = win[i].at_us;
        let span: Vec<&&Sample> = win[i..]
            .iter()
            .take_while(|x| x.at_us < start + 3 * SEC)
            .collect();
        let covered = win[i..].iter().any(|x| x.at_us >= start + 3 * SEC);
        (covered && span.iter().all(|x| inside(x))).then(|| start - from)
    })
}

/// Cadence crossings of 75 rpm inside a hold: `(at_us, downward)`.
fn crossings(s: &[Sample], h: &Hold) -> Vec<(u64, bool)> {
    let mut out = Vec::new();
    let mut above: Option<bool> = None;
    for x in s.iter().filter(|x| x.at_us >= h.at && x.at_us < h.end) {
        let Some(c) = x.cadence else { continue };
        let now = c >= 75;
        if above.is_some_and(|a| a != now) {
            out.push((x.at_us, !now));
        }
        above = Some(now);
    }
    out
}

/// Check 6: ERG step response (t90) and cadence steps at a held target.
pub fn check6(records: &[Record]) -> Value {
    let s = samples(records);
    let ctl = controls(records);
    let holds = erg_holds(&ctl, procedure_end(records, &ctl));
    let mut steps = Vec::new();
    for pair in holds.windows(2) {
        let [(true, prev), (true, h)] = pair else {
            continue;
        };
        let t = t90(&s, prev.watts, h);
        steps.push(json!({
            "from_w": prev.watts, "to_w": h.watts, "t90_s": t.map(|t| round1(secs(t))),
            "mean_power_last_10s_w": mean(powers(&s, h.end.saturating_sub(10 * SEC), h.end)).map(round1),
            "mean_cadence_rpm": mean(cadences(&s, h.at, h.end)).map(round1),
        }));
    }
    let mut cadence_steps = Vec::new();
    for (_, h) in holds.iter().filter(|(held, h)| *held && h.watts > 0.0) {
        for (at, down) in crossings(&s, h) {
            let after = powers(&s, at, (at + 10 * SEC).min(h.end));
            let dev = after
                .iter()
                .map(|p| p - h.watts)
                .fold(0.0_f64, |m, d| if d.abs() > m.abs() { d } else { m });
            cadence_steps.push(json!({
                "target_w": h.watts,
                "direction": if down { "down" } else { "up" },
                "at_s": round1(secs(at)),
                "max_deviation_w": round1(dev),
                "settled_within_5pct_s": settle(&s, at, h.end, h.watts).map(|t| round1(secs(t))),
            }));
        }
    }
    json!({
        "method": "holds = page-49 writes kept >= 10 s; a step is a hold following a hold; t90 = write to the first page-25 sample past 90 % of the step; cadence steps = page-25 cadence crossing 75 rpm inside a hold, settled = first instant from which power stays within ±5 % for 3 s",
        "steps": steps,
        "cadence_steps": cadence_steps,
    })
}

/// Check 7: the procedure's target over the stretch from its write to the next write.
pub fn check7(records: &[Record]) -> Value {
    let s = samples(records);
    let target = f64::from(crate::rider::CHECK7_W);
    let Some(from) = controls(records)
        .iter()
        .find(|(_, c)| *c == Control::Erg(target))
        .map(|(at, _)| *at)
    else {
        return json!({"error": format!("no page-49 write of {target} W")});
    };
    let Some(to) = records
        .iter()
        .filter(|r| !r.rx && r.at_us > from)
        .map(|r| r.at_us)
        .min()
    else {
        return json!({"error": "no write after the target"});
    };
    let p = powers(&s, from + SETTLE, to);
    let last = mean(powers(&s, to.saturating_sub(10 * SEC), to));
    let within10 = share_within(&p, target, 0.10);
    let held = within10.is_some_and(|x| x >= 0.9)
        && last.is_some_and(|m| (m - target).abs() <= target * 0.05);
    json!({
        "method": "silent window = from the page-49 write of the procedure's target (the end of the soft start) to the next write of any page; its first 10 s settle; held = >= 90 % of the later page-25 samples within ±10 % and the last 10 s mean within ±5 % of the target",
        "target_w": target,
        "silent_s": round1(secs(to - from)),
        "samples": p.len(),
        "mean_w": mean(p.iter().copied()).map(round1),
        "min_w": p.iter().copied().reduce(f64::min),
        "max_w": p.iter().copied().reduce(f64::max),
        "within_5pct": share_within(&p, target, 0.05).map(|x| round1(x * 100.0)),
        "within_10pct": within10.map(|x| round1(x * 100.0)),
        "mean_last_10s_w": last.map(round1),
        "held": held,
    })
}

/// SIM state for the check-8 power model.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SimState {
    grade: f64,
    crr: f64,
    cw: f64,
    wind_kmh: f64,
    drafting: f64,
    mass: f64,
}

/// Steady-state road power at `v` m/s (ADR-0002 §7.9 SIM inputs): rolling and grade force
/// times speed plus the wind term `cw · (v + wind)² · drafting · v`.
fn model_power(st: &SimState, v: f64) -> f64 {
    let air = v + st.wind_kmh / 3.6;
    v * st.mass * 9.81 * (st.grade / 100.0 + st.crr) + st.cw * air * air.abs() * st.drafting * v
}

/// Check 8: power per SIM state against the road model at the measured speed.
pub fn check8(records: &[Record]) -> Value {
    let s = samples(records);
    let v = speeds(records);
    let ctl = controls(records);
    let end = procedure_end(records, &ctl);
    let mut st = SimState {
        grade: f64::NAN,
        crr: 0.004,
        cw: 0.51,
        wind_kmh: 0.0,
        drafting: 1.0,
        mass: f64::NAN,
    };
    let mut segments = Vec::new();
    for (i, (at, c)) in ctl.iter().enumerate() {
        match *c {
            Control::Track(g, crr) => (st.grade, st.crr) = (g, crr),
            Control::Wind(cw, w, d) => (st.cw, st.wind_kmh, st.drafting) = (cw, w, d),
            Control::User(r, b) => st.mass = r + b,
            Control::Erg(_) | Control::Resistance(_) => continue,
        }
        let next = ctl.get(i + 1).map_or(end, |n| n.0.min(end));
        if st.grade.is_nan() || st.mass.is_nan() || next.saturating_sub(*at) < HOLD_MIN {
            continue;
        }
        let from = at + 5 * SEC;
        let speed = mean(
            v.iter()
                .filter(|(t, _)| *t >= from && *t < next)
                .map(|(_, x)| *x),
        );
        let power = mean(powers(&s, from, next));
        let model = speed.map(|x| model_power(&st, x));
        segments.push(json!({
            "grade_pct": st.grade, "mass_kg": round1(st.mass), "crr": st.crr,
            "mean_power_w": power.map(round1),
            "mean_speed_kmh": speed.map(|x| round1(x * 3.6)),
            "mean_cadence_rpm": mean(cadences(&s, from, next)).map(round1),
            "model_power_w": model.map(round1),
            "measured_over_model": power.zip(model).filter(|(_, m)| *m > 0.0).map(|(p, m)| (p / m * 100.0).round() / 100.0),
        }));
    }
    json!({
        "method": "a segment is a SIM state (pages 50, 51, 55) held >= 10 s; means from 5 s after the write; model = v·m·g·(grade + Crr) + cw·(v + wind)²·drafting·v at the mean page-16 speed, m = rider + bike from page 55",
        "segments": segments,
    })
}

/// Check 9: the cued descent starts this long after the 250 W write (the 90 rpm hold).
pub const DESCENT_AFTER_US: u64 = 10 * SEC;
/// The cue: from 90 rpm, 5 rpm every 10 s.
const CUE_START_RPM: f64 = 90.0;
const CUE_RPM_PER_S: f64 = 0.5;
/// Following the cue: at most this far above it (30 s behind the cue).
const BEHIND_RPM: f64 = 15.0;
/// Following the cue: the cadence falls at most this much within [`DROP_WINDOW_US`].
const DROP_RPM: f64 = 9.0;
const DROP_WINDOW_US: u64 = 3 * SEC;
/// A stall: power below this share of the target for [`STALL_CONFIRM_US`].
const STALL_SHARE: f64 = 0.9;
const STALL_CONFIRM_US: u64 = 2 * SEC;
/// Power lags cadence by up to this much: the stall cadence is the range over it.
const LAG_US: u64 = SEC;
/// The descent ends at the first sample below this cadence while power is still held.
pub const STALL_FLOOR_RPM: u8 = 25;
/// A cadence bin is followed when this share of its samples is within ±10 % of the target.
const FOLLOWED_SHARE: f64 = 0.8;

/// Where a check-9 descent stands after the samples so far.
///
/// The check measures the trainer: down to which cadence the NEO's ERG holds 250 W while
/// the rider follows the cue. The rider follows it while the cadence never falls more than
/// 9 rpm within 3 s (3 rpm/s, six times the cue's 0.5 rpm/s, so a 5 rpm step made over a
/// second or two passes) and stays no more than 15 rpm above the cue. At 3 rpm/s a power lag
/// of up to 1 s blurs the stall cadence by at most 3 rpm, under the 5 rpm bins. A stall is
/// power below 90 % of the target for 2 s (the 2 s of the §7.4 release rule) with the cadence
/// falling no faster than that from 1 s before the drop: the trainer let go while the rider
/// kept turning. Power and cadence falling together is a rider who stopped, or one the
/// trainer overpowered; from page 25 the two look alike, so neither names a stall cadence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Descent {
    /// No decision yet.
    Going,
    /// The trainer let go: power fell at a cadence between `lo_rpm` and `hi_rpm`.
    Stalled {
        /// First sample of the power drop.
        onset_us: u64,
        /// The drop has lasted 2 s.
        at_us: u64,
        /// Lowest cadence over the second before the drop.
        lo_rpm: u8,
        /// Highest cadence over the second before the drop.
        hi_rpm: u8,
    },
    /// The cadence reached the floor with power still held.
    Floor {
        /// The first sample below the floor.
        at_us: u64,
        /// The last cadence with power held.
        held_to_rpm: u8,
    },
    /// The rider did not follow the cue: no stall can be named.
    Departed {
        /// Where it was seen.
        at_us: u64,
        /// What happened.
        why: String,
    },
}

/// Largest fall of cadence (from a running peak) over the samples `s[..=to]` at or after
/// `from_us`.
fn largest_fall(s: &[&Sample], from_us: u64, to: usize) -> f64 {
    let mut peak: Option<u8> = None;
    let mut fall = 0_u8;
    for x in s[..=to].iter().filter(|x| x.at_us >= from_us) {
        let Some(c) = x.cadence else { continue };
        peak = Some(peak.map_or(c, |p| p.max(c)));
        fall = fall.max(peak.unwrap_or(c).saturating_sub(c));
    }
    f64::from(fall)
}

/// Judge a check-9 descent that started at `from_us` against `target_w` (see [`Descent`]).
/// The live run and `analyse` call it on the same page-25 samples.
pub fn descent(samples: &[Sample], from_us: u64, target_w: f64) -> Descent {
    let s: Vec<&Sample> = samples.iter().filter(|x| x.at_us >= from_us).collect();
    let mut onset: Option<u64> = None;
    for (i, x) in s.iter().enumerate() {
        let departed = |why: String| Descent::Departed {
            at_us: x.at_us,
            why,
        };
        let (Some(c), Some(p)) = (x.cadence, x.power) else {
            return departed("cadence or power unavailable during the descent".into());
        };
        let low = f64::from(p) < STALL_SHARE * target_w;
        onset = if low { onset.or(Some(x.at_us)) } else { None };
        if let Some(o) = onset {
            if x.at_us - o < STALL_CONFIRM_US {
                continue;
            }
            let fall = largest_fall(&s, o.saturating_sub(LAG_US), i);
            if fall > DROP_RPM {
                return departed(format!(
                    "power and cadence fell together ({fall} rpm within {} s, now {c} rpm): the rider stopped or was overpowered, not a trainer stall",
                    round1(secs(x.at_us - o.saturating_sub(LAG_US)))
                ));
            }
            let before = s
                .iter()
                .filter(|y| y.at_us + LAG_US >= o && y.at_us <= o)
                .filter_map(|y| y.cadence);
            // The window holds at least the onset sample itself.
            let (lo, hi) = before.fold((u8::MAX, 0), |(lo, hi), y| (lo.min(y), hi.max(y)));
            return Descent::Stalled {
                onset_us: o,
                at_us: x.at_us,
                lo_rpm: lo.min(hi),
                hi_rpm: hi,
            };
        }
        let fall = largest_fall(&s, x.at_us.saturating_sub(DROP_WINDOW_US), i);
        if fall > DROP_RPM {
            return departed(format!(
                "cadence fell {fall} rpm within 3 s to {c} rpm (the cue is 5 rpm per 10 s): the rider stopped, not the trainer"
            ));
        }
        if c < STALL_FLOOR_RPM {
            let held_to_rpm = i.checked_sub(1).and_then(|k| s[k].cadence).unwrap_or(c);
            return Descent::Floor {
                at_us: x.at_us,
                held_to_rpm,
            };
        }
        let cue = CUE_START_RPM - CUE_RPM_PER_S * secs(x.at_us - from_us);
        if f64::from(c) > cue + BEHIND_RPM {
            return departed(format!(
                "{c} rpm where the cue is at {cue:.0} rpm: the rider did not follow the cued descent"
            ));
        }
    }
    Descent::Going
}

/// The check-9 verdict of a descent that ended in a stall or at the floor.
pub fn stall_verdict(d: &Descent) -> Option<String> {
    match *d {
        Descent::Stalled { lo_rpm, hi_rpm, .. } => Some(if lo_rpm >= RELEASE_RPM {
            "stall at or above the release cadence: the release comes too late".into()
        } else if hi_rpm < RELEASE_RPM {
            "stall below the release cadence: the release comes first".into()
        } else {
            format!(
                "ambiguous: the stall cadence spans {lo_rpm}-{hi_rpm} rpm, across the {RELEASE_RPM} rpm release (power lag)"
            )
        }),
        Descent::Floor { held_to_rpm, .. } => Some(format!(
            "no stall down to {held_to_rpm} rpm: the release comes first"
        )),
        Descent::Going | Descent::Departed { .. } => None,
    }
}

/// A cadence bin is followed: at least 2 samples, [`FOLLOWED_SHARE`] within ±10 %.
fn bin_followed(p: &[f64], target: f64) -> bool {
    p.len() >= 2 && share_within(p, target, 0.10).is_some_and(|x| x >= FOLLOWED_SHARE)
}

/// Check 9: down to which cadence ERG holds 250 W while the rider follows the cued descent.
pub fn check9(records: &[Record]) -> Value {
    let s = samples(records);
    let ctl = controls(records);
    let end = procedure_end(records, &ctl);
    let target = f64::from(crate::rider::MAX_ERG_W);
    let Some(at) = ctl
        .iter()
        .find(|(at, c)| *at < end && *c == Control::Erg(target))
        .map(|(at, _)| *at)
    else {
        return json!({"error": format!("no page-49 write of {target} W")});
    };
    let from = at + DESCENT_AFTER_US;
    let d = descent(&s, from, target);
    let until = match &d {
        Descent::Stalled { at_us, .. }
        | Descent::Floor { at_us, .. }
        | Descent::Departed { at_us, .. } => (*at_us).min(end),
        Descent::Going => end,
    };
    let mut bins: std::collections::BTreeMap<u8, Vec<f64>> = std::collections::BTreeMap::new();
    for x in s.iter().filter(|x| x.at_us >= at && x.at_us <= until) {
        if let (Some(c), Some(p)) = (x.cadence, x.power) {
            bins.entry(c / 5 * 5).or_default().push(f64::from(p));
        }
    }
    let lowest = bins
        .iter()
        .find(|(_, p)| bin_followed(p, target))
        .map(|(c, _)| *c);
    let (event, detail) = match &d {
        Descent::Going => ("not ended", json!(null)),
        Descent::Stalled {
            onset_us,
            at_us,
            lo_rpm,
            hi_rpm,
        } => (
            "stalled",
            json!({"onset_s": round1(secs(*onset_us)), "at_s": round1(secs(*at_us)), "stall_rpm": [lo_rpm, hi_rpm]}),
        ),
        Descent::Floor { at_us, held_to_rpm } => (
            "floor",
            json!({"at_s": round1(secs(*at_us)), "held_to_rpm": held_to_rpm}),
        ),
        Descent::Departed { at_us, why } => (
            "departed",
            json!({"at_s": round1(secs(*at_us)), "why": why}),
        ),
    };
    json!({
        "method": "descent from 10 s after the procedure's 250 W write; the cue falls from 90 rpm by 0.5 rpm/s; followed = cadence never falls > 9 rpm within 3 s and stays <= 15 rpm above the cue; stall = power < 90 % of the target for 2 s with the cadence followed from 1 s before the drop, stall cadence = the range over that second; floor = first sample below 25 rpm with power held; anything else departs (no verdict). Bins: 5 rpm, followed = >= 2 samples and >= 80 % within ±10 %",
        "target_w": target,
        "descent_from_s": round1(secs(from)),
        "descent": event,
        "descent_detail": detail,
        "bins": bins.iter().map(|(c, p)| json!({
            "cadence_rpm": c, "samples": p.len(), "mean_power_w": mean(p.iter().copied()).map(round1),
            "within_10pct": share_within(p, target, 0.10).map(|x| round1(x * 100.0)),
        })).collect::<Vec<_>>(),
        "lowest_followed_rpm": lowest,
        "release_rpm": RELEASE_RPM,
        "reengage_rpm": REENGAGE_RPM,
        "verdict": stall_verdict(&d),
    })
}

/// Check 12: does `Free` (page 50 defaults, page 51 grade 0) release an ERG target?
pub fn check12(records: &[Record]) -> Value {
    let s = samples(records);
    let ctl = controls(records);
    let end = procedure_end(records, &ctl);
    let Some((erg_i, target)) = ctl
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, (_, c))| match c {
            Control::Erg(w) => Some((i, *w)),
            _ => None,
        })
    else {
        return json!({"error": "no page-49 target"});
    };
    let after = &ctl[erg_i + 1..];
    let wind = after.iter().find(|(_, c)| matches!(c, Control::Wind(..)));
    let track = after
        .iter()
        .find(|(_, c)| matches!(c, Control::Track(g, _) if g.abs() < 1e-9));
    let (Some(wind), Some(track)) = (wind, track) else {
        return json!({"error": "no page 50 + page 51 grade 0 after the ERG target"});
    };
    let free_at = wind.0.max(track.0);
    let low: Vec<&Sample> = s
        .iter()
        .filter(|x| x.at_us > free_at && x.at_us < end && x.cadence.is_some_and(|c| c < 80))
        .collect();
    let low_power = mean(low.iter().filter_map(|x| x.power.map(f64::from)));
    // A page-71 reply after the write naming that page and echoing its bytes 4-7.
    let echoed = |page: u8, at: u64| {
        let sent = records
            .iter()
            .find(|r| !r.rx && r.at_us == at && page_of(&r.bytes) == Some(page));
        sent.is_some_and(|w| {
            records.iter().any(|r| {
                r.rx && r.at_us >= at
                    && page_of(&r.bytes) == Some(71)
                    && r.bytes[5] == page
                    && r.bytes[8..12] == w.bytes[8..12]
            })
        })
    };
    json!({
        "method": "Free = page 50 then page 51 grade 0 after a page-49 target; released = mean power of the samples below 80 rpm after Free < 90 % of the ERG target (ERG would hold it); echoed = a later page-71 reply names the page and echoes its bytes 4-7",
        "erg_target_w": target,
        "free_at_s": round1(secs(free_at)),
        "mean_power_before_free_5s_w": mean(powers(&s, free_at.saturating_sub(5 * SEC), free_at)).map(round1),
        "low_cadence_samples": low.len(),
        "low_cadence_mean_power_w": low_power.map(round1),
        "low_cadence_mean_rpm": mean(low.iter().filter_map(|x| x.cadence.map(f64::from))).map(round1),
        "page71_echoed_50": echoed(50, wind.0),
        "page71_echoed_51": echoed(51, track.0),
        "released": low_power.map(|p| p < 0.9 * target),
    })
}

/// Part B analysis by capture stem (`check6` …), `None` for other captures. `outcome` is how
/// the procedure ended (the capture's outcome line); without `completed` there is no verdict.
pub fn analyse(stem: &str, records: &[Record], outcome: Option<&str>) -> Option<Value> {
    let mut v = match stem {
        "check6" => check6(records),
        "check7" => check7(records),
        "check8" => check8(records),
        "check9" => check9(records),
        "check12" => check12(records),
        _ => return None,
    };
    let completed = outcome == Some(COMPLETED);
    v["procedure"] = json!(outcome.unwrap_or("unknown: the capture has no outcome line"));
    v["completed"] = json!(completed);
    if !completed {
        for verdict in ["held", "released", "verdict"] {
            if v.get(verdict).is_some() {
                v[verdict] = Value::Null;
            }
        }
    }
    v["release"] = release(records);
    v["line"] = json!(line(stem, &v));
    Some(v)
}

fn num(v: &Value) -> String {
    v.as_f64().map_or_else(|| "–".into(), |x| format!("{x}"))
}

/// The result row for `docs/hil/<date>-fec.md` (`| # | Check | Result | ADR-0002 |`).
pub fn line(stem: &str, v: &Value) -> String {
    let (n, what, result, adr) = cells(stem, v);
    let result = if v["completed"] == json!(true) {
        result
    } else {
        format!(
            "no result: procedure {}",
            v["procedure"].as_str().unwrap_or("unknown")
        )
    };
    let release = if v["release"]["page48_0pct_last"] == json!(true) {
        "released"
    } else {
        "NOT released"
    };
    format!("| {n} | {what} | {result} (trainer {release} at the end) | {adr} |")
}

/// Check number, check, result and ADR-0002 section of a completed run's row.
fn cells(stem: &str, v: &Value) -> (&'static str, &'static str, String, &'static str) {
    let empty = Vec::new();
    let list = |key: &str| v[key].as_array().unwrap_or(&empty).clone();
    match stem {
        "check6" => {
            let steps = list("steps")
                .iter()
                .map(|s| {
                    format!(
                        "{}→{} W t90 {} s",
                        num(&s["from_w"]),
                        num(&s["to_w"]),
                        num(&s["t90_s"])
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let cad = list("cadence_steps")
                .iter()
                .map(|s| {
                    format!(
                        "cadence {} at {} W: max {} W, settled {} s",
                        s["direction"].as_str().unwrap_or("?"),
                        num(&s["target_w"]),
                        num(&s["max_deviation_w"]),
                        num(&s["settled_within_5pct_s"])
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            (
                "6",
                "ERG steps 100→150→250→100 W; cadence 90→60→90 rpm at 200 W",
                format!("{steps}; {cad}"),
                "§7.2",
            )
        }
        "check7" => (
            "7",
            "Target without keep-alive",
            format!(
                "held: {}; {} W over {} s silent, {} % within ±5 %, last 10 s mean {} W",
                v["held"],
                num(&v["target_w"]),
                num(&v["silent_s"]),
                num(&v["within_5pct"]),
                num(&v["mean_last_10s_w"])
            ),
            "§4.4",
        ),
        "check8" => {
            let segs = list("segments")
                .iter()
                .map(|s| {
                    format!(
                        "{} % at {} kg: {} W at {} km/h (model {} W, ×{})",
                        num(&s["grade_pct"]),
                        num(&s["mass_kg"]),
                        num(&s["mean_power_w"]),
                        num(&s["mean_speed_kmh"]),
                        num(&s["model_power_w"]),
                        num(&s["measured_over_model"])
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            ("8", "SIM page 51 at 0/5/10 %; page 55 mass", segs, "§7.9")
        }
        "check9" => (
            "9",
            "Low cadence at 250 W ERG",
            format!(
                "descent {}; 250 W followed down to the {} rpm bin; {}",
                v["descent"].as_str().unwrap_or("?"),
                num(&v["lowest_followed_rpm"]),
                v["verdict"].as_str().unwrap_or("no verdict")
            ),
            "§7.4",
        ),
        _ => (
            "12",
            "ERG 200 W → Free",
            format!(
                "released: {}; below 80 rpm in Free {} W at {} rpm; page 71 echoed 50/51: {}/{}",
                v["released"],
                num(&v["low_cadence_mean_power_w"]),
                num(&v["low_cadence_mean_rpm"]),
                v["page71_echoed_50"],
                v["page71_echoed_51"]
            ),
            "§7.9",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crank_fec::{ControlPage, encode};

    fn tx(at_us: u64, page: &ControlPage) -> Record {
        Record {
            at_us,
            rx: false,
            bytes: encode(page).expect("encode").to_vec(),
        }
    }

    fn p25(at_us: u64, cadence: u8, watts: u16) -> Record {
        let w = watts.to_le_bytes();
        let mut f = [0xa4, 9, 0x4e, 5, 25, 0, cadence, 0, 0, w[0], w[1], 0x30, 0];
        f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
        Record {
            at_us,
            rx: true,
            bytes: f.to_vec(),
        }
    }

    #[test]
    fn a_bin_is_followed_from_80_percent_within_10_percent() {
        let bin = |inside: usize, outside: usize| {
            let mut p = vec![250.0; inside];
            p.extend(vec![200.0; outside]);
            bin_followed(&p, 250.0)
        };
        assert!(bin(8, 2));
        assert!(!bin(7, 3));
        assert!(!bin(1, 0), "one sample is not a bin");
    }

    #[test]
    fn a_cadence_step_inside_the_lag_window_makes_the_stall_ambiguous() {
        // 42 rpm, a 4 rpm step to 38 at 5 s, power drops at 5.5 s: within the 1 s lag the
        // stall cadence is anywhere from 38 to 42, across the 40 rpm release.
        let s: Vec<Sample> = (0..40)
            .map(|k| {
                let at_us = k * 250_000;
                Sample {
                    at_us,
                    cadence: Some(if at_us < 5 * SEC { 42 } else { 38 }),
                    power: Some(if at_us < 5_500_000 { 250 } else { 200 }),
                    target_limit: 0,
                }
            })
            .collect();
        let d = descent(&s, 0, 250.0);
        assert_eq!(
            d,
            Descent::Stalled {
                onset_us: 5_500_000,
                at_us: 7_500_000,
                lo_rpm: 38,
                hi_rpm: 42
            }
        );
        assert!(stall_verdict(&d).expect("verdict").starts_with("ambiguous"));
    }

    #[test]
    fn check9_ignores_a_250_w_write_after_the_release() {
        // The procedure ended (page 48 0 % after its 0 W write) before any 250 W write.
        let mut records = vec![
            tx(0, &ControlPage::BasicResistance { pct: 0.0 }),
            tx(SEC, &ControlPage::TargetPower { watts: 0.0 }),
            tx(2 * SEC, &ControlPage::BasicResistance { pct: 0.0 }),
            tx(3 * SEC, &ControlPage::TargetPower { watts: 250.0 }),
        ];
        records.extend((0..200).map(|k| p25(3 * SEC + k * 350_000, 90, 250)));
        assert_eq!(
            check9(&records)["error"],
            json!("no page-49 write of 250 W"),
            "{}",
            check9(&records)
        );
    }
}
