//! Offline analysis of the Part B control checks (#44 checks 6, 7, 12; #412).
//!
//! Every result is a pure function of the `.fecap` records and the capture's outcome line, so
//! `analyse` re-derives it from a committed capture: the procedure's own timing is read back
//! from its tx writes, the rider's from page 25 (power, cadence). Only a run whose procedure
//! completed gives a result; any other run (or a capture without an outcome line) reports
//! "no result" and no verdict.
use crate::frames::{Record, page_of};
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
        "check12" => check12(records),
        _ => return None,
    };
    let completed = outcome == Some(COMPLETED);
    v["procedure"] = json!(outcome.unwrap_or("unknown: the capture has no outcome line"));
    v["completed"] = json!(completed);
    if !completed {
        for verdict in ["held", "released"] {
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
    let empty = Vec::new();
    let list = |key: &str| v[key].as_array().unwrap_or(&empty).clone();
    let no_result = (v["completed"] != json!(true)).then(|| {
        format!(
            "no result: procedure {}",
            v["procedure"].as_str().unwrap_or("unknown")
        )
    });
    let (n, what, result, adr) = match stem {
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
    };
    let result = no_result.unwrap_or(result);
    let release = if v["release"]["page48_0pct_last"] == json!(true) {
        "released"
    } else {
        "NOT released"
    };
    format!("| {n} | {what} | {result} (trainer {release} at the end) | {adr} |")
}
