//! Part B check 5: FE-C page 25 against the trainer's Cycling Power Service measurement
//! (2A63) during a ride (#44 check 5, ADR-0002 §5 and §9; #412 part b).
//!
//! Like the other Part B results this is a pure function of the capture: the FE-C records and
//! the `# cps` comment lines ([`crate::frames::CPS_TAG`]) that the live run wrote beside them.
//! `docs/hil/part-b.md` states the method and the reasoning behind every tolerance.
use crate::{
    frames::{Cps, Record},
    partb::{SEC, SETTLE, controls, erg_holds, mean, procedure_end, round1, samples, secs},
};
use serde_json::{Value, json};

/// Per-second mean power must agree within this (ADR-0002 §5, HIL check 5).
pub const POWER_TOL_W: f64 = 1.0;
/// A hold's mean cadence must agree within this: page 25 rounds to 1 rpm (±0.5) and the two
/// streams are sampled at different instants (±0.5).
pub const CADENCE_TOL_RPM: f64 = 1.0;
/// Page 25 must arrive at least this often (ADR-0002 §5, HIL check 5).
pub const RATE_MIN_HZ: f64 = 2.0;
/// Fewer compared seconds than this give no verdict: a verdict needs a ride, not a glimpse.
pub const MIN_SECONDS: usize = 20;
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

/// Check 5: page-25 rate, and page-25 against CPS power and cadence over the settled holds.
pub fn check5(records: &[Record], cps: &[Cps]) -> Value {
    let s = samples(records);
    let ctl = controls(records);
    let parsed = parse(cps);
    // Settled windows: every ERG target held ≥ 10 s (check 6's definition of a hold), after its
    // first 10 s, and never the 0 W wait for the rider.
    let windows: Vec<(u64, u64)> = erg_holds(&ctl, procedure_end(records, &ctl))
        .into_iter()
        .filter(|(held, h)| *held && h.watts > 0.0)
        .map(|(_, h)| (h.at + SETTLE, h.end))
        .collect();

    let mut seconds = Vec::new();
    let mut cadence = Vec::new();
    for &(from, to) in &windows {
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
            if let (Some(fe), Some(cp)) = (fe, cp) {
                seconds.push((fe, cp));
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
        if let (Some(fe), true) = (fe, time > 0.0) {
            cadence.push((fe, 60.0 * revs / time));
        }
    }

    let diffs: Vec<f64> = seconds.iter().map(|(a, b)| a - b).collect();
    let outside = diffs
        .iter()
        .filter(|d| d.abs() > POWER_TOL_W + 1e-9)
        .count();
    let max_abs = diffs.iter().fold(0.0_f64, |m, d| m.max(d.abs()));
    let cadence_diffs: Vec<f64> = cadence.iter().map(|(a, b)| a - b).collect();
    let cadence_max = cadence_diffs.iter().fold(0.0_f64, |m, d| m.max(d.abs()));

    let times: Vec<u64> = s.iter().map(|x| x.at_us).collect();
    #[allow(clippy::cast_precision_loss)]
    let rate_hz = times
        .first()
        .zip(times.last())
        .filter(|(f, l)| l > f)
        .map(|(f, l)| (times.len() - 1) as f64 / secs(l - f));
    let max_gap_s = times.windows(2).map(|w| secs(w[1] - w[0])).reduce(f64::max);

    let verdict = verdict(
        &parsed,
        rate_hz,
        (seconds.len(), outside, max_abs),
        (cadence.len(), cadence_max),
    );
    json!({
        "method": "both streams are stamped on arrival by the same process clock, so no offset is estimated; settled windows = each page-49 target held >= 10 s, after its first 10 s (power is steady under ERG, so the residual jitter of the two notification streams is not mistaken for disagreement); power: for each whole second of a window with at least one sample of each stream, mean page-25 power minus mean 2A63 power, which must be within ±1 W (ADR-0002 §5) in every compared second, of at least 20; cadence: per window, mean page-25 cadence against 2A63 crank revolutions over crank event time (differences modulo 2^16, event time in 1/1024 s), within ±1 rpm; page 25 must arrive at >= 2 Hz",
        "tolerances": {"power_w": POWER_TOL_W, "cadence_rpm": CADENCE_TOL_RPM, "page25_min_hz": RATE_MIN_HZ, "min_seconds": MIN_SECONDS},
        "page25": {"samples": s.len(), "rate_hz": rate_hz.map(round1), "max_gap_s": max_gap_s.map(round1)},
        "cps": {
            "notifications": cps.len(),
            "decoded": parsed.readings.len(),
            "malformed": parsed.malformed,
            "crank_pairs": parsed.pairs.len(),
            "rejected_pairs": parsed.rejected,
        },
        "windows_s": windows.iter().map(|(f, t)| json!([round1(secs(*f)), round1(secs(*t))])).collect::<Vec<_>>(),
        "seconds_compared": seconds.len(),
        "seconds_within_tolerance": seconds.len() - outside,
        "mean_page25_w": mean(seconds.iter().map(|x| x.0)).map(round1),
        "mean_cps_w": mean(seconds.iter().map(|x| x.1)).map(round1),
        "bias_w": mean(diffs.iter().copied()).map(|x| (x * 100.0).round() / 100.0),
        "max_abs_diff_w": (!diffs.is_empty()).then_some(round1(max_abs)),
        "cadence_windows": cadence.iter().map(|(a, b)| json!({"page25_rpm": round1(*a), "cps_rpm": round1(*b)})).collect::<Vec<_>>(),
        "max_abs_cadence_diff_rpm": (!cadence.is_empty()).then_some(round1(cadence_max)),
        "verdict": verdict,
    })
}

/// `(seconds compared, seconds outside ±1 W, largest |difference|)` and
/// `(cadence windows compared, largest |difference|)`.
fn verdict(
    parsed: &Parsed,
    rate_hz: Option<f64>,
    (seconds, outside, max_abs): (usize, usize, f64),
    (windows, cadence_max): (usize, f64),
) -> String {
    if parsed.readings.is_empty() {
        return format!(
            "no CPS: no decodable 2A63 notification in the capture ({} malformed)",
            parsed.malformed
        );
    }
    if seconds < MIN_SECONDS {
        return format!(
            "insufficient: {seconds} seconds compared, {MIN_SECONDS} needed (no settled hold with both streams)"
        );
    }
    if windows == 0 {
        return "insufficient: no 2A63 crank-revolution data to derive cadence".into();
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
            "power: {outside} of {seconds} seconds outside ±{POWER_TOL_W} W (max {} W)",
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
            cps(40 * SEC, 103, 40, 5000),
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
        let mut records = vec![Record {
            at_us: 0,
            rx: false,
            bytes: encode(&ControlPage::TargetPower { watts: 150.0 })
                .expect("encodes")
                .to_vec(),
        }];
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
}
