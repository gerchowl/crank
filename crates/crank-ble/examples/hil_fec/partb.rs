//! Offline analysis of the Part B control checks (#44 checks 6, 7, 8, 9, 12; #412).
//!
//! Every result is a pure function of the `.fecap` records and the capture's outcome line, so
//! `analyse` re-derives it from a committed capture: the procedure's own timing is read back
//! from its tx writes, the rider's from page 25 (power, cadence) and page 16 (speed). Only a run whose procedure
//! completed gives a result; any other run (or a capture without an outcome line) reports
//! "no result" and no verdict.
use crate::{
    frames::{Record, command_status, page_of},
    timeline::{Step, Timeline},
};
use crank_fec::{Page, decode};
use serde_json::{Value, json};

pub(crate) const SEC: u64 = 1_000_000;
/// The ADR-0002 §7.4 release cadence Part B was designed and recorded against (40 rpm), from
/// before #589 made the release rider tuning with gentler defaults. Part B reads the ride's
/// recorded tuning instead of these copies in #593.
pub const RELEASE_RPM: u8 = 40;
/// The re-engage cadence of the same values (60 rpm); #593 as [`RELEASE_RPM`].
pub const REENGAGE_RPM: u8 = 60;
/// The release dwell of the same values: 8 ticks (2 s); #593 as [`RELEASE_RPM`].
pub const RELEASE_TICKS: u64 = 8;

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

/// Prefix of the comment line that follows the outcome line: when the run ended, Unix seconds.
/// It orders the runs of a riding session ([`SESSION_BOUND_S`]).
pub const FINISHED: &str = "# hil_fec finished: ";

/// When the capture's run ended, Unix seconds, if it says.
pub fn finished_in(text: &str) -> Option<u64> {
    text.lines()
        .rev()
        .find_map(|l| l.strip_prefix(FINISHED))
        .and_then(|t| t.trim().parse().ok())
}

/// Append the outcome line and the finished stamp; call it after the link is closed, when
/// nothing else writes.
pub fn append_outcome(
    path: &std::path::Path,
    outcome: &str,
    finished_unix_s: u64,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(path)?;
    writeln!(f, "{OUTCOME}{outcome}")?;
    writeln!(f, "{FINISHED}{finished_unix_s}")
}

/// The trainer serial of a capture: the page-81 serial of its timeline. `Err`: why there is
/// none to go by (no page 81, a redacted `FF FF FF FF`, or frames that disagree).
pub fn serial_in(tl: &Timeline) -> Result<u32, String> {
    let mut serials = tl
        .serials()
        .iter()
        .map(|(_, s)| u32::from_le_bytes(*s))
        .collect::<Vec<_>>();
    serials.sort_unstable();
    serials.dedup();
    match serials[..] {
        [] => Err("no page-81 serial".into()),
        [UNSPECIFIED_SERIAL] => Err("the page-81 serial is unspecified (redacted)".into()),
        [serial] => Ok(serial),
        _ => Err("page-81 serials disagree".into()),
    }
}

/// Page 81's "no serial" value, which `redact` writes over the real one.
const UNSPECIFIED_SERIAL: u32 = 0xffff_ffff;

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
    /// FE state code.
    pub state: u8,
    /// Trainer status bits.
    pub status: u8,
    /// Its index among the frames it was read from (a [`Timeline`]'s positions).
    pub pos: usize,
    /// The update event count (page byte 1) did not change from the page 25 before it: the
    /// trainer repeated its last update (in the committed live captures always byte for byte,
    /// ADR-0007 §1), so it is no new sample of power or cadence.
    pub repeat: bool,
}

/// Page-25 samples of frames in time order (a [`Timeline`]'s, or the live pump's), in that order.
pub fn samples(records: &[Record]) -> Vec<Sample> {
    let mut last: Option<u8> = None;
    records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.rx)
        .filter_map(|(pos, r)| match decode(&r.bytes).ok()?.page {
            Page::Trainer(t) => {
                let repeat = last == Some(t.event_count);
                last = Some(t.event_count);
                Some(Sample {
                    at_us: r.at_us,
                    cadence: t.cadence_rpm,
                    power: t.power_w,
                    target_limit: t.target_limit,
                    state: t.state,
                    status: t.status,
                    pos,
                    repeat,
                })
            }
            _ => None,
        })
        .collect()
}

/// One page-16 (General FE Data) frame: its speed and its elapsed time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speed {
    /// Arrival, µs from capture start.
    pub at_us: u64,
    /// Its index among the frames.
    pub pos: usize,
    /// Speed, m/s; `None` when the trainer reports it unavailable.
    pub mps: Option<f64>,
    /// Elapsed time, page byte 2 (frame byte 6): 0.25 s units, rollover 64 s; held, the frame
    /// repeats the one before (no fresh speed).
    pub elapsed: u8,
}

/// Every page-16 frame among `records`, in time order.
pub(crate) fn speeds(records: &[Record]) -> Vec<Speed> {
    records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.rx)
        .filter_map(|(pos, r)| match decode(&r.bytes).ok()?.page {
            Page::GeneralFe(g) => Some(Speed {
                at_us: r.at_us,
                pos,
                mps: g.speed_mps.map(f64::from),
                elapsed: r.bytes[6],
            }),
            _ => None,
        })
        .collect()
}

/// One page-70 request the harness wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// Its index among the frames.
    pub pos: usize,
    /// When it was written, µs from capture start.
    pub at_us: u64,
    /// The page it requests.
    pub page: u8,
}

/// The page-70 requests among `records`.
pub(crate) fn requests(records: &[Record]) -> Vec<Request> {
    records
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.rx && page_of(&r.bytes) == Some(70))
        .map(|(pos, r)| Request {
            pos,
            at_us: r.at_us,
            page: r.bytes[10],
        })
        .collect()
}

/// One page-71 command status the trainer sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    /// Its index among the frames.
    pub pos: usize,
    /// Arrival, µs from capture start.
    pub at_us: u64,
    /// The last control page it names.
    pub page: u8,
    /// Its bytes 4–7, the echo of that page's.
    pub echo: [u8; 4],
}

/// The page-71 command statuses among `records`.
pub(crate) fn statuses(records: &[Record]) -> Vec<Status> {
    records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.rx)
        .filter_map(|(pos, r)| {
            let (page, _, _, echo) = command_status(&r.bytes)?;
            Some(Status {
                pos,
                at_us: r.at_us,
                page,
                echo,
            })
        })
        .collect()
}

/// The page-81 serials (bytes 8–11) the trainer sent, `(at_us, serial)`.
pub fn serials(records: &[Record]) -> Vec<(u64, [u8; 4])> {
    records
        .iter()
        .filter(|r| r.rx && page_of(&r.bytes) == Some(0x51))
        .filter_map(|r| Some((r.at_us, r.bytes.get(8..12)?.try_into().ok()?)))
        .collect()
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
    /// A control page (this number) with a field that does not decode, or a bad checksum: still
    /// a write the trainer may act on, never the release.
    Malformed(u8),
    /// A tx record that is not a 13-byte ANT frame (sync 0xA4): what the trainer makes of it is
    /// unknown, so it counts as a malformed control write too.
    NotAFrame,
}

/// One control write of a timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Write {
    /// Its index among the frames (a [`Timeline`]'s positions).
    pub pos: usize,
    /// When it was written, µs from capture start.
    pub at_us: u64,
    /// What it writes.
    pub control: Control,
    /// Its page bytes 4–7, what a page-71 reply echoes.
    pub data: [u8; 4],
}

