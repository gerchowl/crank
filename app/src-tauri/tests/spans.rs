//! The `ride` and `device` spans (ADR-0001 §7.1, #465) in vigil's `traces.jsonl`. vigil exports a span
//! when it closes and none still open at shutdown (vigil#8), so each test ends or quits first and then
//! shuts vigil down. vigil allows one pipeline per process: run these with nextest.
use crank_core::{Core, CoreConfig, DeviceMode, DeviceRole, EngineHost};
use crank_desktop::{
    lifecycle::Quit,
    logging::{LogPlan, Logging},
};
use crank_devices::sim::SimConfig;
use crank_engine::VirtualClock;
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};

/// A core on sim devices with a ride loaded and started, under a logging pipeline in `dir`.
async fn riding(dir: &Path) -> (Core, Logging) {
    let plan = LogPlan::new(None, dir.join("logs"));
    let logging = Logging::new(plan.install(None).expect("logging starts"));
    let core = Core::open(CoreConfig {
        data_dir: dir.join("data"),
        devices: DeviceMode::Sim(SimConfig::default()),
        clock: Some(Arc::new(VirtualClock::new(1))),
        engine_host: EngineHost::Task,
    })
    .expect("core opens");
    for (id, role) in [
        ("trainer", DeviceRole::Trainer),
        ("strap", DeviceRole::HeartRate),
    ] {
        core.device_pair(id.into(), role).await.expect("pair");
    }
    core.ride_load(Some("builtin-sensor-check".into()))
        .await
        .expect("load");
    core.ride_start().await.expect("start");
    tokio::time::sleep(Duration::from_secs(20)).await;
    (core, logging)
}

/// Every JSON line of vigil's `signal` files in `dir/logs`, parsed.
fn lines(dir: &Path, signal: &str) -> Vec<Value> {
    vigil::rotate::read_since(&dir.join("logs"), signal, None)
        .expect("read")
        .iter()
        .map(|l| serde_json::from_str(l).expect("a complete line"))
        .collect()
}

/// The values at `path` (array fields flattened) of every line.
fn all<'a>(lines: &'a [Value], path: &[&str]) -> Vec<&'a Value> {
    let mut at: Vec<&Value> = lines.iter().collect();
    for key in path {
        at = at
            .into_iter()
            .flat_map(|v| match &v[*key] {
                Value::Array(a) => a.iter().collect(),
                Value::Null => vec![],
                other => vec![other],
            })
            .collect();
    }
    at
}

fn attr<'a>(span: &'a Value, key: &str) -> Option<&'a str> {
    span["attributes"]
        .as_array()?
        .iter()
        .find(|a| a["key"] == key)?["value"]["stringValue"]
        .as_str()
}

/// What `traces.jsonl` and `logs.jsonl` hold of a ride that was ended one way or another.
fn check_exported(dir: &Path) {
    let traces = lines(dir, "traces");
    let spans = all(&traces, &["resourceSpans", "scopeSpans", "spans"]);
    let named =
        |name: &str| -> Vec<&&Value> { spans.iter().filter(|s| s["name"] == name).collect() };
    let ride = named("ride");
    assert_eq!(ride.len(), 1, "{spans:#?}");
    let ride = ride[0];
    assert!(attr(ride, "id").is_some_and(|id| id.starts_with("ride-")));
    assert_ne!(ride["endTimeUnixNano"], Value::Null, "closed: {ride:#?}");
    let devices = named("device");
    let mut kinds: Vec<_> = devices.iter().filter_map(|d| attr(d, "kind")).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["strap", "trainer"], "{devices:#?}");
    for device in &devices {
        assert_eq!(
            device["parentSpanId"], ride["spanId"],
            "a child of the ride"
        );
        // The role placeholder of ADR-0003 §2.3: no name, serial or platform id.
        let id = attr(device, "id").expect("id");
        assert!(id.starts_with('<') && id.ends_with("-id>"), "{id}");
    }
    let logs = lines(dir, "logs");
    let records = all(&logs, &["resourceLogs", "scopeLogs", "logRecords"]);
    assert!(
        records
            .iter()
            .any(|r| r["spanId"] == ride["spanId"] && r["body"]["stringValue"] == "ride released"),
        "the ride's own record carries its span id: {records:#?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_ride_that_ends_exports_its_ride_and_device_spans_closed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (core, logging) = riding(dir.path()).await;
    core.ride_end().await.expect("end");
    logging.shut_down(Duration::from_secs(5));
    check_exported(dir.path());
}

#[tokio::test(start_paused = true)]
async fn a_quit_mid_ride_exports_the_ride_span_closed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (core, logging) = riding(dir.path()).await;
    let quit = Quit::default();
    quit.shut_down(&core).await;
    logging.shut_down(quit.remaining());
    check_exported(dir.path());
}
