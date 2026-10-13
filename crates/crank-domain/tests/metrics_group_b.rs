//! Group B schema parity for the frozen TypeScript reference.
mod common;
use common::{assert_vectors, load, number, numbers};
use crank_domain::{
    hr_recovery::{FitOptions, build_hr_recovery_state, fit_mono_exp, predict_at},
    hrv::{dfa_alpha1, rmssd},
    wbal::{wbal_series, wbal_step},
};
use serde_json::{Value, json};
#[test]
fn wbal_vectors() {
    let expected = load("wbal");
    let mut actual = expected.clone();
    for c in actual["cases"].as_array_mut().unwrap() {
        let i = &c["input"];
        c["output"] = if c["name"] == "step" {
            json!(
                wbal_step(
                    number(&i["w"]),
                    number(&i["power"]),
                    number(&i["dt"]),
                    number(&i["cp"]),
                    number(&i["wPrime"])
                )
                .unwrap()
            )
        } else {
            Value::Object(
                wbal_series(
                    &numbers(&i["powers"]),
                    number(&i["dt"]),
                    number(&i["cp"]),
                    number(&i["wPrime"]),
                )
                .unwrap()
                .into_iter()
                .enumerate()
                .map(|(i, v)| (i.to_string(), json!(v)))
                .collect(),
            )
        };
    }
    assert_vectors("wbal", &actual, &expected);
}
#[test]
fn hrv_vectors() {
    let expected = load("hrv");
    let mut actual = expected.clone();
    for c in actual["cases"].as_array_mut().unwrap() {
        let rr = numbers(&c["input"]["rr"]);
        c["output"] = json!({"rmssd":rmssd(&rr),"dfaAlpha1":dfa_alpha1(&rr,4,16,25).unwrap()});
    }
    assert_vectors("hrv", &actual, &expected);
}
#[test]
fn recovery_vectors() {
    let expected = load("recovery");
    let mut actual = expected.clone();
    for c in actual["cases"].as_array_mut().unwrap() {
        let i = &c["input"];
        c["output"] = if c["name"].as_str().unwrap().starts_with("fit-") {
            let fit = fit_mono_exp(
                &numbers(&i["ts"]),
                &numbers(&i["ys"]),
                number(&i["t0"]),
                &serde_json::from_value(i["prior"].clone()).unwrap(),
                &FitOptions::default(),
            )
            .unwrap();
            let prediction = fit.as_ref().map(|f| predict_at(f, 1180.).unwrap());
            json!({"fit":fit,"prediction":prediction})
        } else {
            json!(build_hr_recovery_state(&serde_json::from_value(i.clone()).unwrap()).unwrap())
        };
    }
    assert_vectors("recovery", &actual, &expected);
}