/// Control writes in write order: every tx page 48–51 and 55, whatever its fields
/// ([`Control::Malformed`]), and every tx record that is no ANT frame ([`Control::NotAFrame`]).
pub fn writes(records: &[Record]) -> Vec<Write> {
    records
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.rx)
        .filter_map(|(pos, r)| {
            let control = match page_of(&r.bytes) {
                None => Control::NotAFrame,
                Some(page @ (48..=51 | 55)) => {
                    control(&r.bytes).unwrap_or(Control::Malformed(page))
                }
                Some(_) => return None,
            };
            Some(Write {
                pos,
                at_us: r.at_us,
                control,
                data: r
                    .bytes
                    .get(8..12)
                    .and_then(|d| d.try_into().ok())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// [`writes`] as `(at_us, control)`, for the checks that read durations between writes.
#[cfg(test)]
pub fn controls(records: &[Record]) -> Vec<(u64, Control)> {
    writes(records)
        .into_iter()
        .map(|w| (w.at_us, w.control))
        .collect()
}

/// The control a frame writes, when every field decodes.
fn control(bytes: &[u8]) -> Option<Control> {
    Some(match decode(bytes).ok()?.page {
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
        Page::Track { grade_pct, crr } => Control::Track(f64::from(grade_pct?), f64::from(crr)),
        Page::UserConfig {
            rider_kg, bike_kg, ..
        } => Control::User(f64::from(rider_kg?), f64::from(bike_kg?)),
        _ => return None,
    })
}

#[allow(clippy::cast_precision_loss)]
pub(crate) fn mean(v: impl IntoIterator<Item = f64>) -> Option<f64> {
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

pub(crate) fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[allow(clippy::cast_precision_loss)]
pub(crate) fn share_within(p: &[f64], target: f64, tol: f64) -> Option<f64> {
    (!p.is_empty()).then(|| {
        p.iter()
            .filter(|x| (*x - target).abs() <= target * tol)
            .count() as f64
            / p.len() as f64
    })
}

/// End of the procedure: the release write (page 48 0 %), or the capture's last frame, at
/// `last_us`.
pub(crate) fn procedure_end(last_us: u64, ctl: &[(u64, Control)]) -> u64 {
    released_at(ctl).unwrap_or(last_us)
}

/// The run's closing release: the last page 48 at 0 %, after some other write. `None` in a
/// capture cut off before it.
pub(crate) fn released_at(ctl: &[(u64, Control)]) -> Option<u64> {
    ctl.iter()
        .rev()
        .find(|(_, c)| *c == Control::Resistance(0.0))
        .filter(|(at, _)| ctl.iter().any(|(b, _)| b < at))
        .map(|(at, _)| *at)
}

/// The release at the end of a timeline: page 48 0 % written after the last other control write,
/// and whether a later page-71 reply echoed it.
pub fn release(ws: &[Write], st: &[Status]) -> Value {
    let last = ws
        .last()
        .filter(|w| w.control == Control::Resistance(0.0))
        .copied();
    let echoed = last.is_some_and(|w| {
        st.iter()
            .any(|s| s.at_us >= w.at_us && s.page == 48 && s.echo[3] == 0)
    });
    json!({
        "page48_0pct_last": last.is_some(),
        "at_s": last.map(|w| round1(secs(w.at_us))),
        "page71_echoed": echoed,
    })
}

/// A page-49 target held for at least `HOLD_MIN` with no other control write.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hold {
    pub(crate) at: u64,
    pub(crate) end: u64,
    pub(crate) watts: f64,
}

const HOLD_MIN: u64 = 10 * SEC;
/// Check 7: the start of the silent window that is still settling.
pub(crate) const SETTLE: u64 = 10 * SEC;

pub(crate) fn erg_holds(ctl: &[(u64, Control)], end: u64) -> Vec<(bool, Hold)> {
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

/// The ERG steps of a check-6 capture (a hold following a hold): `(from_w, hold, t90)`.
fn erg_steps(tl: &Timeline, s: &[Sample]) -> Vec<(f64, Hold, Option<u64>)> {
    let ctl = tl.controls();
    erg_holds(&ctl, procedure_end(tl.end_us(), &ctl))
        .windows(2)
        .filter_map(|pair| match pair {
            [(true, prev), (true, h)] => Some((prev.watts, *h, t90(s, prev.watts, h))),
            _ => None,
        })
        .collect()
}

/// Check 6: ERG step response (t90) and cadence steps at a held target.
pub fn check6(tl: &Timeline) -> Value {
    let s = tl.samples();
    let ctl = tl.controls();
    let holds = erg_holds(&ctl, procedure_end(tl.end_us(), &ctl));
    let mut steps = Vec::new();
    for (from_w, h, t) in erg_steps(tl, s) {
        steps.push(json!({
            "from_w": from_w, "to_w": h.watts, "t90_s": t.map(|t| round1(secs(t))),
            "mean_power_last_10s_w": mean(powers(s, h.end.saturating_sub(10 * SEC), h.end)).map(round1),
            "mean_cadence_rpm": mean(cadences(s, h.at, h.end)).map(round1),
        }));
    }
    let mut cadence_steps = Vec::new();
    for (_, h) in holds.iter().filter(|(held, h)| *held && h.watts > 0.0) {
        for (at, down) in crossings(s, h) {
            let after = powers(s, at, (at + 10 * SEC).min(h.end));
            let dev = after
                .iter()
                .map(|p| p - h.watts)
                .fold(0.0_f64, |m, d| if d.abs() > m.abs() { d } else { m });
            cadence_steps.push(json!({
                "target_w": h.watts,
                "direction": if down { "down" } else { "up" },
                "at_s": round1(secs(at)),
                "max_deviation_w": round1(dev),
                "settled_within_5pct_s": settle(s, at, h.end, h.watts).map(|t| round1(secs(t))),
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
pub fn check7(tl: &Timeline) -> Value {
    let s = tl.updates();
    let target = f64::from(crate::rider::CHECK7_W);
    let Some(write) = tl
        .writes()
        .iter()
        .find(|w| w.control == Control::Erg(target))
        .copied()
    else {
        return json!({"error": format!("no page-49 write of {target} W")});
    };
    let from = write.at_us;
    // The next write of any page after it in the timeline (a control write or a page-70
    // request), one of the same µs included.
    let next_write = tl
        .writes()
        .iter()
        .find(|w| w.pos > write.pos)
        .map(|w| (w.pos, w.at_us));
    let next_request = tl
        .requests()
        .iter()
        .find(|q| q.pos > write.pos)
        .map(|q| (q.pos, q.at_us));
    let Some((_, to)) = next_write.into_iter().chain(next_request).min() else {
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

/// Check 8 needs this many fresh page-16 speeds in each SIM segment (the NEO sends page 16 about
/// once a second; a segment's window is at least 5 s).
const MIN_FRESH_SPEEDS: usize = 3;

/// Check 8: power per SIM state against the road model at the measured speed.
pub fn check8(tl: &Timeline) -> Value {
    let s = tl.samples();
    let v = tl.speeds();
    let ctl = tl.controls();
    let end = procedure_end(tl.end_us(), &ctl);
    let mut st = SimState {
        grade: f64::NAN,
        crr: 0.004,
        cw: 0.51,
        wind_kmh: 0.0,
        drafting: 1.0,
        mass: f64::NAN,
    };
    let mut segments = Vec::new();
    let mut no_speed: Option<&str> = None;
    for (i, (at, c)) in ctl.iter().enumerate() {
        match *c {
            Control::Track(g, crr) => (st.grade, st.crr) = (g, crr),
            Control::Wind(cw, w, d) => (st.cw, st.wind_kmh, st.drafting) = (cw, w, d),
            Control::User(r, b) => st.mass = r + b,
            Control::Erg(_)
            | Control::Resistance(_)
            | Control::Malformed(_)
            | Control::NotAFrame => {
                continue;
            }
        }
        let next = ctl.get(i + 1).map_or(end, |n| n.0.min(end));
        if st.grade.is_nan() || st.mass.is_nan() || next.saturating_sub(*at) < HOLD_MIN {
            continue;
        }
        let from = at + 5 * SEC;
        let fresh: Vec<f64> = v
            .iter()
            .filter(|(t, _)| *t >= from && *t < next)
            .map(|(_, x)| *x)
            .collect();
        // Fail closed: speeds only from page 16 whose elapsed time advanced; a segment without enough
        // of them gives no result, never a speed from repeats.
        if fresh.len() < MIN_FRESH_SPEEDS {
            no_speed = Some(
                if tl
                    .page16()
                    .iter()
                    .any(|(p, _)| p.at_us >= from && p.at_us < next)
                {
                    "page 16 counters did not advance (no fresh speed samples)"
                } else {
                    "no page 16 in a SIM segment"
                },
            );
        }
        let speed = mean(fresh);
        let power = mean(powers(s, from, next));
        let model = speed.map(|x| model_power(&st, x));
        segments.push(json!({
            "grade_pct": st.grade, "mass_kg": round1(st.mass), "crr": st.crr,
            "mean_power_w": power.map(round1),
            "mean_speed_kmh": speed.map(|x| round1(x * 3.6)),
            "mean_cadence_rpm": mean(cadences(s, from, next)).map(round1),
            "model_power_w": model.map(round1),
            "measured_over_model": power.zip(model).filter(|(_, m)| *m > 0.0).map(|(p, m)| (p / m * 100.0).round() / 100.0),
        }));
    }
    let mut out = json!({
        "method": "a segment is a SIM state (pages 50, 51, 55) held >= 10 s; means from 5 s after the write; model = v·m·g·(grade + Crr) + cw·(v + wind)²·drafting·v at the mean page-16 speed (fresh page 16 only: elapsed-time counter advanced; at least 3 per segment, else no result), m = rider + bike from page 55",
        "segments": segments,
    });
    if let Some(why) = no_speed {
        out["error"] = json!(why);
    }
    out
}

/// Check 9: the cued descent starts this long after the 250 W write (the 90 rpm hold).
pub const DESCENT_AFTER_US: u64 = 10 * SEC;
/// The cue: from 90 rpm, 5 rpm every 10 s.
const CUE_START_RPM: f64 = 90.0;
const CUE_RPM_PER_S: f64 = 0.5;
/// Following the cue: at most this far above it (30 s behind the cue) or below it.
const BEHIND_RPM: f64 = 15.0;
const AHEAD_RPM: f64 = 15.0;
/// Following the cue: the cadence falls at most this much within [`DROP_WINDOW_US`].
const DROP_RPM: f64 = 9.0;
const DROP_WINDOW_US: u64 = 3 * SEC;
/// A stall: power below this share of the target for [`STALL_CONFIRM_US`].
const STALL_SHARE: f64 = 0.9;
const STALL_CONFIRM_US: u64 = 2 * SEC;
/// The stated bound on how page-25 power lags the trainer: a first-order lag with a time
/// constant of at most 1 s (`t90` at most 2.3 s, which check 6 measures on the trainer).
const POWER_TAU_US: u64 = SEC;
/// Power held this long (5 time constants) is settled: within e^-5 (< 1 %) of a step.
const SETTLE_US: u64 = 5 * POWER_TAU_US;
/// Samples further apart than this do not show a settled hold.
const MAX_GAP_US: u64 = 500_000;
/// Cadence uncertainty of a sample: 0.5 rpm of rounding plus 1.5 rpm of movement within
/// [`MAX_GAP_US`] at the 3 rpm/s a following rider may reach, rounded up.
const MARGIN_RPM: u8 = 2;
/// The descent ends at the first sample below this cadence while power is still held.
pub const STALL_FLOOR_RPM: u8 = 25;
/// Check 6 shows the power-lag bound when every ERG step reaches 90 % within this: a
/// first-order lag of time constant [`POWER_TAU_US`] does after ln 10 = 2.303 of them, and
/// any delay before the response only adds to t90, so it can only fail a trainer.
pub const MAX_T90_US: u64 = 2_300_000;

/// Check 6 and check 9 are of one riding session when check 6 ended at most this long before
/// check 9 (2 h): long enough for the runbook's checks and rests in between, short enough that
/// it is the same rider on the same setup rather than another day's.
pub const SESSION_BOUND_S: u64 = 2 * 60 * 60;

/// The trainer's power response as check 6 of the same capture set measured it. Check 9's
/// "the release comes first" holds only within [`MAX_T90_US`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerLag {
    /// No t90 to go by: why.
    NotMeasured(String),
    /// Every step within [`MAX_T90_US`]; the slowest t90, µs.
    Within(u64),
    /// A step slower than [`MAX_T90_US`]: the slowest t90, µs (`None`: never reached 90 %).
    TooSlow(Option<u64>),
}

impl PowerLag {
    /// From the step t90s of a completed check 6.
    fn of_steps(t90s: &[Option<u64>]) -> Self {
        if t90s.is_empty() {
            return Self::NotMeasured("check 6 has no ERG step".into());
        }
        match t90s.iter().copied().max_by_key(|t| t.unwrap_or(u64::MAX)) {
            Some(Some(t)) if t <= MAX_T90_US => Self::Within(t),
            Some(t) => Self::TooSlow(t),
            None => unreachable!("not empty"),
        }
    }

    /// From the text of a check-6 `.fecap` (`None`: there is none).
    pub fn of_check6(text: Option<&str>) -> Self {
        let Some(text) = text else {
            return Self::NotMeasured("no check6.fecap in the capture set".into());
        };
        match outcome_in(text) {
            Some(o) if o == COMPLETED => {
                let steps = Timeline::parse(text).and_then(|tl| {
                    malformed(tl.writes())
                        .or_else(|| cut_off(&tl))
                        .map_or(Ok(()), Err)?;
                    tl.program(PROGRAM_6)?;
                    Ok(erg_steps(&tl, tl.samples()))
                });
                match steps {
                    Ok(steps) => {
                        let t90s: Vec<Option<u64>> = steps.iter().map(|(_, _, t)| *t).collect();
                        Self::of_steps(&t90s)
                    }
                    // No timeline, not the procedure's program or steps: no t90 (ADR-0007 §4).
                    Err(why) => Self::NotMeasured(format!("check 6: {why}")),
                }
            }
            o => Self::NotMeasured(format!(
                "check 6 did not complete ({})",
                o.unwrap_or_else(|| "no outcome line".into())
            )),
        }
    }

    /// Check 6 of the same trainer and session as `capture` (its text `text`), read from
    /// `check6.fecap` next to it (the same `--out` directory). A check 6 that cannot be shown to
    /// be this trainer's, from this session, is [`Self::NotMeasured`], never a measurement:
    /// "the release comes first" rests on it.
    pub fn beside(capture: &std::path::Path, text: &str) -> Self {
        match std::fs::read_to_string(capture.with_file_name("check6.fecap")) {
            Ok(check6) => Self::of_session(text, Some(&check6)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::of_session(text, None),
            Err(e) => Self::NotMeasured(format!("check6.fecap: {e}")),
        }
    }

    /// [`Self::of_check6`] of `check6`, if it is of the trainer that `check9` (a check-9 capture
    /// text) is of, and ended no more than [`SESSION_BOUND_S`] before it. Both captures must
    /// carry the same page-81 serial and a finished stamp ([`FINISHED`]).
    pub fn of_session(check9: &str, check6: Option<&str>) -> Self {
        let lag = Self::of_check6(check6);
        let Some(check6) = check6.filter(|_| matches!(lag, Self::Within(_) | Self::TooSlow(_)))
        else {
            return lag;
        };
        let not = |why: String| Self::NotMeasured(why);
        let serial = |name: &str, text: &str| {
            Timeline::parse(text)
                .and_then(|tl| serial_in(&tl))
                .map_err(|e| not(format!("{name}: {e}")))
        };
        let (s9, s6) = match (serial("check 9", check9), serial("check 6", check6)) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        if s9 != s6 {
            return not("check 6 is of a different trainer (page-81 serials differ)".into());
        }
        let (Some(f9), Some(f6)) = (finished_in(check9), finished_in(check6)) else {
            return not(
                "check 6 or check 9 has no finished stamp, so the session is unknown".into(),
            );
        };
        match f9.checked_sub(f6) {
            None => not("check 6 ended after check 9; run check 6 first".into()),
            Some(gap) if gap > SESSION_BOUND_S => not(format!(
                "check 6 ended {} min before check 9, outside the {} min session bound",
                gap / 60,
                SESSION_BOUND_S / 60
            )),
            Some(_) => lag,
        }
    }

    fn t90(&self) -> String {
        match self {
            Self::Within(t) | Self::TooSlow(Some(t)) => format!("{:.2} s", secs(*t)),
            Self::TooSlow(None) => "not reached".into(),
            Self::NotMeasured(_) => "not measured".into(),
        }
    }

    fn json(&self) -> Value {
        json!({
            "check6_t90": self.t90(),
            "max_t90_s": secs(MAX_T90_US),
            "within": match self {
                Self::Within(_) => json!(true),
                Self::TooSlow(_) => json!(false),
                Self::NotMeasured(why) => json!(why),
            },
        })
    }
}
/// A cadence bin is followed when this share of its samples is within ±10 % of the target.
const FOLLOWED_SHARE: f64 = 0.8;

/// Where a check-9 descent stands after the samples so far.
///
/// The rider follows the cue while the cadence never falls more than 9 rpm within 3 s
/// (3 rpm/s, six times the cue's 0.5 rpm/s, so a 5 rpm step made over a second or two
/// passes) and stays within 15 rpm of the cue. A stall is power below 90 % of the target for
/// 2 s (the 2 s of the §7.4 release rule) with the cadence falling no faster than that from
/// one power time constant before the drop: the trainer let go while the rider kept turning.
/// Power and cadence falling together is a rider who stopped, or one the trainer overpowered;
/// from page 25 the two look alike, so neither names a stall cadence. The verdict is
/// [`stall_verdict`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Descent {
    /// No decision yet.
    Going,
    /// The trainer let go while the rider followed.
    Stalled {
        /// First sample of the power drop.
        onset_us: u64,
        /// The drop has lasted 2 s.
        at_us: u64,
        /// Highest cadence among the samples of the drop (power already below 90 %).
        dropped_at_rpm: u8,
    },
    /// The cadence reached the floor with power still held.
    Floor {
        /// The first sample below the floor.
        at_us: u64,
    },
    /// The rider did not follow the cue: no stall can be named.
    Departed {
        /// Where it was seen.
        at_us: u64,
        /// What happened.
        why: String,
    },
}

/// Largest fall of cadence (from a running peak) over `s[..=to]` at or after `from_us`.
fn largest_fall(s: &[&Sample], from_us: u64, to: usize) -> f64 {
    let start = s[..=to].partition_point(|x| x.at_us < from_us);
    let mut peak: Option<u8> = None;
    let mut fall = 0_u8;
    for c in s[start..=to].iter().filter_map(|x| x.cadence) {
        let p = peak.map_or(c, |p| p.max(c));
        peak = Some(p);
        fall = fall.max(p - c);
    }
    f64::from(fall)
}

/// Judge a check-9 descent that started at `from_us` against `target_w` (see [`Descent`]).
/// The live run and `analyse` call it on the same page-25 samples.
pub fn descent(samples: &[Sample], from_us: u64, target_w: f64) -> Descent {
    let s: Vec<&Sample> = samples.iter().filter(|x| x.at_us >= from_us).collect();
    let mut onset: Option<usize> = None;
    for (i, x) in s.iter().enumerate() {
        let departed = |why: String| Descent::Departed {
            at_us: x.at_us,
            why,
        };
        let (Some(c), Some(p)) = (x.cadence, x.power) else {
            return departed("cadence or power unavailable during the descent".into());
        };
        let low = f64::from(p) < STALL_SHARE * target_w;
        onset = if low { onset.or(Some(i)) } else { None };
        if let Some(o) = onset {
            let o_us = s[o].at_us;
            if x.at_us - o_us < STALL_CONFIRM_US {
                continue;
            }
            let from = o_us.saturating_sub(POWER_TAU_US);
            let fall = largest_fall(&s, from, i);
            if fall > DROP_RPM {
                return departed(format!(
                    "power and cadence fell together ({fall} rpm within {} s, now {c} rpm): the rider stopped or was overpowered, not a trainer stall",
                    round1(secs(x.at_us - from))
                ));
            }
            let dropped_at_rpm = s[o..=i].iter().filter_map(|y| y.cadence).max().unwrap_or(c);
            return Descent::Stalled {
                onset_us: o_us,
                at_us: x.at_us,
                dropped_at_rpm,
            };
        }
        let fall = largest_fall(&s, x.at_us.saturating_sub(DROP_WINDOW_US), i);
        if fall > DROP_RPM {
            return departed(format!(
                "cadence fell {fall} rpm within 3 s to {c} rpm (the cue is 5 rpm per 10 s): the rider stopped, not the trainer"
            ));
        }
        if c < STALL_FLOOR_RPM {
            return Descent::Floor { at_us: x.at_us };
        }
        let cue = CUE_START_RPM - CUE_RPM_PER_S * secs(x.at_us - from_us);
        if f64::from(c) > cue + BEHIND_RPM {
            return departed(format!(
                "{c} rpm where the cue is at {cue:.0} rpm: the rider did not follow the cued descent"
            ));
        }
        if f64::from(c) < cue - AHEAD_RPM {
            return departed(format!(
                "{c} rpm where the cue is at {cue:.0} rpm: far below the cued descent, the rider did not follow it"
            ));
        }
    }
    Descent::Going
}

/// The lowest cadence at which power was settled at or above 90 % of the target: the highest
/// cadence of a stretch of samples spanning [`SETTLE_US`], at most [`MAX_GAP_US`] apart, with
/// power at or above 90 % throughout; the lowest such over all stretches.
pub fn settled_hold(samples: &[&Sample], target_w: f64) -> Option<u8> {
    let held = |x: &&Sample| {
        x.cadence.is_some()
            && x.power
                .is_some_and(|p| f64::from(p) >= STALL_SHARE * target_w)
    };
    let mut best: Option<u8> = None;
    for k in 0..samples.len() {
        let end = samples[k].at_us;
        let Some(j) = (0..=k).rev().find(|&j| end - samples[j].at_us >= SETTLE_US) else {
            continue;
        };
        let w = &samples[j..=k];
        if w.iter().all(held) && w.windows(2).all(|p| p[1].at_us - p[0].at_us <= MAX_GAP_US) {
            let top = w.iter().filter_map(|x| x.cadence).max();
            best = match (best, top) {
                (Some(b), Some(t)) => Some(b.min(t)),
                (b, t) => b.or(t),
            };
        }
    }
    best
}

/// The check-9 verdict of a descent that ended in a stall or at the floor, from two bounds
/// on the cadence below which the trainer can no longer hold 90 %:
/// - it is above `dropped_at_rpm` (minus 0.5 rpm rounding): power below 90 % there, and a
///   lagging power only reads high, never low, while cadence falls;
/// - it is at most `held_rpm` + [`MARGIN_RPM`] when power was settled at or above 90 % at
///   cadences up to `held_rpm` (valid under the 1 s power-lag bound).
///
/// "First" needs the upper bound below 40 rpm; "too late" the lower bound at or above it;
/// anything else is ambiguous. The upper bound rests on the lag bound, so "first" needs it
/// measured by check 6 (`lag`): unmeasured, the verdict says so; too slow, only "too late"
/// (which needs no lag bound) stands and anything else is not valid.
pub fn stall_verdict(d: &Descent, held_rpm: Option<u8>, lag: &PowerLag) -> Option<String> {
    let dropped = match *d {
        Descent::Stalled { dropped_at_rpm, .. } => Some(dropped_at_rpm),
        Descent::Floor { .. } => None,
        Descent::Going | Descent::Departed { .. } => return None,
    };
    let first = held_rpm.is_some_and(|h| h + MARGIN_RPM < RELEASE_RPM);
    let late = dropped.is_some_and(|c| c > RELEASE_RPM);
    let range = format!(
        "power fell below 90 % at {} rpm, held settled down to {} rpm",
        dropped.map_or_else(|| "no".into(), |c| c.to_string()),
        held_rpm.map_or_else(|| "no settled hold".into(), |c| c.to_string()),
    );
    if let PowerLag::TooSlow(_) = lag {
        return Some(if late {
            "stall at or above the release cadence: the release comes too late".into()
        } else {
            format!("not valid: power response too slow (t90 = {})", lag.t90())
        });
    }
    Some(match (first, late) {
        (true, false) if matches!(lag, PowerLag::NotMeasured(_)) => format!(
            "stall below the release cadence: the release comes first only if check 6 t90 ≤ {} s (not measured)",
            secs(MAX_T90_US)
        ),
        (true, false) => "stall below the release cadence: the release comes first".into(),
        (false, true) => "stall at or above the release cadence: the release comes too late".into(),
        (true, true) => format!("inconsistent: {range} (power not monotone in cadence)"),
        (false, false) => format!(
            "ambiguous: {range}; the stall is not resolved against the {RELEASE_RPM} rpm release"
        ),
    })
}

/// A cadence bin is followed: at least 2 samples, [`FOLLOWED_SHARE`] within ±10 %.
fn bin_followed(p: &[f64], target: f64) -> bool {
    p.len() >= 2 && share_within(p, target, 0.10).is_some_and(|x| x >= FOLLOWED_SHARE)
}

/// The capture time of the procedure's first page-49 write of 250 W: [`DESCENT_AFTER_US`]
/// before the descent starts. The live run and `analyse` both start it from here, also when
/// the soft start begins at 250 W and writes it six times.
pub fn descent_target_at(w: &[Write]) -> Option<u64> {
    // Before the run's last release (as [`released_at`]), when there is one.
    let end = w
        .iter()
        .rposition(|x| x.control == Control::Resistance(0.0))
        .filter(|i| *i > 0)
        .map_or(usize::MAX, |i| w[i].pos);
    let target = f64::from(crate::rider::MAX_ERG_W);
    w.iter()
        .find(|w| w.pos < end && w.control == Control::Erg(target))
        .map(|w| w.at_us)
}

/// Check 9: down to which cadence ERG holds 250 W while the rider follows the cued descent.
/// `lag` is check 6's measure of the power response, which "the release comes first" needs.
pub fn check9(tl: &Timeline, lag: &PowerLag) -> Value {
    // The descent reads every page 25, repeats too, as the live run does; the bins read updates.
    let s = tl.samples();
    let ctl = tl.controls();
    let end = procedure_end(tl.end_us(), &ctl);
    let target = f64::from(crate::rider::MAX_ERG_W);
    let Some(at) = descent_target_at(tl.writes()) else {
        return json!({"error": format!("no page-49 write of {target} W")});
    };
    let from = at + DESCENT_AFTER_US;
    let d = descent(s, from, target);
    let until = match &d {
        Descent::Stalled { at_us, .. }
        | Descent::Floor { at_us, .. }
        | Descent::Departed { at_us, .. } => (*at_us).min(end),
        Descent::Going => end,
    };
    // The settled hold before the power drop (or up to the floor).
    let hold_until = match &d {
        Descent::Stalled { onset_us, .. } => *onset_us,
        _ => until,
    };
    let descent_samples: Vec<&Sample> = s
        .iter()
        .filter(|x| !x.repeat && x.at_us >= from && x.at_us < hold_until)
        .collect();
    let held = settled_hold(&descent_samples, target);
    let mut bins: std::collections::BTreeMap<u8, Vec<f64>> = std::collections::BTreeMap::new();
    for x in s
        .iter()
        .filter(|x| !x.repeat && x.at_us >= at && x.at_us <= until)
    {
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
            dropped_at_rpm,
        } => (
            "stalled",
            json!({"onset_s": round1(secs(*onset_us)), "at_s": round1(secs(*at_us)), "dropped_at_rpm": dropped_at_rpm}),
        ),
        Descent::Floor { at_us } => ("floor", json!({"at_s": round1(secs(*at_us))})),
        Descent::Departed { at_us, why } => (
            "departed",
            json!({"at_s": round1(secs(*at_us)), "why": why}),
        ),
    };
    json!({
        "method": "descent from 10 s after the procedure's 250 W write; the cue falls from 90 rpm by 0.5 rpm/s; followed = cadence never falls > 9 rpm within 3 s and stays within 15 rpm of the cue; stall = power < 90 % of the target for 2 s with the cadence followed from 1 s before the drop; floor = first sample below 25 rpm with power held; anything else departs (no verdict). Verdict (power lag assumed first-order, tau <= 1 s): first = power settled >= 90 % for 5 s (samples <= 0.5 s apart) at cadences up to N with N + 2 < 40, and only with check 6 of the same capture set at t90 <= 2.3 s; too late = power < 90 % at a cadence > 40; otherwise ambiguous (not valid when check 6 t90 > 2.3 s). Bins: 5 rpm, followed = >= 2 samples and >= 80 % within ±10 %",
        "target_w": target,
        "descent_from_s": round1(secs(from)),
        "descent": event,
        "descent_detail": detail,
        "held_settled_to_rpm": held,
        "bins": bins.iter().map(|(c, p)| json!({
            "cadence_rpm": c, "samples": p.len(), "mean_power_w": mean(p.iter().copied()).map(round1),
            "within_10pct": share_within(p, target, 0.10).map(|x| round1(x * 100.0)),
        })).collect::<Vec<_>>(),
        "lowest_followed_rpm": lowest,
        "release_rpm": RELEASE_RPM,
        "reengage_rpm": REENGAGE_RPM,
        "power_lag": lag.json(),
        "verdict": stall_verdict(&d, held, lag),
    })
}

/// Check 12's question (ADR-0007 §2), asked at the end of the 70 rpm hold; its answer's stamp
/// ends the 70 rpm window.
pub const EASIER: crate::check10::Question = crate::check10::Question {
    key: "easier",
    text: "Compared with the 90 rpm part right after the switch, did pedalling at 70 rpm get clearly easier? (y = easier / n = not easier / s = I shifted gear)",
    options: &["y", "n", "s"],
};
/// Tag of check 12's cue stamp, `# check12 <µs> 46`: the cue to 70 rpm, code [`TO_70`].
pub const CUE12: &str = "check12";
/// The code of check 12's 70 rpm cue stamp (its cadence).
pub const TO_70: u8 = 70;
/// Check 12's settle after the page-51 write and after the first 70 rpm update (ADR-0007 §2).
const SETTLE12_US: u64 = 5 * SEC;
/// Check 12 reads a window of fewer updates than this as no result.
const MIN_UPDATES12: usize = 15;
/// Check 12's power floor, W (ADR-0007 §2): a `free90` or `free70` median below it is no result.
/// The slowest road ADR-0007 §1 models is 10 km/h at 90 rpm; on it, a released NEO's flat road
/// (Crr 0.004, 85 kg, Kw 0.51 kg/m) needs 8.75 W at 65 rpm, the lowest cadence of either band,
/// and more at every other in-band cadence. Rounded down to page 25's whole watts: 8 W. Below
/// it the ratio is a ratio of a few 1 W steps (or of 0), which says nothing about a release.
const FLOOR12_W: f64 = 8.0;
/// Check 12's cadence bands, rpm.
const BAND90: std::ops::RangeInclusive<u8> = 85..=95;
const BAND70: std::ops::RangeInclusive<u8> = 65..=75;

/// The median of `v`, `None` when it is empty.
fn median(mut v: Vec<f64>) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    (n > 0).then(|| f64::midpoint(v[(n - 1) / 2], v[n / 2]))
}

/// Check 12's one answer, after the cue (the note at index `cue_seq`): its note and its text.
fn answer12(tl: &Timeline, cue_seq: usize) -> Result<(crate::frames::Note, String), String> {
    let tag = crate::check10::answer_tag(EASIER.key);
    // The answer notes, with their index among the notes (file order).
    let answers: Vec<(usize, &crate::frames::Note)> = tl
        .notes()
        .iter()
        .enumerate()
        .filter(|(_, n)| n.tag == tag)
        .collect();
    let [(seq, answer)] = answers[..] else {
        return Err(format!(
            "{} answers to the check 12 question, not one",
            answers.len()
        ));
    };
    let said = String::from_utf8_lossy(&answer.bytes).into_owned();
    if seq < cue_seq || !EASIER.options.contains(&said.as_str()) {
        return Err(format!(
            "the answer {said:?} is not one to the check 12 question after its cue"
        ));
    }
    Ok((answer.clone(), said))
}

/// Why check 12 gives no result (ADR-0007 §2's verdict, in order), `None` for PASS or FAIL;
/// `updates` is the smaller window's count, `lowest` the smaller of the `free90` and `free70`
/// medians.
fn verdict12(
    erg: Option<f64>,
    updates: usize,
    lowest: Option<f64>,
    r: Option<f64>,
    said: &str,
) -> Option<&'static str> {
    if !erg.is_some_and(|e| (180.0..=220.0).contains(&e)) {
        Some("ERG did not hold 180-220 W before Free, so no release can be seen")
    } else if updates < MIN_UPDATES12 {
        Some("fewer than 15 updates in the 90 or the 70 rpm window")
    } else if !lowest.is_some_and(|w| w >= FLOOR12_W) {
        Some("power under Free below the 8 W floor (slower than a 10 km/h road)")
    } else if said == "s" {
        Some("the rider shifted gear")
    } else if (r.is_some_and(|r| r <= 0.85) && said == "y")
        || (r.is_some_and(|r| r >= 0.95) && said == "n")
    {
        None
    } else {
        Some("power and rider disagree, or r is between 0.85 and 0.95")
    }
}

/// Check 12 (ADR-0007 §2): does `Free` (page 50 defaults, page 51 grade 0) release an ERG target?
/// Read from the ratio r of the median power of the updates at 65–75 rpm to that at 85–95 rpm
/// under `Free`, and from the rider's answer: PASS iff r ≤ 0.85 and `y`, FAIL iff r ≥ 0.95 and
/// `n`, otherwise no result.
pub fn check12(tl: &Timeline) -> Value {
    let ctl = tl.writes();
    let Some((erg_i, target)) = ctl
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, w)| match w.control {
            Control::Erg(watts) => Some((i, watts)),
            _ => None,
        })
    else {
        return json!({"error": "no page-49 target"});
    };
    let after = &ctl[erg_i + 1..];
    let wind = after
        .iter()
        .find(|w| matches!(w.control, Control::Wind(..)));
    let track = after
        .iter()
        .find(|w| matches!(w.control, Control::Track(g, _) if g.abs() < 1e-9));
    let (Some(wind), Some(track)) = (wind, track) else {
        return json!({"error": "no page 50 + page 51 grade 0 after the ERG target"});
    };
    let cue = match crate::frames::stamps(tl.notes(), CUE12, &[TO_70]) {
        Err(why) => return json!({ "error": why }),
        Ok(st) => crate::frames::stamp_of(&st, TO_70),
    };
    let Some(cue) = cue.filter(|c| c.before > track.pos) else {
        return json!({"error": "no 70 rpm cue stamp after page 51"});
    };
    let (answer, said) = match answer12(tl, cue.seq) {
        Ok(a) => a,
        Err(why) => return json!({ "error": why }),
    };
    let ups = tl.updates();
    let power = |from: usize, to: usize, band: Option<&std::ops::RangeInclusive<u8>>| {
        let p: Vec<f64> = ups
            .iter()
            .filter(|s| s.pos >= from && s.pos < to)
            .filter(|s| band.is_none_or(|b| s.cadence.is_some_and(|c| b.contains(&c))))
            .filter_map(|s| s.power.map(f64::from))
            .collect();
        (p.len(), median(p))
    };
    let (_, erg) = power(
        tl.index_at(wind.at_us.saturating_sub(5 * SEC)),
        wind.pos,
        None,
    );
    let from90 = tl.index_at(track.at_us + SETTLE12_US);
    let (n90, free90) = power(from90, cue.before, Some(&BAND90));
    let t70 = ups
        .iter()
        .find(|s| s.pos >= cue.before && s.cadence.is_some_and(|c| BAND70.contains(&c)));
    let (n70, free70) = t70.map_or((0, None), |t| {
        power(
            tl.index_at(t.at_us + SETTLE12_US),
            answer.before,
            Some(&BAND70),
        )
    });
    let r = free70.zip(free90).map(|(a, b)| a / b);
    let lowest = free70.zip(free90).map(|(a, b)| a.min(b));
    let no_result = verdict12(erg, n90.min(n70), lowest, r, &said);
    let released = if no_result.is_some() {
        Value::Null
    } else {
        json!(said == "y")
    };
    // A page-71 reply after the write naming that page and echoing its bytes 4-7.
    let echoed = |page: u8, w: &Write| {
        tl.statuses()
            .iter()
            .any(|st| st.at_us >= w.at_us && st.page == page && st.echo == w.data)
    };
    json!({
        "method": "ADR-0007 §2, page-25 updates only: erg = median power in the 5 s before page 50; free90 = median power at 85-95 rpm from page 51 + 5 s to the 70 rpm cue; free70 = median power at 65-75 rpm from the first such update after the cue + 5 s to the answer; r = free70 / free90. No result unless erg is 180-220 W, each window has >= 15 updates and free90 and free70 are >= 8 W; the answer s is no result; released iff r <= 0.85 and y, not released iff r >= 0.95 and n, else no result. echoed = a later page-71 reply names the page and echoes its bytes 4-7",
        "erg_target_w": target,
        "free_at_s": round1(secs(track.at_us)),
        "cue_70rpm_at_s": round1(secs(cue.at_us)),
        "answer_at_s": round1(secs(answer.at_us)),
        "erg_w": erg.map(round1),
        "free90_w": free90.map(round1),
        "free90_updates": n90,
        "free70_w": free70.map(round1),
        "free70_updates": n70,
        "r": r.map(|r| (r * 1000.0).round() / 1000.0),
        "answer": said,
        "no_result": no_result,
        "page71_echoed_50": echoed(50, wind),
        "page71_echoed_51": echoed(51, track),
        "released": released,
    })
}

/// Check 6's exact control program ([`Timeline::program`], ADR-0007 §4): the soft start to the
/// first of [`crate::rider::CHECK6_W`], then each next one.
const PROGRAM_6: &[&[Step]] = &[&[
    Step::Release,
    Step::SoftStart(crate::rider::CHECK6_W[0]),
    Step::Erg(crate::rider::CHECK6_W[1]),
    Step::Erg(crate::rider::CHECK6_W[2]),
    Step::Erg(crate::rider::CHECK6_W[3]),
    Step::Erg(crate::rider::CHECK6_W[4]),
    Step::Release,
]];
/// Check 9's: the soft start to 250 W, nothing more.
const PROGRAM_9: &[&[Step]] = &[&[
    Step::Release,
    Step::SoftStart(crate::rider::MAX_ERG_W),
    Step::Release,
]];
/// Check 12's: the soft start to 200 W, then `Free` (page 50 defaults, page 51 at 0 %).
const PROGRAM_12: &[&[Step]] = &[&[
    Step::Release,
    Step::SoftStart(crate::rider::CHECK12_W),
    Step::Wind,
    Step::Track(0),
    Step::Release,
]];

/// The first malformed control write, named: the harness encodes only valid fields, so such a
/// write is not its procedure's, and no check gives a result from that capture.
fn malformed(ws: &[Write]) -> Option<String> {
    ws.iter().find_map(|w| match w.control {
        Control::Malformed(page) => Some(format!(
            "malformed control write (page {page} at {} s)",
            round1(secs(w.at_us))
        )),
        Control::NotAFrame => Some(format!(
            "malformed control write (not an ANT frame, at {} s)",
            round1(secs(w.at_us))
        )),
        _ => None,
    })
}

/// A capture without its closing release: every run writes the release last (recorded before it
/// is written, a failed one too), so a capture whose last control write is not a release, or is
/// the opening one, was cut off before the run's end and gives no result, as a live run without
/// its end would not.
fn cut_off(tl: &Timeline) -> Option<String> {
    tl.closing_release()
        .is_none()
        .then(|| "the capture is cut off: no release write ends it".into())
}

/// A Part B check of a timeline.
type Check = fn(&Timeline, &PowerLag) -> Value;

/// An `unknown` result.
fn unknown(why: &String) -> Value {
    json!({ "verdict": format!("unknown: {why}") })
}

/// The check's value of `tl`, and why it gives no result if it gives none: a malformed write or a
/// cut-off capture; for a run that claims to have `completed`, a breach of its exact control
/// `program` (checks 6, 9 and 12; then the check does not read it) or the check's own `error`
/// (what its procedure writes is not there).
fn result_of(
    tl: &Timeline,
    check: Check,
    program: Option<&[&[Step]]>,
    completed: bool,
    lag: &PowerLag,
) -> (Value, Option<String>) {
    if let Some(why) = malformed(tl.writes()).or_else(|| cut_off(tl)) {
        return (check(tl, lag), Some(why));
    }
    if completed && let Some(Err(why)) = program.map(|p| tl.program(p)) {
        return (unknown(&why), Some(why));
    }
    let v = check(tl, lag);
    let error = v["error"]
        .as_str()
        .filter(|_| completed)
        .map(str::to_string);
    (v, error)
}

/// Part B analysis by capture stem (`check5` …) of a capture's text, `None` for other captures.
/// `outcome` is how the procedure ended (the capture's outcome line); without `completed` there
/// is no verdict. `lag` is check 6 of the same capture set ([`PowerLag::beside`]), which check 9
/// needs. The text is read once into a [`Timeline`]; one that is not in time order is no result,
/// and no check reads it.
pub fn analyse(stem: &str, text: &str, outcome: Option<&str>, lag: &PowerLag) -> Option<Value> {
    let (check, program): (Check, Option<&[&[Step]]>) = match stem {
        "check5" => (|tl, _| crate::cps::check5(tl), None),
        "check6" => (|tl, _| check6(tl), Some(PROGRAM_6)),
        "check7" => (|tl, _| check7(tl), None),
        "check8" => (|tl, _| check8(tl), None),
        "check9" => (check9, Some(PROGRAM_9)),
        "check10" => (|tl, _| crate::check10::check10(tl), None),
        "check11a" => (|tl, _| crate::check11ac::check11a(tl), None),
        "check11b" => (|tl, _| crate::check11::check11b(tl), None),
        "check11c" => (|tl, _| crate::check11ac::check11c(tl), None),
        "check12" => (|tl, _| check12(tl), Some(PROGRAM_12)),
        _ => return None,
    };
    // A capture that claims a completed run is held to the check's control program before the
    // check reads it; a run that did not complete gives no result anyway.
    let timeline = Timeline::parse(text);
    let (mut v, invalid) = match &timeline {
        Err(why) => (unknown(why), Some(why.clone())),
        Ok(tl) => result_of(tl, check, program, outcome == Some(COMPLETED), lag),
    };
    let malformed = invalid;
    let completed = outcome == Some(COMPLETED) && malformed.is_none();
    v["procedure"] = json!(outcome.unwrap_or("unknown: the capture has no outcome line"));
    v["completed"] = json!(completed);
    v["invalid"] = json!(malformed);
    if !completed {
        for verdict in [
            "held",
            "released",
            "verdict",
            "advertising",
            "second_central",
            "drop",
            "cause",
            "reconnect",
            "after_reconnect",
            "link",
            "before",
            "after",
            "times",
            "low_cadence",
            "normal_cadence",
        ] {
            if v.get(verdict).is_some() {
                v[verdict] = Value::Null;
            }
        }
    }
    // Read from the timeline too: a capture that is none says nothing about the release either.
    v["release"] = timeline.as_ref().map_or_else(
        |_| json!({ "page48_0pct_last": null, "at_s": null, "page71_echoed": null }),
        |tl| release(tl.writes(), tl.statuses()),
    );
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
    } else if let Some(why) = v["invalid"].as_str() {
        format!("no result: {why}")
    } else {
        format!(
            "no result: procedure {}",
            v["procedure"].as_str().unwrap_or("unknown")
        )
    };
    let release = match v["release"]["page48_0pct_last"].as_bool() {
        Some(true) => "released",
        Some(false) => "NOT released",
        None => "release not read (no timeline)",
    };
    format!("| {n} | {what} | {result} (trainer {release} at the end) | {adr} |")
}

/// Check number, check, result and ADR-0002 section of a completed run's row.
fn cells(stem: &str, v: &Value) -> (&'static str, &'static str, String, &'static str) {
    let empty = Vec::new();
    let list = |key: &str| v[key].as_array().unwrap_or(&empty).clone();
    match stem {
        "check5" => (
            "5",
            "Page 25 rate and power/cadence against CPS 2A63",
            crate::cps::result(v),
            "§5",
        ),
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
        "check10" => crate::check10::cells(v),
        "check11a" | "check11b" | "check11c" => crate::check11ac::cells(stem, v),
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
        _ => cells12(v),
    }
}

/// Check 12's row cells: PASS, FAIL or no result with why, and r and the answer either way.
fn cells12(v: &Value) -> (&'static str, &'static str, String, &'static str) {
    let read = format!(
        "r = {} (Free {} W at 90 rpm over {} updates, {} W at 70 rpm over {}; ERG {} W), rider: {}; page 71 echoed 50/51: {}/{}",
        num(&v["r"]),
        num(&v["free90_w"]),
        num(&v["free90_updates"]),
        num(&v["free70_w"]),
        num(&v["free70_updates"]),
        num(&v["erg_w"]),
        v["answer"].as_str().unwrap_or("?"),
        v["page71_echoed_50"],
        v["page71_echoed_51"]
    );
    let result = match v["released"].as_bool() {
        Some(true) => format!("PASS, released: {read}"),
        Some(false) => format!("FAIL, not released: {read}"),
        None => format!(
            "no result: {}; {read}",
            v["no_result"].as_str().unwrap_or("?")
        ),
    };
    ("12", "ERG 200 W → Free", result, "§7.9")
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
        let count = u8::try_from(at_us / 250_000 % 256).expect("a byte");
        let mut f = [
            0xa4, 9, 0x4e, 5, 25, count, cadence, 0, 0, w[0], w[1], 0x30, 0,
        ];
        f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
        Record {
            at_us,
            rx: true,
            bytes: f.to_vec(),
        }
    }

    /// The page-70 requests every run writes: for page 81 after its opening release, for page 71
    /// after its closing one.
    fn serial_request(at_us: u64) -> Record {
        tx(at_us, &ControlPage::Request { page: 81, times: 1 })
    }
    fn status_request(at_us: u64) -> Record {
        tx(at_us, &ControlPage::Request { page: 71, times: 1 })
    }

    /// `Rig::engage` from 0 W to `target` W as the soft start writes it, its last write at `end_us`.
    fn soft_start(end_us: u64, target: f32) -> Vec<Record> {
        let erg = |watts| ControlPage::TargetPower { watts };
        let mut v = vec![tx(end_us - 600_000, &erg(0.0))];
        v.extend((0..=5_u8).map(|k| {
            tx(
                end_us - u64::from(5 - k) * 100_000,
                &erg(target * f32::from(k) / 5.0),
            )
        }));
        v
    }

    /// A control page with a field set to its "invalid" value (checksum fixed).
    fn invalid(at_us: u64, page: &ControlPage, field: [usize; 2]) -> Record {
        let mut f = tx(at_us, page);
        for i in field {
            f.bytes[i] = 0xff;
        }
        f.bytes[12] = f.bytes[..12].iter().fold(0, |x, v| x ^ v);
        f
    }

    /// Every run ends on its release: a capture whose last control write is not page 48 at 0 %, or
    /// whose only one is the opening release, was cut off and gives no result, whatever its
    /// outcome line says.
    #[test]
    fn a_capture_cut_off_before_its_closing_release_gives_no_result() {
        let free = ControlPage::BasicResistance { pct: 0.0 };
        let p = |n: u64| (2..n).map(|k| p25(k * SEC / 2, 90, 150));
        let none = PowerLag::NotMeasured("test".into());
        let cut = "the capture is cut off: no release write ends it";
        for (ride, invalid) in [
            (
                [
                    vec![tx(0, &free), serial_request(100_000)],
                    soft_start(SEC, 150.0),
                    p(120).collect(),
                    vec![tx(60 * SEC, &free), status_request(60 * SEC + 300_000)],
                ]
                .concat(),
                None,
            ),
            (
                [vec![tx(0, &free)], soft_start(SEC, 150.0), p(120).collect()].concat(),
                Some(cut),
            ),
            ([vec![tx(0, &free)], p(120).collect()].concat(), Some(cut)),
            (p(120).collect(), Some(cut)),
        ] {
            let text = crate::timeline::text_of(&ride, &[], &[]);
            let v = analyse("check7", &text, Some(COMPLETED), &none).expect("stem");
            assert_eq!(v["invalid"].as_str(), invalid, "{v}");
            assert_eq!(v["completed"], json!(invalid.is_none()));
        }
    }

    /// A malformed control page is a control write: it is never the release, it ends an ERG hold,
    /// and no check (here 7 and 12) gives a result from a capture that has one.
    #[test]
    fn a_malformed_control_write_counts_and_no_check_gives_a_result_beside_it() {
        let grade = |at| {
            invalid(
                at,
                &ControlPage::Track {
                    grade_pct: 5.0,
                    crr: 0.004,
                },
                [9, 10],
            )
        };
        let rider = |at| {
            let user = ControlPage::UserConfig {
                rider_kg: 75.0,
                bike_kg: 10.0,
                wheel_m: 0.708,
            };
            invalid(at, &user, [5, 6])
        };
        let free = ControlPage::BasicResistance { pct: 0.0 };
        let mut ride = vec![tx(0, &free), serial_request(100_000)];
        ride.extend(soft_start(SEC, 150.0));
        ride.extend((2..120).map(|k| p25(k * SEC / 2, 90, 150)));
        ride.push(tx(60 * SEC, &free));
        ride.push(status_request(60 * SEC + 300_000));
        let none = PowerLag::NotMeasured("test".into());
        let run = |r: &[Record], stem| {
            let text = crate::timeline::text_of(r, &[], &[]);
            analyse(stem, &text, Some(COMPLETED), &none).expect("stem")
        };
        assert_eq!(run(&ride, "check7")["completed"], json!(true));
        assert_eq!(
            run(&ride, "check7")["release"]["page48_0pct_last"],
            json!(true)
        );
        for (bad, page) in [(grade(30 * SEC), 51), (rider(30 * SEC), 55)] {
            let mut r = ride.clone();
            r.push(bad.clone());
            r.sort_by_key(|x| x.at_us);
            assert_eq!(controls(&r)[8], (30 * SEC, Control::Malformed(page)));
            let holds = erg_holds(
                &controls(&r),
                procedure_end(r.last().map_or(0, |x| x.at_us), &controls(&r)),
            );
            assert_eq!(
                holds.last().expect("the 150 W hold").1.end,
                30 * SEC,
                "the malformed write ends the hold"
            );
            for stem in ["check7", "check12"] {
                let v = run(&r, stem);
                assert_eq!(v["completed"], json!(false), "{stem}: {v}");
                assert_eq!(
                    v["line"].as_str().map(|l| l.contains(&format!(
                        "no result: malformed control write (page {page} at 30 s)"
                    ))),
                    Some(true),
                    "{stem}: {v}"
                );
            }
        }
        // A tx record that is no ANT frame (short, or without the 0xA4 sync) counts the same.
        for bytes in [vec![0xa4, 9, 0x4f], {
            let mut f = tx(0, &free).bytes;
            f[0] = 0;
            f
        }] {
            let mut r = ride.clone();
            r.push(Record {
                at_us: 30 * SEC,
                rx: false,
                bytes,
            });
            r.sort_by_key(|x| x.at_us);
            assert_eq!(controls(&r)[8], (30 * SEC, Control::NotAFrame));
            let v = run(&r, "check7");
            assert_eq!(v["completed"], json!(false), "{v}");
            assert!(
                v["line"].as_str().is_some_and(|l| l
                    .contains("no result: malformed control write (not an ANT frame, at 30 s)")),
                "{v}"
            );
        }
        // After the final release, it makes the run not released.
        let mut late = ride.clone();
        late.push(grade(61 * SEC));
        assert_eq!(
            release(&writes(&late), &statuses(&late))["page48_0pct_last"],
            json!(false)
        );
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

    /// Samples every `gap_us` from 0 while `at_us < end_us`, `(cadence, power)` by time.
    fn series(gap_us: u64, end_us: u64, f: impl Fn(u64) -> (u8, u16)) -> Vec<Sample> {
        (0..end_us.div_ceil(gap_us))
            .map(|k| k * gap_us)
            .map(|at_us| {
                let (c, p) = f(at_us);
                Sample {
                    at_us,
                    cadence: Some(c),
                    power: Some(p),
                    target_limit: 0,
                    state: 3,
                    status: 0,
                    pos: 0,
                    repeat: false,
                }
            })
            .collect()
    }

    fn departed(d: &Descent) -> bool {
        matches!(d, Descent::Departed { .. })
    }

    #[test]
    fn a_fall_of_9_rpm_in_3_s_follows_10_does_not() {
        for (to, leaves) in [(81, false), (80, true)] {
            let s = series(250_000, 6 * SEC, |t| {
                (if t < 2 * SEC { 90 } else { to }, 250)
            });
            assert_eq!(departed(&descent(&s, 0, 250.0)), leaves, "90 -> {to}");
        }
    }

    #[test]
    fn following_is_within_15_rpm_of_the_cue() {
        // The cue is 90 rpm at the start of the descent.
        for (c, leaves) in [(105, false), (106, true), (75, false), (74, true)] {
            let s = series(250_000, 250_000, |_| (c, 250));
            assert_eq!(departed(&descent(&s, 0, 250.0)), leaves, "{c} rpm");
        }
    }

    #[test]
    fn power_at_exactly_90_percent_is_held() {
        let s = series(250_000, 4 * SEC, |_| (90, 225));
        assert_eq!(descent(&s, 0, 250.0), Descent::Going);
        let s = series(250_000, 4 * SEC, |_| (90, 224));
        assert!(matches!(
            descent(&s, 0, 250.0),
            Descent::Stalled {
                dropped_at_rpm: 90,
                ..
            }
        ));
    }

    /// `series` placed `at_s` into the descent, where the cue is at `90 - at_s / 2` rpm.
    fn into_descent(mut s: Vec<Sample>, at_s: u64) -> Vec<Sample> {
        for x in &mut s {
            x.at_us += at_s * SEC;
        }
        s
    }

    #[test]
    fn power_and_cadence_falling_together_counts_from_one_time_constant_before() {
        // Cue near 55 rpm. 60 rpm, 6 rpm lower in the second before power drops, then down to
        // `to` with power at 200 W: 9 rpm in all is a stall, 10 rpm is the rider stopping.
        for (to, leaves) in [(51, false), (50, true)] {
            let s = into_descent(
                series(250_000, 6 * SEC, |t| match t {
                    t if t < 2_250_000 => (60, 250),
                    t if t < 3 * SEC => (54, 250),
                    _ => (to, 200),
                }),
                70,
            );
            let d = descent(&s, 0, 250.0);
            assert_eq!(departed(&d), leaves, "60 -> {to}: {d:?}");
        }
    }

    #[test]
    fn the_stall_is_bounded_by_the_highest_cadence_of_the_drop() {
        // Cue near 41 rpm: power drops at 41 rpm, the cadence then eases to 40.
        let s = into_descent(
            series(250_000, 6 * SEC, |t| match t {
                t if t < 2 * SEC => (42, 250),
                t if t < 2_500_000 => (41, 200),
                _ => (40, 200),
            }),
            98,
        );
        let d = descent(&s, 0, 250.0);
        assert!(
            matches!(
                d,
                Descent::Stalled {
                    dropped_at_rpm: 41,
                    ..
                }
            ),
            "{d:?}"
        );
        assert_eq!(
            stall_verdict(&d, None, &PowerLag::Within(MAX_T90_US)).expect("verdict"),
            "stall at or above the release cadence: the release comes too late"
        );
    }

    #[test]
    fn a_settled_hold_spans_5_s_with_samples_at_most_0_5_s_apart() {
        let held = |gap_us, end_us, power| {
            let s = series(gap_us, end_us, |t| (if t == 0 { 40 } else { 36 }, power));
            settled_hold(&s.iter().collect::<Vec<_>>(), 250.0)
        };
        // 0 to 5 s: the one stretch includes the 40 rpm sample.
        assert_eq!(held(500_000, 5_000_001, 250), Some(40));
        assert_eq!(held(500_000, 5_000_000, 250), None, "4.5 s is not settled");
        // A later stretch without the 40 rpm sample.
        assert_eq!(held(500_000, 5_500_001, 250), Some(36));
        assert_eq!(held(500_000, 5_500_001, 225), Some(36), "90 % is held");
        assert_eq!(held(500_000, 5_500_001, 224), None);
        assert_eq!(held(550_000, 6 * SEC, 250), None, "samples 0.55 s apart");
    }

    #[test]
    fn the_verdict_needs_a_bound_on_the_right_side_of_40_rpm() {
        let stalled = |dropped_at_rpm| Descent::Stalled {
            onset_us: 0,
            at_us: 0,
            dropped_at_rpm,
        };
        let floor = Descent::Floor { at_us: 0 };
        let ok = PowerLag::Within(MAX_T90_US);
        let v = |d: &Descent, held| stall_verdict(d, held, &ok).expect("verdict");
        let first = "stall below the release cadence: the release comes first";
        let late = "stall at or above the release cadence: the release comes too late";
        // Held settled up to 37 rpm: the trainer holds at 39 rpm at the latest.
        assert_eq!(v(&floor, Some(37)), first);
        assert!(v(&floor, Some(38)).starts_with("ambiguous"));
        assert!(v(&floor, None).starts_with("ambiguous"));
        // Power below 90 % at 41 rpm: the trainer lets go above 40 rpm.
        assert_eq!(v(&stalled(41), None), late);
        assert!(v(&stalled(40), None).starts_with("ambiguous"));
        assert_eq!(v(&stalled(30), Some(35)), first);
        assert!(v(&stalled(41), Some(30)).starts_with("inconsistent"));
        assert_eq!(stall_verdict(&Descent::Going, Some(30), &ok), None);
    }

    #[test]
    fn first_needs_check_6_t90_within_2_3_s() {
        assert_eq!(
            PowerLag::of_steps(&[Some(SEC), Some(2_300_000)]),
            PowerLag::Within(2_300_000)
        );
        assert_eq!(
            PowerLag::of_steps(&[Some(2_300_001), Some(SEC)]),
            PowerLag::TooSlow(Some(2_300_001))
        );
        assert_eq!(
            PowerLag::of_steps(&[Some(SEC), None]),
            PowerLag::TooSlow(None)
        );
        assert!(matches!(PowerLag::of_steps(&[]), PowerLag::NotMeasured(_)));
        assert!(matches!(
            PowerLag::of_check6(None),
            PowerLag::NotMeasured(_)
        ));
        assert!(matches!(
            PowerLag::of_check6(Some(&format!("{OUTCOME}aborted: x\n"))),
            PowerLag::NotMeasured(_)
        ));
        let floor = Descent::Floor { at_us: 0 };
        let late = Descent::Stalled {
            onset_us: 0,
            at_us: 0,
            dropped_at_rpm: 41,
        };
        let v = |d: &Descent, lag| stall_verdict(d, Some(37), &lag).expect("verdict");
        assert_eq!(
            v(&floor, PowerLag::Within(2_300_000)),
            "stall below the release cadence: the release comes first"
        );
        assert_eq!(
            v(&floor, PowerLag::of_check6(None)),
            "stall below the release cadence: the release comes first only if check 6 t90 ≤ 2.3 s (not measured)"
        );
        assert_eq!(
            v(&floor, PowerLag::TooSlow(Some(2_310_000))),
            "not valid: power response too slow (t90 = 2.31 s)"
        );
        assert_eq!(
            v(&floor, PowerLag::TooSlow(None)),
            "not valid: power response too slow (t90 = not reached)"
        );
        // Power below 90 % at 41 rpm needs no lag bound.
        assert_eq!(
            v(&late, PowerLag::TooSlow(None)),
            "stall at or above the release cadence: the release comes too late"
        );
    }

    #[test]
    fn a_gradual_fall_of_10_rpm_within_3_s_leaves_the_cue() {
        // 90 rpm, then 1 rpm lower every 0.25 s: no fall between two samples exceeds 1 rpm.
        for (to, leaves) in [(81, false), (80, true)] {
            let s = series(250_000, 6 * SEC, |t| {
                let k = u8::try_from(t.saturating_sub(SEC) / 250_000).expect("fits");
                (90_u8.saturating_sub(k).max(to), 250)
            });
            assert_eq!(departed(&descent(&s, 0, 250.0)), leaves, "90 -> {to}");
        }
    }

    #[test]
    fn a_stall_is_confirmed_after_exactly_2_s() {
        // Power below 90 % from 1 s; the samples 0.5 s apart reach 3 s exactly.
        let s = series(500_000, 3_000_001, |t| {
            (60, if t < SEC { 250 } else { 200 })
        });
        let s = into_descent(s, 60);
        assert_eq!(
            descent(&s, 0, 250.0),
            Descent::Stalled {
                onset_us: 61 * SEC,
                at_us: 63 * SEC,
                dropped_at_rpm: 60
            }
        );
        assert_eq!(descent(&s[..s.len() - 1], 0, 250.0), Descent::Going);
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
            check9(
                &Timeline::of_parts(&records, &[], &[]),
                &PowerLag::Within(MAX_T90_US)
            )["error"],
            json!("no page-49 write of 250 W")
        );
    }

    /// A page-25 frame of a synthetic check 12 run: `(at_us, cadence, watts, repeat)`; a repeat
    /// keeps the update event count of the frame before it (the NEO repeating its last update).
    type Frame12 = (u64, u8, u16, bool);

    /// Updates every `step_us` in [`from_us`, `to_us`) at `cadence` and `watts`.
    fn every(from_us: u64, to_us: u64, step_us: u64, cadence: u8, watts: u16) -> Vec<Frame12> {
        (from_us..to_us)
            .step_by(usize::try_from(step_us).expect("small"))
            .map(|at| (at, cadence, watts, false))
            .collect()
    }

    /// A synthetic check 12 run (ADR-0007 §2), its program and notes as `rider.rs` `check12`
    /// writes them: the opening release, the soft start to 200 W at 1 s, page 50 at 20 s, page 51
    /// at 20.5 s, the 70 rpm cue at `cue_us`, the answer at `answer_us`, the closing release at
    /// 66 s. By default: ERG 200 W at 90 rpm, 200 W at 90 rpm under `Free` (settled from 25.5 s),
    /// 150 W at 70 rpm from 40.25 s (settled from 45.25 s), and `y`: r = 0.75, PASS.
    struct Run12 {
        /// Before page 50.
        erg: Vec<Frame12>,
        /// From page 51 to the cue.
        free90: Vec<Frame12>,
        /// From the cue to the answer.
        free70: Vec<Frame12>,
        /// After the answer.
        after: Vec<Frame12>,
        cue_us: u64,
        answer_us: u64,
        answer: &'static str,
    }

    const MS: u64 = 1_000;

    impl Default for Run12 {
        fn default() -> Self {
            Self {
                erg: every(1_250 * MS, 20 * SEC, 250 * MS, 90, 200),
                free90: every(20_750 * MS, 40 * SEC, 250 * MS, 90, 200),
                free70: every(40_250 * MS, 60 * SEC, 250 * MS, 70, 150),
                after: every(60_250 * MS, 65 * SEC, 250 * MS, 70, 150),
                cue_us: 40 * SEC,
                answer_us: 60 * SEC,
                answer: "y",
            }
        }
    }

    impl Run12 {
        /// The run with its free70 updates set to `watts`, free90 to `free90` W.
        fn ratio(free90: u16, free70: u16, answer: &'static str) -> Self {
            Self {
                free90: every(20_750 * MS, 40 * SEC, 250 * MS, 90, free90),
                free70: every(40_250 * MS, 60 * SEC, 250 * MS, 70, free70),
                answer,
                ..Self::default()
            }
        }

        /// The capture text.
        fn text(&self) -> String {
            let mut frames: Vec<Frame12> = [&self.erg, &self.free90, &self.free70, &self.after]
                .into_iter()
                .flatten()
                .copied()
                .collect();
            frames.sort_by_key(|f| f.0);
            let mut count = 0_u8;
            let mut records: Vec<Record> = frames
                .iter()
                .map(|&(at_us, cadence, watts, repeat)| {
                    if !repeat {
                        count = count.wrapping_add(1);
                    }
                    let w = watts.to_le_bytes();
                    let mut f = [
                        0xa4, 9, 0x4e, 5, 25, count, cadence, 0, 0, w[0], w[1], 0x30, 0,
                    ];
                    f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
                    Record {
                        at_us,
                        rx: true,
                        bytes: f.to_vec(),
                    }
                })
                .collect();
            let free = ControlPage::BasicResistance { pct: 0.0 };
            records.extend([tx(0, &free), serial_request(100 * MS)]);
            records.extend(soft_start(SEC, f32::from(crate::rider::CHECK12_W)));
            records.extend([
                tx(
                    20 * SEC,
                    &ControlPage::Wind {
                        cw_kg_per_m: 0.51,
                        speed_kmh: 0.0,
                        drafting: 1.0,
                    },
                ),
                status_request(20_100 * MS),
                tx(
                    20_500 * MS,
                    &ControlPage::Track {
                        grade_pct: 0.0,
                        crr: 0.004,
                    },
                ),
                status_request(20_600 * MS),
                tx(66 * SEC, &free),
                status_request(66_300 * MS),
            ]);
            records.sort_by_key(|r| r.at_us);
            let note = |tag: &str, at_us: u64, bytes: Vec<u8>| crate::frames::Note {
                tag: tag.into(),
                at_us,
                bytes,
                before: 0,
            };
            let notes = [
                note(CUE12, self.cue_us, vec![TO_70]),
                note(
                    &crate::check10::answer_tag(EASIER.key),
                    self.answer_us,
                    self.answer.as_bytes().to_vec(),
                ),
            ];
            crate::timeline::text_of(&records, &[], &notes)
        }

        /// Check 12's row of the run, completed.
        fn row(&self) -> Value {
            let none = PowerLag::NotMeasured("test".into());
            analyse("check12", &self.text(), Some(COMPLETED), &none).expect("stem")
        }

        /// The row, asserted to read `want`.
        fn reads(&self, want: &str) -> Value {
            let v = self.row();
            assert!(
                v["line"].as_str().is_some_and(|l| l.contains(want)),
                "{want}: {}",
                v["line"]
            );
            v
        }
    }

    const PASS: &str = "PASS, released: r = ";
    const FAIL: &str = "FAIL, not released: r = ";
    const DISAGREE: &str = "no result: power and rider disagree";

    /// The synthetic run reads as the procedure's: completed, PASS at r = 0.75, each window
    /// counted from updates only.
    #[test]
    fn check12_a_synthetic_run_passes() {
        let v = Run12::default().reads(PASS);
        assert_eq!(v["completed"], json!(true), "{v}");
        assert_eq!(v["released"], json!(true), "{v}");
        assert_eq!(v["r"], json!(0.75), "{v}");
        assert_eq!(v["free90_updates"], json!(58), "{v}");
        assert_eq!(v["free70_updates"], json!(59), "{v}");
    }

    /// ADR-0007 §2's thresholds are inclusive: r = 0.85 with `y` is PASS, r = 0.855 is not; r = 0.95
    /// with `n` is FAIL, r = 0.945 is not.
    #[test]
    fn check12_r_at_0_85_passes_and_at_0_95_fails() {
        let v = Run12::ratio(200, 170, "y").reads(PASS);
        assert_eq!(v["r"], json!(0.85), "{v}");
        Run12::ratio(200, 171, "y").reads(DISAGREE);
        let v = Run12::ratio(200, 190, "n").reads(FAIL);
        assert_eq!(
            (v["r"].clone(), v["released"].clone()),
            (json!(0.95), json!(false))
        );
        Run12::ratio(200, 189, "n").reads(DISAGREE);
    }

    /// The median of an even count is the midpoint of the middle two: 180 and 200 W at 70 rpm
    /// read 190 W, r = 0.95, FAIL with `n`; and 160 and 180 W read 170 W, r = 0.85, PASS with `y`.
    #[test]
    fn check12_the_median_of_an_even_count_is_the_midpoint() {
        for (low, answer, want) in [(180, "n", FAIL), (160, "y", PASS)] {
            // 30 updates, 15 of each (the window starts 5 s after the first at 40.25 s).
            let mut free70 = every(40_250 * MS, 45_250 * MS, 250 * MS, 70, 150);
            free70.extend((0..30_u64).map(|k| {
                (
                    45_250 * MS + k * 250 * MS,
                    70,
                    low + 20 * u16::from(k % 2 == 1),
                    false,
                )
            }));
            let run = Run12 {
                free70,
                answer,
                ..Run12::default()
            };
            let v = run.reads(want);
            assert_eq!(v["free70_updates"], json!(30), "{v}");
        }
    }

    /// The median, not the mean: 9 of 59 updates at 70 rpm spiking to 2000 W leave r = 0.75.
    #[test]
    fn check12_a_minority_of_spikes_does_not_move_the_median() {
        let mut run = Run12::default();
        for f in run.free70.iter_mut().rev().take(9) {
            f.2 = 2000;
        }
        let v = run.reads(PASS);
        assert_eq!(v["r"], json!(0.75), "{v}");
    }

    /// ERG must have held 180–220 W, both ends inclusive, in the 5 s before page 50 only.
    #[test]
    fn check12_erg_holds_180_to_220_w() {
        for (watts, ok) in [
            (179, false),
            (180, true),
            (181, true),
            (219, true),
            (220, true),
            (221, false),
        ] {
            let run = Run12 {
                erg: every(1_250 * MS, 20 * SEC, 250 * MS, 90, watts),
                ..Run12::default()
            };
            let v = run.reads(if ok {
                PASS
            } else {
                "no result: ERG did not hold 180-220 W before Free"
            });
            assert_eq!(v["erg_w"], json!(f64::from(watts)), "{v}");
        }
        // Earlier than 5 s before page 50, and between page 50 and page 51, ERG is not read:
        // 55 updates at 100 W before 15 s and 40 at 100 W after page 50 are outvoted by none.
        let mut run = Run12 {
            erg: every(1_250 * MS, 15 * SEC, 250 * MS, 90, 100),
            ..Run12::default()
        };
        run.erg.extend(every(15 * SEC, 20 * SEC, 250 * MS, 90, 200));
        run.erg
            .extend(every(20 * SEC, 20_400 * MS, 10 * MS, 90, 100));
        let v = run.reads(PASS);
        assert_eq!(v["erg_w"], json!(200.0), "{v}");
    }

    /// Page-25 updates only (ADR-0007 §2): repeats of an update (count held) are not read. 20 of
    /// 41 updates at 190 W each repeated 3 times would make the median of all frames 190 W
    /// (r = 0.95, no result with `y`); of the updates it is 150 W (r = 0.75, PASS).
    #[test]
    fn check12_reads_updates_not_repeats() {
        let mut run = Run12 {
            free70: every(40_250 * MS, 45_250 * MS, 250 * MS, 70, 150),
            ..Run12::default()
        };
        for k in 0..41_u64 {
            let at = 45_250 * MS + k * 250 * MS;
            if k % 2 == 0 {
                run.free70.push((at, 70, 150, false));
            } else {
                run.free70.push((at, 70, 190, false));
                run.free70
                    .extend((1..=3).map(|j| (at + j * 50 * MS, 70, 190, true)));
            }
        }
        run.free70
            .extend(every(55_750 * MS, 60 * SEC, 250 * MS, 70, 150));
        let v = run.reads(PASS);
        assert_eq!(v["r"], json!(0.75), "{v}");
        assert_eq!(v["free70_updates"], json!(58), "{v}");
    }

    /// The bands are 85–95 and 65–75 rpm, both ends inclusive: updates at 84 and 96 rpm (100 W)
    /// and at 64 and 76 rpm (400 W), as many as the in-band ones, are not read.
    #[test]
    fn check12_reads_85_to_95_and_65_to_75_rpm() {
        let edges = |from: u64, cadences: [u8; 4], watts: [u16; 4]| -> Vec<Frame12> {
            (0..116_u64)
                .map(|k| {
                    let i = usize::try_from(k % 4).expect("small");
                    (from + k * 125 * MS, cadences[i], watts[i], false)
                })
                .collect()
        };
        let run = Run12 {
            free90: [
                every(20_750 * MS, 25_500 * MS, 250 * MS, 90, 200),
                edges(25_500 * MS, [84, 85, 95, 96], [100, 200, 200, 100]),
            ]
            .concat(),
            free70: [
                every(40_250 * MS, 45_250 * MS, 250 * MS, 70, 150),
                edges(45_250 * MS, [64, 65, 75, 76], [400, 150, 150, 400]),
            ]
            .concat(),
            ..Run12::default()
        };
        let v = run.reads(PASS);
        assert_eq!(v["r"], json!(0.75), "{v}");
        assert_eq!(
            (v["free90_updates"].clone(), v["free70_updates"].clone()),
            (json!(58), json!(58))
        );
    }

    /// The 70 rpm window ends at the answer's stamp: what comes after it (here 110 updates at
    /// 400 W) is not read.
    #[test]
    fn check12_the_70_rpm_window_ends_at_the_answer() {
        let run = Run12 {
            after: every(60_050 * MS, 65_550 * MS, 50 * MS, 70, 400),
            ..Run12::default()
        };
        let v = run.reads(PASS);
        assert_eq!(v["free70_updates"], json!(59), "{v}");
    }

    /// Both windows start 5 s after their event: after page 51 (95 updates at 160 W while the
    /// trainer settles) and after the first 70 rpm update (100 at 300 W), neither is read.
    #[test]
    fn check12_settles_5_s_after_page_51_and_after_reaching_70_rpm() {
        let run = Run12 {
            free90: [
                every(20_750 * MS, 25_500 * MS, 50 * MS, 90, 160),
                every(25_500 * MS, 40 * SEC, 250 * MS, 90, 200),
            ]
            .concat(),
            ..Run12::default()
        };
        let v = run.reads(PASS);
        assert_eq!(v["free90_w"], json!(200.0), "{v}");
        let run = Run12 {
            free70: [
                every(40_250 * MS, 45_250 * MS, 50 * MS, 70, 300),
                every(45_250 * MS, 60 * SEC, 250 * MS, 70, 150),
            ]
            .concat(),
            ..Run12::default()
        };
        let v = run.reads(PASS);
        assert_eq!(v["free70_w"], json!(150.0), "{v}");
    }

    /// t₇₀ is the first update in 65–75 rpm after the cue, not the first update: a rider who
    /// takes 4 s at 80 rpm, then 5 s at 70 rpm and 300 W before the power settles, reads PASS.
    #[test]
    fn check12_t70_is_the_first_update_in_the_band() {
        let run = Run12 {
            free70: [
                every(40_250 * MS, 44_250 * MS, 250 * MS, 80, 200),
                every(44_250 * MS, 49_250 * MS, 50 * MS, 70, 300),
                every(49_250 * MS, 60 * SEC, 250 * MS, 70, 150),
            ]
            .concat(),
            ..Run12::default()
        };
        let v = run.reads(PASS);
        assert_eq!(v["free70_updates"], json!(43), "{v}");
    }

    /// The cue counts only after page 51, the answer only after the cue and only as `y`, `n` or
    /// `s`: each otherwise is no result, for that reason.
    #[test]
    fn check12_the_cue_follows_page_51_and_the_answer_follows_the_cue() {
        let run = Run12 {
            cue_us: 20_200 * MS,
            ..Run12::default()
        };
        run.reads("no result: no 70 rpm cue stamp after page 51");
        let run = Run12 {
            answer_us: 39 * SEC,
            ..Run12::default()
        };
        run.reads("no result: the answer \"y\" is not one to the check 12 question after its cue");
        let run = Run12 {
            answer: "x",
            ..Run12::default()
        };
        run.reads("no result: the answer \"x\" is not one to the check 12 question after its cue");
        for (answer, want) in [
            ("y", PASS),
            ("n", DISAGREE),
            ("s", "no result: the rider shifted gear"),
        ] {
            Run12 {
                answer,
                ..Run12::default()
            }
            .reads(want);
        }
    }

    /// The power floor (ADR-0007 §2, [`FLOOR12_W`]): below 8 W in either window is no result.
    /// Without it, free90 = 0 W with free70 = 100 W and `n` read FAIL with r infinite; free70 = 0
    /// W on a held trainer with `y` read PASS with r = 0; and a road at 3 km/h (page 25 at 2 W in
    /// both windows) read FAIL with `n` at r = 1.
    #[test]
    fn check12_power_below_the_floor_is_no_result() {
        let floor = "no result: power under Free below the 8 W floor";
        for (free90, free70, answer) in [(0, 100, "n"), (200, 0, "y"), (2, 2, "n"), (10, 7, "y")] {
            let v = Run12::ratio(free90, free70, answer).reads(floor);
            assert_eq!(v["released"], Value::Null, "{v}");
        }
        // At the floor, r reads as ever: 10 W and 8 W, r = 0.8, PASS with `y`.
        let v = Run12::ratio(10, 8, "y").reads(PASS);
        assert_eq!(v["r"], json!(0.8), "{v}");
    }
}
