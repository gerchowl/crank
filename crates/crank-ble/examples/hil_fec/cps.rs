//! Part B check 5: FE-C page 25 against the trainer's Cycling Power Service measurement
//! (2A63) during a ride (#44 check 5, ADR-0002 §5 and §9; #412 part b).
//!
//! Like the other Part B results this is a pure function of the capture: the FE-C records and
//! the `# cps` comment lines ([`crate::frames::CPS_TAG`]) that the live run wrote beside them.
//! `docs/hil/part-b.md` states the method and the reasoning behind every tolerance.
use crate::{
    frames::{Cps, Record},
    partb::{SEC, SETTLE, Sample, controls, erg_holds, mean, procedure_end, round1, samples, secs},
};
use serde_json::{Value, json};

/// Per-second mean power must agree within this (ADR-0002 §5, HIL check 5).
pub const POWER_TOL_W: f64 = 1.0;
/// A hold's mean cadence must agree within this: page 25 rounds to 1 rpm (±0.5) and the two
/// streams are sampled at different instants (±0.5).
pub const CADENCE_TOL_RPM: f64 = 1.0;
/// Page 25 must arrive at least this often (ADR-0002 §5, HIL check 5).
pub const RATE_MIN_HZ: f64 = 2.0;
/// Inside a settled window page 25 may be silent for at most this long (ADR-0002 §6's 2 s).
const PAGE25_GAP_MAX_US: u64 = 2 * SEC;
/// Inside a settled window 2A63 may be silent for at most this long: three missed notifications
/// at its nominal 1 Hz.
const CPS_GAP_MAX_US: u64 = 3 * SEC;
/// A window needs at least `COVERAGE_NUM / COVERAGE_DEN` of its whole seconds with both streams.
const COVERAGE_NUM: usize = 3;
const COVERAGE_DEN: usize = 4;
/// ... and at least this many seconds in absolute terms: a verdict needs a ride, not a glimpse.
const MIN_WINDOW_SECONDS: usize = 10;
/// More than this share of malformed 2A63 notifications leaves no verdict.
const MALFORMED_MAX_PERCENT: usize = 10;
/// Two CPS notifications further apart than this are not paired for cadence: the 16-bit
/// event time wraps every 64 s, so a longer gap could hide a whole wrap.
const PAIR_MAX_US: u64 = 32 * SEC;
/// Cadence above this is not a rider (page 25 itself ends at 254).
const MAX_RPM: f64 = 255.0;

/// A decoded Cycling Power Measurement (Bluetooth SIG 2A63).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measurement {
    /// Instantaneous power, W.
    pub power_w: i16,
    /// Cumulative crank revolutions and last crank event time (1/1024 s), both 16 bit.
    pub crank: Option<(u16, u16)>,
}

fn le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

/// Decode a 2A63 value; `None` when it is shorter than its flags say.
pub fn decode(b: &[u8]) -> Option<Measurement> {
    let flags = le(b, 0)?;
    let power_w = i16::from_le_bytes(le(b, 2)?.to_le_bytes());
    // Optional fields before the crank data: pedal power balance (bit 0, 1 byte), accumulated
    // torque (bit 2, 2 bytes), wheel revolution data (bit 4, 6 bytes).
    let mut at = 4;
    for (bit, len) in [(0x01, 1), (0x04, 2), (0x10, 6)] {
        if flags & bit != 0 {
            at += len;
        }
    }
    let crank = if flags & 0x20 != 0 {
        Some((le(b, at)?, le(b, at + 2)?))
    } else {
        None
    };
    Some(Measurement { power_w, crank })
}

struct Reading {
    at_us: u64,
    power_w: f64,
}

/// Crank revolutions over the crank-event time between two notifications.
struct Pair {
    at_us: u64,
    revs: f64,
    secs: f64,
}

#[derive(Default)]
struct Parsed {
    readings: Vec<Reading>,
    pairs: Vec<Pair>,
    malformed: usize,
    rejected: usize,
}

