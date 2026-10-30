//! Raw-stream parity of `SimCore` with `simulator.json` (#67, ADR-0001 §3 float rule).
//!
//! The vector records a whole-ride run. Its inputs are the workout, athlete, seed and
//! tick count; the trainer commands are not stored. They are derived here the way
//! `RideSim.computeTarget` derives them (`shared/src/sim.ts`) **for this vector's
//! features only**: block power from the compiled workout (`compile_workout`),
//! lerped over the block, the `free` sine target, and block advancement by time or by
//! the HR-recovery end rule fed from the simulated strap's last reading. Not derived
//! (they do not occur in the vectors, and the harness panics on unsupported end
//! kinds): HR targets, `kj` ends, coach `blockScale`, manual offset and the
//! recovery-lowered ×0.75. Full target derivation belongs to the engine (#32) and
//! #203. The target is passed to the core as `f64`; the drivers quantise to 1 W
//! (ADR-0002 §7.1), which is tested separately.
//!
//! Compared: `power`, `cadence`, `speed`, `hr`, `rr` and `wbal_pct`. Derived metrics,
//! the bus and coach hooks are #203.
use crank_devices::sim::{DT, Drive, SimCore, SimRider};
use crank_domain::workout::{Athlete, CompileOptions, Workout, compile_workout};
use serde_json::Value;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= f64::max(1e-12 * a.abs().max(b.abs()), 1e-15)
}
fn steps<'a>(nodes: &'a [Value], out: &mut Vec<&'a Value>) {
    for n in nodes {
        if n["type"] == "step" {
            out.push(n);
        } else if let Some(c) = n["children"].as_array() {
            steps(c, out);
        }
    }
}
#[derive(Default)]
struct Streams {
    power: Vec<(f64, Option<f64>)>,
    cadence: Vec<(f64, Option<f64>)>,
    speed: Vec<(f64, Option<f64>)>,
    wbal: Vec<(f64, Option<f64>)>,
    hr: Vec<(f64, Option<f64>)>,
    rr: Vec<(f64, Option<f64>)>,
}

fn run(input: &Value) -> Streams {
    let workout: Workout = serde_json::from_value(input["workout"].clone()).expect("vector");
    let athlete: Athlete = serde_json::from_value(input["athlete"].clone()).expect("vector");
    let compiled = compile_workout(
        &workout,
        &CompileOptions {
            athlete: Some(athlete.clone()),
            usual_tau_s: None,
        },
    )
    .expect("vector");
    let mut nodes = Vec::new();
    steps(
        input["workout"]["nodes"].as_array().expect("vector"),
        &mut nodes,
    );
    let find = |id: &str| *nodes.iter().find(|n| n["id"] == id).expect("vector");

    let seed = u32::try_from(input["seed"].as_u64().expect("vector")).expect("vector");
    let ticks = input["ticks"].as_u64().expect("vector");
    assert!(close(input["dt"].as_f64().expect("vector"), DT));
    // mass 72 kg rider + 9 kg bike, as in sim.ts (`weightKg + 9`)
    let mut core = SimCore::new(
        SimRider::new(athlete.ftp, athlete.max_hr, athlete.rest_hr),
        athlete.weight_kg + 9.0,
        seed,
    );
    let (mut block, mut block_start, mut last_hr) = (0usize, 0.0, None::<f64>);
    let mut s = Streams::default();
    'ride: for k in 1..=ticks {
        let t = f64::from(u32::try_from(k).expect("vector")) * DT;
        loop {
            let b = &compiled.blocks[block];
            let step = find(&b.node_id);
            let el = t - block_start;
            let done = match step["end"]["kind"].as_str().expect("vector") {
                "time" | "lap" => el >= b.dur_s - 1e-9,
                "hr_recovery" => {
                    let e = &step["end"];
                    el >= e["maxS"].as_f64().expect("vector")
                        || (el >= e["minS"].as_f64().expect("vector")
                            && last_hr
                                .is_some_and(|h| h <= e["targetBpm"].as_f64().expect("vector")))
                }
                other => panic!("unsupported end kind {other}"),
            };
            if !done {
                break;
            }
            if block + 1 == compiled.blocks.len() {
                break 'ride; // the reference finishes without publishing this tick
            }
            block += 1;
            block_start = t;
        }
        let b = &compiled.blocks[block];
        let step = find(&b.node_id);
        let el = t - block_start;
        assert!(
            matches!(step["target"]["kind"].as_str(), Some("pct_ftp" | "free")),
            "unsupported target kind {}",
            step["target"]["kind"]
        );
        let target = if step["target"]["kind"] == "free" {
            0.55 * athlete.ftp * (1.0 + 0.1 * libm::sin(t / 90.0) + 0.05 * libm::sin(t / 23.0))
        } else {
            let frac = if b.dur_s > 0.0 {
                (el / b.dur_s).clamp(0.0, 1.0)
            } else {
                0.0
            };
            b.power_from + (b.power_to - b.power_from) * frac
        }
        .max(0.0);
        let tick = core.step(Drive {
            target_w: target,
            grade_pct: 0.0,
            cadence_pref: None,
            cadence_force: None,
        });
        s.power.push((t, tick.power_w));
        s.cadence.push((t, tick.cadence_rpm));
        s.speed.push((t, tick.speed_kmh));
        s.wbal.push((t, Some(tick.wbal_pct)));
        if let Some(h) = tick.hr {
            let v = h.bpm.map(f64::from);
            if v.is_some() {
                last_hr = v;
            }
            s.hr.push((t, v));
        }
        for (bt, rr) in tick.beats {
            s.rr.push((bt, Some(f64::from(rr))));
        }
    }
    s
}

/// The crate directory at run time. `env!` would bake in the path of the build that compiled this
/// binary, which no longer exists when the Nix checks reuse it (#371); cargo and nextest set the variable.
fn manifest_dir() -> String {
    std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR for tests")
}

#[test]
fn raw_streams_match_simulator_vectors() {
    let path = format!("{}/../crank-domain/tests/vectors/simulator.json", manifest_dir());
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("vector")).expect("vector");
    let cases = doc["cases"].as_array().expect("vector");
    assert!(
        cases.len() >= 3,
        "two 120 s rides and the harder 380 s ride"
    );
    for case in cases {
        let got = run(&case["input"]);
        for (metric, ours) in [
            ("power", &got.power),
            ("cadence", &got.cadence),
            ("speed", &got.speed),
            ("wbal_pct", &got.wbal),
            ("hr", &got.hr),
            ("rr", &got.rr),
        ] {
            let want: Vec<(f64, Option<f64>)> = case["output"]["published"]
                .as_array()
                .expect("vector")
                .iter()
                .filter(|p| p["metric"] == metric && p["reset"] == false)
                .map(|p| (p["t"].as_f64().expect("vector"), p["v"].as_f64()))
                .collect();
            assert_eq!(want.len(), ours.len(), "{} {metric}: count", case["name"]);
            for (i, (w, o)) in want.iter().zip(ours.iter()).enumerate() {
                let same_v = match (w.1, o.1) {
                    (Some(a), Some(b)) => close(a, b),
                    (None, None) => true,
                    _ => false,
                };
                assert!(
                    close(w.0, o.0) && same_v,
                    "{} {metric}[{i}]: vector {w:?} != sim {o:?}",
                    case["name"]
                );
            }
        }
    }
}
