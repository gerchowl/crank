//! Rust equivalents and golden parity for the frozen metric reference.
mod common;
use common::{assert_vectors, load, number, numbers};
use crank_domain::{
    distributions::{
        MMP_DURATIONS_S, heatmap_from, histogram_from, mmp_from, synthetic_mmp,
        typical_distribution,
    },
    filters::FilterChain,
    history::HistoryStore,
    scales::{TimeScale, time_ago_edges, time_ago_to_u, u_to_time_ago},
};
use serde_json::{Value, json};

fn scale(v: &Value) -> TimeScale {
    serde_json::from_value(v.clone()).expect("valid vector parameter")
}
fn count(v: &Value) -> u32 {
    serde_json::from_value(v.clone()).expect("valid vector parameter")
}

#[test]
fn filter_vectors() {
    let expected = load("filters");
    let mut actual = expected.clone();
    for c in actual["cases"].as_array_mut().unwrap() {
        let input = &c["input"];
        let mut chain =
            FilterChain::new(serde_json::from_value::<Vec<_>>(input["filters"].clone()).unwrap());
        c["output"] = json!(
            input["samples"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| chain.push(
                    number(&s["t"]),
                    if s["v"].is_null() {
                        None
                    } else {
                        Some(number(&s["v"]))
                    }
                ))
                .collect::<Vec<_>>()
        );
    }
    assert_vectors("filters", &actual, &expected);
}
#[test]
fn history_vectors() {
    let expected = load("history");
    let mut actual = expected.clone();
    for c in actual["cases"].as_array_mut().unwrap() {
        let input = &c["input"];
        c["output"] = if input.get("samples").is_some() {
            let mut store = HistoryStore::default();
            for s in input["samples"].as_array().unwrap() {
                store.add(
                    "power",
                    number(&s["t"]),
                    if s["v"].is_null() {
                        None
                    } else {
                        Some(number(&s["v"]))
                    },
                );
            }
            if input["reset"] == true {
                store.reset();
            }
            let q = &input["query"];
            json!(store.query(
                q["metric"].as_str().unwrap(),
                number(&q["now"]),
                number(&q["spanS"]),
                count(&q["buckets"]),
                scale(&q["scale"])
            ))
        } else {
            let span = number(&input["spanS"]);
            let sc = scale(&input["scale"]);
            // Mapping inputs are derived from the supplied bucket count, not output values.
            let mapping: Vec<_> = [-0.1, 0., 0.05, 0.5, 0.95, 1., 1.1]
                .into_iter()
                .map(|u| {
                    let t = u_to_time_ago(u, span, sc);
                    json!({"u":u,"tAgo":t,"back":time_ago_to_u(t,span,sc)})
                })
                .collect();
            json!({"edges":time_ago_edges(span,count(&input["buckets"]),sc),"mapping":mapping})
        };
    }
    assert_vectors("history", &actual, &expected);
}
#[test]
fn distribution_vectors() {
    let expected = load("distributions");
    let mut actual = expected.clone();
    for c in actual["cases"].as_array_mut().unwrap() {
        let i = &c["input"];
        c["output"] = match c["name"].as_str().unwrap() {
            "histogram" => {
                let typical = typical_distribution(
                    i["metric"].as_str().unwrap(),
                    number(&i["athlete"]["ftp"]),
                );
                json!(histogram_from(
                    i["metric"].as_str().unwrap(),
                    &numbers(&i["series"]),
                    number(&i["binWidth"]),
                    typical.as_ref().map(|f| f as &dyn Fn(f64, f64) -> f64)
                ))
            }
            "heatmap" => json!(heatmap_from(
                i["x"].as_str().unwrap(),
                i["y"].as_str().unwrap(),
                &numbers(&i["xs"]),
                &numbers(&i["ys"]),
                number(&i["xBin"]),
                number(&i["yBin"])
            )),
            "mmp" => json!(mmp_from(
                &numbers(&i["power"]),
                &serde_json::from_value::<Vec<u16>>(i["durations"].clone()).unwrap()
            )),
            "syntheticMmp" => json!(synthetic_mmp(
                number(&i["athlete"]["ftp"]),
                number(&i["scale"]),
                &MMP_DURATIONS_S
            )),
            name => panic!("unhandled vector {name}"),
        };
    }
    assert_vectors("distributions", &actual, &expected);
}