/// Decode the notifications and pair consecutive crank events. Both counters wrap at 65 536,
/// so differences are taken modulo 2^16; a notification without a new crank event (same event
/// time) adds a power reading only.
fn parse(cps: &[Cps]) -> Parsed {
    let mut sorted: Vec<&Cps> = cps.iter().collect();
    sorted.sort_by_key(|c| c.at_us);
    let mut out = Parsed::default();
    let mut prev: Option<(u64, u16, u16)> = None;
    for c in sorted {
        let Some(m) = decode(&c.bytes) else {
            out.malformed += 1;
            continue;
        };
        out.readings.push(Reading {
            at_us: c.at_us,
            power_w: f64::from(m.power_w),
        });
        let Some((revs, time)) = m.crank else {
            continue;
        };
        let Some((at, p_revs, p_time)) = prev else {
            prev = Some((c.at_us, revs, time));
            continue;
        };
        let dt = time.wrapping_sub(p_time);
        if dt == 0 {
            continue;
        }
        let (dr, secs) = (f64::from(revs.wrapping_sub(p_revs)), f64::from(dt) / 1024.0);
        if c.at_us - at > PAIR_MAX_US || 60.0 * dr / secs > MAX_RPM {
            out.rejected += 1;
        } else {
            out.pairs.push(Pair {
                at_us: c.at_us,
                revs: dr,
                secs,
            });
        }
        prev = Some((c.at_us, revs, time));
    }
    out
}

/// The whole seconds `[from, to)` contains, as capture-second indices.
fn whole_seconds(from: u64, to: u64) -> std::ops::Range<u64> {
    from.div_ceil(SEC)..to / SEC
}

fn in_window(at: u64, from: u64, to: u64) -> bool {
    at >= from && at < to
}

/// One settled window: a page-49 target after its first 10 s, and how well both streams cover it.
struct Window {
    watts: f64,
    from: u64,
    to: u64,
    whole: usize,
    compared: usize,
    page25_gap: u64,
    cps_gap: u64,
    /// Mean page-25 cadence and the CPS cadence over the window, rpm.
    cadence: Option<(f64, f64)>,
}

impl Window {
    fn label(&self) -> String {
        format!(
            "{} W target ({}–{} s)",
            self.watts,
            round1(secs(self.from)),
            round1(secs(self.to))
        )
    }
}

/// Longest stretch of `[from, to)` without a timestamp in `times`, the window's edges included.
fn max_gap(mut times: Vec<u64>, from: u64, to: u64) -> u64 {
    times.sort_unstable();
    let mut prev = from;
    let mut gap = 0;
    for t in times.into_iter().filter(|t| in_window(*t, from, to)) {
        gap = gap.max(t - prev);
        prev = t;
    }
    gap.max(to.saturating_sub(prev))
}

/// Enough of a window's whole seconds were compared (`COVERAGE_NUM` / `COVERAGE_DEN` of them,
/// and at least `MIN_WINDOW_SECONDS`).
fn covered(compared: usize, whole: usize) -> bool {
    compared >= MIN_WINDOW_SECONDS && compared * COVERAGE_DEN >= whole * COVERAGE_NUM
}

/// The comparison inside one settled window; `seconds` collects the per-second `(page 25, 2A63)`
/// mean powers.
fn compare_window(
    s: &[Sample],
    parsed: &Parsed,
    (watts, from, to): (f64, u64, u64),
    seconds: &mut Vec<(f64, f64)>,
) -> Window {
    let (mut whole, mut compared) = (0, 0);
    for sec in whole_seconds(from, to) {
        let (lo, hi) = (sec * SEC, (sec + 1) * SEC);
        let fe = mean(
            s.iter()
                .filter(|x| in_window(x.at_us, lo, hi))
                .filter_map(|x| x.power.map(f64::from)),
        );
        let cp = mean(
            parsed
                .readings
                .iter()
                .filter(|r| in_window(r.at_us, lo, hi))
                .map(|r| r.power_w),
        );
        whole += 1;
        if let (Some(fe), Some(cp)) = (fe, cp) {
            seconds.push((fe, cp));
            compared += 1;
        }
    }
    let fe = mean(
        s.iter()
            .filter(|x| in_window(x.at_us, from, to))
            .filter_map(|x| x.cadence.map(f64::from)),
    );
    let (revs, time) = parsed
        .pairs
        .iter()
        .filter(|p| in_window(p.at_us, from, to))
        .fold((0.0, 0.0), |(r, t), p| (r + p.revs, t + p.secs));
    Window {
        watts,
        from,
        to,
        whole,
        compared,
        page25_gap: max_gap(
            s.iter()
                .filter(|x| x.power.is_some())
                .map(|x| x.at_us)
                .collect(),
            from,
            to,
        ),
        cps_gap: max_gap(parsed.readings.iter().map(|r| r.at_us).collect(), from, to),
        cadence: fe.filter(|_| time > 0.0).map(|fe| (fe, 60.0 * revs / time)),
    }
}

