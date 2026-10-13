//! Export Rust bucket edges for the TS side of the scale parity gate.
use crank_domain::scales::{TimeScale, time_ago_edges};
use serde_json::json;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cases = Vec::new();
    for scale in [TimeScale::Log, TimeScale::Fisheye] {
        for span in [1., 10., 480., 1800., 7200., 86400.] {
            for buckets in [1, 2, 7, 100, 2000] {
                cases.push(json!({"spanS":span,"buckets":buckets,"scale":scale,"edges":time_ago_edges(span,buckets,scale)}));
            }
        }
    }
    println!("{}", serde_json::to_string(&cases)?);
    Ok(())
}
