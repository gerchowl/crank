//! Golden vectors (#620): `tests/fixtures/slew_golden.json`, cases with per-tick R, y, v, a, on
//! the line, cut and degraded. This test compares the port's run with them within 1e-6 (the
//! drift check) and rewrites them with `CRANK_SLEW_GOLDEN=write`; `erg_slew_vectors.py --check`
//! compares the same vectors with the model (upstream Ruckig), so the port and the model agree.
use super::harness::{Ev, Faults, Kind, Plan, Report, Trace, check, run, tu};
use crate::slew::{Block, Cut};
use serde_json::{Value, json};

const PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/slew_golden.json"
);
const TOL: f64 = 1e-6;

fn load() -> Value {
    let text = std::fs::read_to_string(PATH).expect("the golden fixture");
    serde_json::from_str(&text).expect("JSON")
}

fn f(v: &Value) -> f64 {
    v.as_f64().expect("a number")
}

/// A fixture case as a plan (the exporter's `case()`).
pub(super) fn plan_of(c: &Value) -> Plan {
    let t = &c["tuning"];
    let tuning = tu(
        u16::try_from(t["riseWPerS"].as_u64().expect("rise")).expect("u16"),
        f(&t["fallFactor"]),
        f(&t["easeS"]),
    );
    let blocks = c["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .map(|b| Block {
            ticks: u32::try_from(b[0].as_u64().expect("ticks")).expect("u32"),
            from_w: f(&b[1]),
            to_w: f(&b[2]),
        })
        .collect();
    let events = c["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| Ev {
            tick: e[0].as_i64().expect("tick"),
            kind: match e[1].as_str().expect("kind") {
                "offset" => Kind::Offset,
                "cap" => Kind::Cap,
                "skip" => Kind::Skip,
                "engage" => Kind::Engage,
                other => panic!("event kind {other}"),
            },
            value: f(&e[2]),
        })
        .collect();
    let fz = &c["faults"];
    let mut faults = Faults {
        ticks: fz["ticks"]
            .as_array()
            .map(|a| a.iter().map(|k| k.as_i64().expect("tick")).collect())
            .unwrap_or_default(),
        twin_answers: fz["twin_answers"].as_bool().unwrap_or(false),
        fail_brake: fz["fail_brake"].as_bool().unwrap_or(false),
        ..Faults::default()
    };
    if let Some(from) = fz["from"].as_i64() {
        faults.share = 1.0;
        faults.from = from;
        faults.until = fz["until"].as_i64();
    }
    Plan {
        tuning,
        blocks,
        events,
        origin: c["origin"].as_f64(),
        m0: f(&c["m0"]),
        y0: c["y0"].as_f64(),
        v0: f(&c["v0"]),
        faults,
    }
}

fn cut_name(c: Option<Cut>) -> Value {
    match c {
        None => Value::Null,
        Some(c) => json!(format!("{c:?}")),
    }
}

fn round9(x: f64) -> f64 {
    (x * 1e9).round() / 1e9
}

fn vectors(tr: &Trace) -> Value {
    let r9 = |xs: &[f64]| xs.iter().map(|x| round9(*x)).collect::<Vec<_>>();
    json!({
        "r": r9(&tr.r), "y": r9(&tr.y), "v": r9(&tr.v), "a": r9(&tr.a), "on": tr.on,
        "cut": tr.exc.iter().map(|c| cut_name(*c)).collect::<Vec<_>>(), "degraded": tr.degraded,
    })
}

/// The first disagreement between the port's vectors and the committed ones, if any.
fn diff(name: &str, got: &Value, want: &Value) -> Option<String> {
    for key in ["r", "y", "v", "a"] {
        let (g, w) = (got[key].as_array()?, want[key].as_array()?);
        if g.len() != w.len() {
            return Some(format!(
                "{name}: {key} has {} ticks, committed {}",
                g.len(),
                w.len()
            ));
        }
        for (k, (x, y)) in g.iter().zip(w).enumerate() {
            if (f(x) - f(y)).abs() > TOL {
                return Some(format!("{name}: {key}[{k}] = {x}, committed {y}"));
            }
        }
    }
    for key in ["on", "cut", "degraded"] {
        if got[key] != want[key] {
            let (g, w) = (got[key].as_array()?, want[key].as_array()?);
            let k = g.iter().zip(w).position(|(x, y)| x != y).unwrap_or(0);
            return Some(format!(
                "{name}: {key}[{k}] = {:?}, committed {:?}",
                g.get(k),
                w.get(k)
            ));
        }
    }
    None
}

#[test]
fn the_golden_vectors_agree_with_the_committed_ones_and_every_property_holds() {
    let mut data = load();
    let write = std::env::var("CRANK_SLEW_GOLDEN").is_ok_and(|v| v == "write");
    let mut rep = Report::default();
    let mut drift = Vec::new();
    let cases = data["golden"].as_array_mut().expect("golden");
    assert!(cases.len() >= 50, "the golden cases are missing");
    for c in cases.iter_mut() {
        let name = c["name"].as_str().expect("name").to_owned();
        let p = plan_of(c);
        let tr = run(&p);
        check(&tr, &p.tuning, &mut rep, &format!("golden: {name}"));
        let got = vectors(&tr);
        if let Some(d) = diff(&name, &got, &c["vectors"]) {
            drift.push(d);
        }
        if write {
            c["vectors"] = got;
        }
    }
    if write {
        let mut text = serde_json::to_string(&data).expect("JSON");
        text.push('\n');
        std::fs::write(PATH, text).expect("write the golden fixture");
    }
    rep.assert_clean();
    assert!(
        drift.is_empty(),
        "golden vectors drifted ({}):\n{}",
        drift.len(),
        drift.join("\n")
    );
}

#[test]
fn the_models_generator_plans_keep_every_property() {
    // pinned seeds, the falsifiers' schemes (refusal windows, q4v7 persistent refusals, cap events
    // during refusals, planned falls with refusals), as the model draws them.
    let data = load();
    let plans = data["plans"].as_array().expect("plans");
    assert!(plans.len() > 800, "the model's plans are missing");
    let mut rep = Report::default();
    for c in plans {
        let p = plan_of(c);
        let tr = run(&p);
        check(&tr, &p.tuning, &mut rep, c["name"].as_str().expect("name"));
    }
    rep.assert_clean();
    assert!(
        rep.recoveries > 50,
        "only {} recoveries checked",
        rep.recoveries
    );
    assert!(
        rep.degraded_runs > 100,
        "only {} degraded runs checked",
        rep.degraded_runs
    );
}