/// Check 5: page-25 rate, and page-25 against CPS power and cadence over the settled holds.
pub fn check5(records: &[Record], cps: &[Cps]) -> Value {
    let s = samples(records);
    let ctl = controls(records);
    let parsed = parse(cps);
    // Settled windows: every ERG target held ≥ 10 s (check 6's definition of a hold), after its
    // first 10 s, and never the 0 W wait for the rider.
    let spans: Vec<(f64, u64, u64)> = erg_holds(&ctl, procedure_end(records, &ctl))
        .into_iter()
        .filter(|(held, h)| *held && h.watts > 0.0)
        .map(|(_, h)| (h.watts, h.at + SETTLE, h.end))
        .collect();

    let mut seconds = Vec::new();
    let windows: Vec<Window> = spans
        .iter()
        .map(|&span| compare_window(&s, &parsed, span, &mut seconds))
        .collect();

    let diffs: Vec<f64> = seconds.iter().map(|(a, b)| a - b).collect();
    let outside = diffs
        .iter()
        .filter(|d| d.abs() > POWER_TOL_W + 1e-9)
        .count();
    let max_abs = diffs.iter().fold(0.0_f64, |m, d| m.max(d.abs()));
    let cadence_max = windows
        .iter()
        .filter_map(|w| w.cadence)
        .fold(0.0_f64, |m, (a, b)| m.max((a - b).abs()));

    let times: Vec<u64> = s.iter().map(|x| x.at_us).collect();
    #[allow(clippy::cast_precision_loss)]
    let rate_hz = times
        .first()
        .zip(times.last())
        .filter(|(f, l)| l > f)
        .map(|(f, l)| (times.len() - 1) as f64 / secs(l - f));
    let max_gap_s = times.windows(2).map(|w| secs(w[1] - w[0])).reduce(f64::max);

    let verdict = verdict(&parsed, rate_hz, &windows, (outside, max_abs, cadence_max));
    json!({
        "method": "both streams are stamped on arrival by the same process clock, so no offset is estimated; settled windows = each page-49 target held >= 10 s, after its first 10 s (power is steady under ERG, so the residual jitter of the two notification streams is not mistaken for disagreement); coverage: every ERG target of the procedure needs its window, in each window both streams must have no gap above 2 s (page 25) / 3 s (2A63) and at least 3/4 of its whole seconds must hold a sample of each, else the verdict names the target and is insufficient; more than 10 % malformed 2A63 notifications is insufficient; power: for each whole second of a window with at least one sample of each stream, mean page-25 power minus mean 2A63 power, which must be within ±1 W (ADR-0002 §5) in every compared second; cadence: per window, mean page-25 cadence against 2A63 crank revolutions over crank event time (differences modulo 2^16, event time in 1/1024 s), within ±1 rpm; page 25 must arrive at >= 2 Hz",
        "tolerances": {
            "power_w": POWER_TOL_W, "cadence_rpm": CADENCE_TOL_RPM, "page25_min_hz": RATE_MIN_HZ,
            "page25_gap_max_s": secs(PAGE25_GAP_MAX_US), "cps_gap_max_s": secs(CPS_GAP_MAX_US),
            "coverage_share": 0.75, "min_window_seconds": MIN_WINDOW_SECONDS,
            "malformed_max_percent": MALFORMED_MAX_PERCENT,
        },
        "page25": {"samples": s.len(), "rate_hz": rate_hz.map(round1), "max_gap_s": max_gap_s.map(round1)},
        "cps": {
            "notifications": cps.len(),
            "decoded": parsed.readings.len(),
            "malformed": parsed.malformed,
            "crank_pairs": parsed.pairs.len(),
            "rejected_pairs": parsed.rejected,
        },
        "windows": windows.iter().map(|w| json!({
            "target_w": w.watts,
            "from_s": round1(secs(w.from)),
            "to_s": round1(secs(w.to)),
            "seconds": w.whole,
            "seconds_compared": w.compared,
            "page25_max_gap_s": round1(secs(w.page25_gap)),
            "cps_max_gap_s": round1(secs(w.cps_gap)),
        })).collect::<Vec<_>>(),
        "seconds_compared": seconds.len(),
        "seconds_within_tolerance": seconds.len() - outside,
        "mean_page25_w": mean(seconds.iter().map(|x| x.0)).map(round1),
        "mean_cps_w": mean(seconds.iter().map(|x| x.1)).map(round1),
        "bias_w": mean(diffs.iter().copied()).map(|x| (x * 100.0).round() / 100.0),
        "max_abs_diff_w": (!diffs.is_empty()).then_some(round1(max_abs)),
        "cadence_windows": windows.iter().filter_map(|w| w.cadence).map(|(a, b)| json!({"page25_rpm": round1(a), "cps_rpm": round1(b)})).collect::<Vec<_>>(),
        "max_abs_cadence_diff_rpm": windows.iter().any(|w| w.cadence.is_some()).then_some(round1(cadence_max)),
        "verdict": verdict,
    })
}

