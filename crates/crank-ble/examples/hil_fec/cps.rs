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
/// A settled window must be at least this long: a verdict needs a ride, not a glimpse.
const MIN_WINDOW_SECONDS: u64 = 10;
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
    /// Arrival of the notification the pair starts at.
    from_us: u64,
    /// Arrival of the notification the pair ends at.
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
                from_us: at,
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
    /// Longest stretch of the window without a page-25 sample that carries a cadence.
    cadence_gap: u64,
    /// Longest stretch of the window not covered by a crank-revolution pair lying inside it.
    crank_gap: u64,
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

/// Longest stretch of `[from, to)` not covered by a pair lying wholly inside it: the cadence of
/// a window may only come from revolutions counted between two notifications of the window.
fn crank_gap(pairs: &[&Pair], from: u64, to: u64) -> u64 {
    let mut sorted: Vec<&&Pair> = pairs.iter().collect();
    sorted.sort_by_key(|p| p.from_us);
    let (mut prev, mut gap) = (from, 0);
    for p in sorted {
        gap = gap.max(p.from_us.saturating_sub(prev));
        prev = prev.max(p.at_us);
    }
    gap.max(to.saturating_sub(prev))
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
    let inside: Vec<&Pair> = parsed
        .pairs
        .iter()
        .filter(|p| p.from_us >= from && p.at_us < to)
        .collect();
    let (revs, time) = inside
        .iter()
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
        cadence_gap: max_gap(
            s.iter()
                .filter(|x| x.cadence.is_some())
                .map(|x| x.at_us)
                .collect(),
            from,
            to,
        ),
        crank_gap: crank_gap(&inside, from, to),
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
        "method": "both streams are stamped on arrival by the same process clock, so no offset is estimated; settled windows = each page-49 target held >= 10 s, after its first 10 s (power is steady under ERG, so the residual jitter of the two notification streams is not mistaken for disagreement); coverage, in continuous time: every ERG target of the procedure needs its window (>= 10 s); inside it neither stream may be silent for more than 2 s (page 25, power and cadence each) / 3 s (2A63), window edges included, and the crank-revolution pairs lying wholly inside the window must leave no stretch of it uncovered by more than 3 s, else the verdict names the target and is insufficient; more than 10 % malformed 2A63 notifications is insufficient; power: for each whole second of a window with at least one sample of each stream, mean page-25 power minus mean 2A63 power, which must be within ±1 W (ADR-0002 §5) in every compared second; cadence: per window, mean page-25 cadence against 2A63 crank revolutions over crank event time (differences modulo 2^16, event time in 1/1024 s), within ±1 rpm; page 25 must arrive at >= 2 Hz",
        "tolerances": {
            "power_w": POWER_TOL_W, "cadence_rpm": CADENCE_TOL_RPM, "page25_min_hz": RATE_MIN_HZ,
            "page25_gap_max_s": secs(PAGE25_GAP_MAX_US), "cps_gap_max_s": secs(CPS_GAP_MAX_US),
            "min_window_seconds": MIN_WINDOW_SECONDS,
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
            "page25_cadence_max_gap_s": round1(secs(w.cadence_gap)),
            "crank_max_gap_s": round1(secs(w.crank_gap)),
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

/// The verdict: `insufficient` unless every target is covered, in continuous time, by both streams
/// and by crank data, else `agree` or
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
        if w.to - w.from < MIN_WINDOW_SECONDS * SEC {
            gaps.push(format!("{what}: shorter than {MIN_WINDOW_SECONDS} s"));
        }
        if w.compared == 0 {
            gaps.push(format!("{what}: no second holds both streams"));
        }
        if w.cadence_gap > PAGE25_GAP_MAX_US {
            gaps.push(format!(
                "{what}: no page-25 cadence for {} s (> {} s)",
                round1(secs(w.cadence_gap)),
                secs(PAGE25_GAP_MAX_US)
            ));
        }
        if w.crank_gap > CPS_GAP_MAX_US {
            gaps.push(format!(
                "{what}: no 2A63 crank-revolution data for {} s (> {} s) inside the window",
                round1(secs(w.crank_gap)),
                secs(CPS_GAP_MAX_US)
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
            .map(|k| cps_at(k * SEC + 400_000, k * SEC + 400_000, 150 + offset_w))
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

    // ---- coverage in continuous time ----

    fn erg(at_us: u64, watts: f32) -> Record {
        use crank_fec::{ControlPage, encode};
        Record {
            at_us,
            rx: false,
            bytes: encode(&ControlPage::TargetPower { watts })
                .expect("encodes")
                .to_vec(),
        }
    }

    /// A page-25 frame at 90 rpm and 150 W (power unavailable when asked).
    fn fe_frame(at_us: u64, power_unavailable: bool) -> Record {
        fe_frame_with(at_us, power_unavailable, false)
    }

    fn fe_frame_with(at_us: u64, power_unavailable: bool, cadence_unavailable: bool) -> Record {
        let mut f = [0xa4, 9, 0x4e, 5, 25, 0, 90, 0, 0, 150, 0, 0x30, 0];
        if power_unavailable {
            (f[9], f[10]) = (0xff, 0x0f);
        }
        if cadence_unavailable {
            f[6] = 0xff;
        }
        f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
        Record {
            at_us,
            rx: true,
            bytes: f.to_vec(),
        }
    }

    /// The 2A63 notification a 90 rpm rider's trainer sends at `nominal` (a crank event every
    /// 2/3 s, event time in 1/1024 s), arriving at `arrival`.
    fn cps_at(arrival: u64, nominal: u64, power: i16) -> Cps {
        let events = nominal * 3 / 2 / SEC;
        let ticks = (events * 2048 + 1) / 3;
        cps(
            arrival,
            power,
            u16::try_from(events % 65_536).expect("fits"),
            u16::try_from(ticks % 65_536).expect("fits"),
        )
    }

    /// `step`-spaced times in `[first, 60 s)`.
    fn grid(step: u64, first: u64) -> Vec<u64> {
        (0..=60 * SEC / step)
            .map(|k| first + k * step)
            .filter(|t| *t < 60 * SEC)
            .collect()
    }

    /// `times` with a hole: nothing in `(a, a + gap)`, a sample at exactly `a + gap`.
    fn holed(times: &[u64], a: u64, gap: u64) -> Vec<u64> {
        times
            .iter()
            .copied()
            .filter(|t| *t <= a)
            .chain([a + gap])
            .chain(times.iter().copied().filter(|t| *t > a + gap))
            .collect()
    }

    /// Check 5's verdict of a 60 s ride (150 W, then 200 W from 30 s) with these streams.
    fn verdict_of(fe: Vec<Record>, cp: &[Cps]) -> String {
        let mut records = vec![erg(0, 150.0), erg(30 * SEC, 200.0)];
        records.extend(fe);
        check5(&records, cp)["verdict"]
            .as_str()
            .expect("verdict")
            .to_string()
    }

    fn fe_of(times: &[u64]) -> Vec<Record> {
        times.iter().map(|t| fe_frame(*t, false)).collect()
    }

    fn cps_of(times: &[u64]) -> Vec<Cps> {
        times.iter().map(|t| cps_at(*t, *t, 150)).collect()
    }

    fn insufficient(v: &str, what: &str) -> bool {
        v.starts_with("insufficient: ") && v.contains(what)
    }

    #[test]
    fn the_page_25_gap_bound_is_2_s_exactly() {
        let (fe, cp) = (grid(250_000, 0), cps_of(&grid(SEC, 400_000)));
        assert_eq!(verdict_of(fe_of(&fe), &cp), "agree");
        let at = verdict_of(fe_of(&holed(&fe, 15 * SEC, 2 * SEC)), &cp);
        assert_eq!(at, "agree", "a gap of exactly 2 s is within the bound");
        let over = verdict_of(fe_of(&holed(&fe, 15 * SEC, 2 * SEC + 1)), &cp);
        assert!(insufficient(&over, "no page 25 for"), "{over}");
    }

    #[test]
    fn the_2a63_gap_bound_is_3_s_exactly() {
        let (fe, cp) = (grid(250_000, 0), grid(SEC, 400_000));
        let at = verdict_of(
            fe_of(&fe),
            &cps_of(&holed(&cp, 15 * SEC + 400_000, 3 * SEC)),
        );
        assert_eq!(at, "agree", "a gap of exactly 3 s is within the bound");
        let over = verdict_of(
            fe_of(&fe),
            &cps_of(&holed(&cp, 15 * SEC + 400_000, 3 * SEC + 1)),
        );
        assert!(insufficient(&over, "no 2A63 for"), "{over}");
    }

    #[test]
    fn page_25_samples_without_power_do_not_cover_a_gap() {
        let (fe, cp) = (grid(250_000, 0), cps_of(&grid(SEC, 400_000)));
        let blind: Vec<Record> = fe
            .iter()
            .map(|t| fe_frame(*t, (15 * SEC..20 * SEC).contains(t)))
            .collect();
        let v = verdict_of(blind, &cp);
        assert!(insufficient(&v, "no page 25 for"), "{v}");
    }

    #[test]
    fn the_malformed_bound_is_10_percent_exactly() {
        let (fe, times) = (grid(250_000, 0), grid(SEC, 400_000));
        let cut = |n: usize| -> Vec<Cps> {
            let mut cp = cps_of(&times);
            for i in &[5, 15, 25, 35, 45, 55, 20][..n] {
                cp[*i].bytes.truncate(5);
            }
            cp
        };
        assert_eq!(
            verdict_of(fe_of(&fe), &cut(6)),
            "agree",
            "6 of 60 is exactly 10 %"
        );
        let v = verdict_of(fe_of(&fe), &cut(7));
        assert!(insufficient(&v, "malformed"), "{v}");
    }

    #[test]
    fn cadence_needs_crank_pairs_inside_the_window_not_one_that_reaches_back() {
        let (fe, times) = (grid(250_000, 0), grid(SEC, 400_000));
        let mut cp = cps_of(&times);
        // No crank data from 33.5 s to 59 s: window 2 (40–60 s) has it in its last notification
        // only, whose pair would start about 25 s before the window.
        for c in cp
            .iter_mut()
            .filter(|c| c.at_us >= 33_500_000 && c.at_us < 59 * SEC)
        {
            c.bytes = vec![0, 0, c.bytes[2], c.bytes[3]];
        }
        let v = verdict_of(fe_of(&fe), &cp);
        assert!(
            insufficient(&v, "200 W target") && v.contains("no 2A63 crank-revolution data"),
            "{v}"
        );
    }

    /// A small deterministic generator (xorshift64) for the sweep.
    struct Rng(u64);
    impl Rng {
        #[allow(clippy::cast_precision_loss)]
        fn unit(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1_u64 << 53) as f64
        }
        /// Normal with the given σ, µs (sum of 12 uniforms).
        fn jitter(&mut self, sigma_us: f64) -> f64 {
            ((0..12).map(|_| self.unit()).sum::<f64>() - 6.0) * sigma_us
        }
    }

    /// `(page-25 hole, 2A63 hole)`, each `(from, length)` in µs.
    type Holes = (Option<(u64, u64)>, Option<(u64, u64)>);

    /// One 60 s ride with jittered, thinned streams; `(page-25 hole, 2A63 hole)` removes what
    /// arrives in `[a, a + len)`.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn jittered((phase, sigma_ms, drop_pct, seed): (u64, u64, u64, u64), holes: Holes) -> String {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15
            ^ ((seed + 1) * 0x1234_5679)
            ^ phase << 20
            ^ sigma_ms << 8
            ^ drop_pct);
        let sigma = sigma_ms as f64 * 1000.0;
        let stamp =
            |nominal: u64, rng: &mut Rng| (nominal as f64 + rng.jitter(sigma)).max(0.0) as u64;
        let keep = |rng: &mut Rng| rng.unit() * 100.0 >= drop_pct as f64;
        let gone = |hole: Option<(u64, u64)>, at: u64| {
            hole.is_some_and(|(a, len)| at >= a && at < a + len)
        };
        let mut fe: Vec<u64> = grid(250_000, 0)
            .into_iter()
            .map(|t| stamp(t, &mut rng))
            .collect();
        fe.retain(|t| !gone(holes.0, *t));
        fe.retain(|_| keep(&mut rng));
        let mut cp: Vec<Cps> = grid(SEC, phase * 1000)
            .into_iter()
            .map(|t| cps_at(stamp(t, &mut rng), t, 150))
            .collect();
        cp.retain(|c| !gone(holes.1, c.at_us));
        cp.retain(|_| keep(&mut rng));
        fe.sort_unstable();
        verdict_of(fe_of(&fe), &cp)
    }

    #[test]
    fn a_matching_run_agrees_wherever_its_notifications_fall_and_missing_coverage_never_does() {
        let (mut runs, mut agreed, mut leaked) = (0, 0, Vec::new());
        let mut failures = Vec::new();
        for phase in (0..1000).step_by(100) {
            for sigma_ms in [0, 5, 20, 50, 100] {
                for drop_pct in [0, 1, 3] {
                    for seed in 0..4 {
                        let k = (phase, sigma_ms, drop_pct, seed);
                        runs += 1;
                        let v = jittered(k, (None, None));
                        if v == "agree" {
                            agreed += 1;
                        } else if failures.len() < 5 {
                            failures.push(format!("{k:?}: {v}"));
                        }
                        let a = (12 + seed * 3) * SEC;
                        for holes in [(Some((a, 2_600_000)), None), (None, Some((a, 4 * SEC)))] {
                            let v = jittered(k, holes);
                            if v == "agree" || v.starts_with("disagree") && !v.contains("rate") {
                                leaked.push(format!("{k:?} {holes:?}: {v}"));
                            }
                        }
                    }
                }
            }
        }
        assert!(leaked.is_empty(), "{leaked:?}");
        assert!(
            agreed * 100 >= runs * 99,
            "{agreed} of {runs} agree: {failures:?}"
        );
    }

    /// Page-25 frames on `times` whose cadence is unavailable in `(a, b)` and whose last frame
    /// before `b` is followed by one at exactly `b`.
    fn fe_without_cadence(times: &[u64], a: u64, b: u64) -> Vec<Record> {
        let mut fe: Vec<Record> = times
            .iter()
            .map(|t| fe_frame_with(*t, false, *t > a && *t < b))
            .collect();
        if !times.contains(&b) {
            fe.push(fe_frame(b, false));
        }
        fe.sort_by_key(|r| r.at_us);
        fe
    }

    #[test]
    fn the_page_25_cadence_gap_bound_is_2_s_exactly() {
        let (fe, cp) = (grid(250_000, 0), cps_of(&grid(SEC, 400_000)));
        let at = verdict_of(fe_without_cadence(&fe, 15 * SEC, 17 * SEC), &cp);
        assert_eq!(at, "agree", "2 s without a cadence is within the bound");
        let over = verdict_of(fe_without_cadence(&fe, 15 * SEC, 17 * SEC + 1), &cp);
        assert!(insufficient(&over, "no page-25 cadence for"), "{over}");
    }

    #[test]
    fn page_25_cadence_in_one_frame_of_a_window_is_not_coverage_and_the_message_names_page_25() {
        let (fe, cp) = (grid(250_000, 0), cps_of(&grid(SEC, 400_000)));
        // Window 2 (40–60 s): cadence only in the frame at 50 s.
        let one: Vec<Record> = fe
            .iter()
            .map(|t| fe_frame_with(*t, false, *t >= 40 * SEC && *t != 50 * SEC))
            .collect();
        let v = verdict_of(one, &cp);
        assert!(
            insufficient(&v, "200 W target") && v.contains("no page-25 cadence for"),
            "{v}"
        );
        // Unavailable throughout: the page-25 stream is named, and 2A63 is not blamed.
        let none: Vec<Record> = fe
            .iter()
            .map(|t| fe_frame_with(*t, false, *t >= 40 * SEC))
            .collect();
        let v = verdict_of(none, &cp);
        assert!(insufficient(&v, "no page-25 cadence for"), "{v}");
        assert!(!v.contains("2A63 crank"), "{v}");
    }

    /// 2A63 notifications at `phase` after each second, crank data off in `[from, to)`.
    fn without_crank(phase: u64, from: u64, to: u64) -> Vec<Cps> {
        grid(SEC, phase)
            .into_iter()
            .map(|t| {
                let mut c = cps_at(t, t, 150);
                if t >= from && t < to {
                    c.bytes = vec![0, 0, c.bytes[2], c.bytes[3]];
                }
                c
            })
            .collect()
    }

    #[test]
    fn the_crank_gap_bound_is_3_s_exactly_at_the_start_of_a_window() {
        let fe = fe_of(&grid(250_000, 0));
        // No crank data 40–43 s: the first pair of window 2 starts at 43.0 s, 3 s in.
        let at = verdict_of(fe.clone(), &without_crank(0, 40 * SEC, 43 * SEC));
        assert_eq!(
            at, "agree",
            "a crank gap of exactly 3 s is within the bound"
        );
        let over = verdict_of(fe.clone(), &without_crank(1, 40 * SEC, 43 * SEC + 1));
        assert!(
            insufficient(&over, "no 2A63 crank-revolution data for 3 s"),
            "{over}"
        );
        // 35–44 s off: the first pair starts at 44.4 s, 4.4 s into window 2.
        let long = verdict_of(fe, &without_crank(400_000, 35 * SEC, 44 * SEC));
        assert!(
            insufficient(&long, "200 W target") && long.contains("data for 4.4 s"),
            "{long}"
        );
    }

    #[test]
    fn the_crank_gap_counts_the_end_of_a_window_too() {
        let fe = fe_of(&grid(250_000, 0));
        // Crank data stops at 56 s; window 2 ends at 59.75 s with its last frame.
        let v = verdict_of(fe, &without_crank(400_000, 56 * SEC, 70 * SEC));
        assert!(insufficient(&v, "no 2A63 crank-revolution data for"), "{v}");
    }

    #[test]
    fn a_stream_silent_from_the_start_or_to_the_end_of_a_window_is_a_gap() {
        let (fe, cp) = (grid(250_000, 0), grid(SEC, 400_000));
        // Window 1 is 10–30 s: page 25 resumes 2.5 s after it starts.
        let late: Vec<u64> = fe
            .iter()
            .copied()
            .filter(|t| *t < 10 * SEC || *t >= 12_500_000)
            .collect();
        let v = verdict_of(fe_of(&late), &cps_of(&cp));
        assert!(insufficient(&v, "no page 25 for 2.5 s"), "{v}");
        // 2A63 stops at 55.4 s, 4.35 s before window 2 ends.
        let early: Vec<u64> = cp.iter().copied().filter(|t| *t < 56 * SEC).collect();
        let v = verdict_of(fe_of(&fe), &cps_of(&early));
        assert!(insufficient(&v, "no 2A63 for"), "{v}");
    }
}