/// The verdict: `insufficient` unless every target is covered by both streams, else `agree` or
/// `disagree`. `(outside, max_abs, cadence_max)` are the seconds outside ±1 W, the largest
/// power difference and the largest cadence difference.
fn verdict(
    parsed: &Parsed,
    rate_hz: Option<f64>,
    windows: &[Window],
    (outside, max_abs, cadence_max): (usize, f64, f64),
) -> String {
    if parsed.readings.is_empty() {
        return format!(
            "no CPS: no decodable 2A63 notification in the capture ({} malformed)",
            parsed.malformed
        );
    }
    let total = parsed.readings.len() + parsed.malformed;
    if parsed.malformed * 100 > MALFORMED_MAX_PERCENT * total {
        return format!(
            "insufficient: {} of {total} 2A63 notifications malformed (> {MALFORMED_MAX_PERCENT} %)",
            parsed.malformed
        );
    }
    let mut gaps = Vec::new();
    if windows.len() < crate::rider::CHECK5_W.len() {
        gaps.push(format!(
            "{} settled windows, one per ERG target ({}) needed",
            windows.len(),
            crate::rider::CHECK5_W.len()
        ));
    }
    for w in windows {
        let what = w.label();
        if w.page25_gap > PAGE25_GAP_MAX_US {
            gaps.push(format!(
                "{what}: no page 25 for {} s (> {} s)",
                round1(secs(w.page25_gap)),
                secs(PAGE25_GAP_MAX_US)
            ));
        }
        if w.cps_gap > CPS_GAP_MAX_US {
            gaps.push(format!(
                "{what}: no 2A63 for {} s (> {} s)",
                round1(secs(w.cps_gap)),
                secs(CPS_GAP_MAX_US)
            ));
        }
        if !covered(w.compared, w.whole) {
            gaps.push(format!(
                "{what}: only {} of {} seconds hold both streams",
                w.compared, w.whole
            ));
        }
        if w.cadence.is_none() {
            gaps.push(format!(
                "{what}: no 2A63 crank-revolution data to derive cadence"
            ));
        }
    }
    if !gaps.is_empty() {
        return format!("insufficient: {}", gaps.join("; "));
    }
    let mut fails = Vec::new();
    if !rate_hz.is_some_and(|r| r >= RATE_MIN_HZ) {
        fails.push(format!(
            "page 25 rate {} Hz < {RATE_MIN_HZ} Hz",
            rate_hz.map_or("–".into(), |r| format!("{:.1}", round1(r)))
        ));
    }
    if outside > 0 {
        fails.push(format!(
            "power: {outside} seconds outside ±{POWER_TOL_W} W (max {} W)",
            round1(max_abs)
        ));
    }
    if cadence_max > CADENCE_TOL_RPM + 1e-9 {
        fails.push(format!(
            "cadence: {} rpm apart (> ±{CADENCE_TOL_RPM} rpm)",
            round1(cadence_max)
        ));
    }
    if fails.is_empty() {
        "agree".into()
    } else {
        format!("disagree: {}", fails.join("; "))
    }
}

/// The result cell of check 5's row in `docs/hil/<date>-fec.md`.
pub fn result(v: &Value) -> String {
    let n = |x: &Value| x.as_f64().map_or_else(|| "–".into(), |x| format!("{x}"));
    format!(
        "page 25 {} Hz; {} s compared, bias {} W, max |Δ| {} W; cadence max |Δ| {} rpm: {}",
        n(&v["page25"]["rate_hz"]),
        v["seconds_compared"],
        n(&v["bias_w"]),
        n(&v["max_abs_diff_w"]),
        n(&v["max_abs_cadence_diff_rpm"]),
        v["verdict"].as_str().unwrap_or("?"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cps(at_us: u64, power: i16, revs: u16, time: u16) -> Cps {
        let mut bytes = vec![0x20, 0x00];
        bytes.extend(power.to_le_bytes());
        bytes.extend(revs.to_le_bytes());
        bytes.extend(time.to_le_bytes());
        Cps { at_us, bytes }
    }

    #[test]
    fn decodes_optional_fields_in_order() {
        // flags: balance (1 B) + accumulated torque (2 B) + wheel (6 B) + crank (4 B)
        let mut b = vec![0x35, 0x00, 0x9c, 0xff, 0xaa, 1, 2];
        b.extend([0; 6]);
        b.extend([0x34, 0x12, 0x78, 0x56]);
        assert_eq!(
            decode(&b),
            Some(Measurement {
                power_w: -100,
                crank: Some((0x1234, 0x5678))
            })
        );
        assert_eq!(
            decode(&[0, 0, 150, 0]),
            Some(Measurement {
                power_w: 150,
                crank: None
            })
        );
        assert_eq!(
            decode(&[0x20, 0, 150, 0, 1, 0, 2]),
            None,
            "short crank data"
        );
        assert_eq!(decode(&[0, 0, 1]), None);
    }

    #[test]
    fn cadence_pairs_survive_both_counters_wrapping() {
        // 90 rpm: a crank event every 683 ticks of 1/1024 s; revolutions wrap after 65 535,
        // event time after 65 535 ticks (64 s).
        let p = parse(&[
            cps(0, 100, 65_534, 64_800),
            cps(SEC, 100, 65_535, 65_483),
            cps(2 * SEC, 100, 0, 630),
            cps(3 * SEC, 100, 1, 1_313),
        ]);
        assert_eq!((p.pairs.len(), p.rejected), (3, 0));
        let (revs, time) = p
            .pairs
            .iter()
            .fold((0.0, 0.0), |(r, t), x| (r + x.revs, t + x.secs));
        assert!(
            (60.0 * revs / time - 90.0).abs() < 0.1,
            "{}",
            60.0 * revs / time
        );
    }

    #[test]
    fn no_new_crank_event_adds_power_only_and_a_stale_gap_is_rejected() {
        let p = parse(&[
            cps(0, 100, 10, 1000),
            cps(SEC, 101, 10, 1000),
            cps(2 * SEC, 102, 11, 1700),
            cps(40 * SEC, 103, 14, 3748),
        ]);
        assert_eq!((p.readings.len(), p.pairs.len(), p.rejected), (4, 1, 1));
    }

    #[test]
    fn an_impossible_cadence_is_rejected_not_averaged() {
        let p = parse(&[cps(0, 1, 0, 0), cps(SEC, 1, 100, 1024)]);
        assert_eq!((p.pairs.len(), p.rejected), (0, 1));
    }

    /// A Part B capture of a steady 150 W hold: page 25 every `every_us`, one CPS reading a
    /// second `offset_w` away, `secs` long.
    fn ride(every_us: u64, offset_w: i16, secs: u64) -> (Vec<Record>, Vec<Cps>) {
        use crank_fec::{ControlPage, encode};
        let erg = |at_us, watts| Record {
            at_us,
            rx: false,
            bytes: encode(&ControlPage::TargetPower { watts })
                .expect("encodes")
                .to_vec(),
        };
        let mut records = vec![erg(0, 150.0), erg(secs * SEC / 2, 200.0)];
        let mut at = 0;
        while at < secs * SEC {
            let mut f = [0xa4, 9, 0x4e, 5, 25, 0, 90, 0, 0, 150, 0, 0x30, 0];
            f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
            records.push(Record {
                at_us: at,
                rx: true,
                bytes: f.to_vec(),
            });
            at += every_us;
        }
        let cps = (0..secs)
            .map(|k| {
                cps(
                    k * SEC + 400_000,
                    150 + offset_w,
                    u16::try_from(k * 3 / 2).expect("fits"),
                    u16::try_from(k * 1024).expect("fits"),
                )
            })
            .collect();
        (records, cps)
    }

    #[test]
    fn page_25_slower_than_2_hz_disagrees_even_when_the_power_matches() {
        let (records, cps) = ride(1_000_000, 0, 60);
        let v = check5(&records, &cps);
        let verdict = v["verdict"].as_str().expect("verdict");
        assert!(verdict.starts_with("disagree: page 25 rate 1.0 Hz"), "{v}");
        let (records, cps) = ride(500_000, 0, 60);
        assert_eq!(check5(&records, &cps)["verdict"], "agree");
    }

    #[test]
    fn too_short_a_ride_has_no_verdict() {
        let (records, cps) = ride(250_000, 0, 25);
        let v = check5(&records, &cps);
        assert!(
            v["verdict"]
                .as_str()
                .is_some_and(|x| x.starts_with("insufficient: ")),
            "{v}"
        );
    }

    #[test]
    fn a_cadence_beyond_1_rpm_disagrees_and_within_it_agrees() {
        let (records, _) = ride(250_000, 0, 60);
        // One notification per revolution at `rpm`; the event time follows from the revolutions.
        let at_rpm = |rpm: f64| -> Vec<Cps> {
            (0..200_u32)
                .map(|k| f64::from(k) * 60.0 / rpm)
                .take_while(|t| *t < 60.0)
                .enumerate()
                .map(|(k, t)| {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let (at, ticks) = ((t * 1e6) as u64 + 400_000, (t * 1024.0).round() as u16);
                    cps(at, 150, u16::try_from(k).expect("fits"), ticks)
                })
                .collect()
        };
        let verdict = |rpm| check5(&records, &at_rpm(rpm))["verdict"].clone();
        assert_eq!(verdict(90.0), "agree");
        assert_eq!(verdict(89.2), "agree", "within 1 rpm");
        let off = verdict(75.0);
        assert!(
            off.as_str()
                .is_some_and(|x| x.starts_with("disagree: cadence: 15")),
            "{off}"
        );
        let off = verdict(88.0);
        assert!(
            off.as_str()
                .is_some_and(|x| x.starts_with("disagree: cadence: 2")),
            "{off}"
        );
    }

    #[test]
    fn coverage_needs_three_quarters_and_ten_seconds() {
        assert!(covered(10, 10) && covered(15, 20));
        assert!(!covered(14, 20), "under 3/4");
        assert!(!covered(9, 9), "under 10 s whatever the share");
        assert!(covered(10, 13) && !covered(10, 14));
    }

    #[test]
    fn a_gap_is_measured_from_the_edges_of_the_window() {
        assert_eq!(max_gap(vec![], 10, 30), 20);
        assert_eq!(max_gap(vec![12, 13, 29], 10, 30), 16);
        assert_eq!(
            max_gap(vec![5, 11, 14, 35], 10, 30),
            16,
            "outside samples do not count"
        );
    }

    #[test]
    fn the_wait_for_the_rider_at_0_w_is_no_target() {
        use crank_fec::{ControlPage, encode};
        let (mut records, cps) = ride(250_000, 0, 60);
        // The first hold is the 0 W wait for the rider, not a target: one target is left.
        records[0].bytes = encode(&ControlPage::TargetPower { watts: 0.0 })
            .expect("encodes")
            .to_vec();
        let v = check5(&records, &cps);
        assert_eq!(v["windows"].as_array().map(Vec::len), Some(1), "{v}");
        assert!(
            v["verdict"]
                .as_str()
                .is_some_and(|x| x.starts_with("insufficient: 1 settled windows")),
            "{v}"
        );
    }
}
