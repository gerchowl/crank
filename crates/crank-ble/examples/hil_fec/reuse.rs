//! `reuse`: does the production connect path find a device by adapter lookup or by a scan
//! (handle reuse, ADR-0002 §5.1.2, #468)? It records `via` in each of three cases: right
//! after a Devices scan, after that scan stopped, and after a disconnect.
//!
//! One run is one fresh process: a Devices-style scan of `--scan-secs` (the app's is 5 s),
//! a wait of `--idle-secs` after it stopped, a connect (`first`), its release, and at once a
//! second connect (`after_disconnect`). Run it with `--idle-secs 0` for "right after a
//! Devices scan" and with a longer wait for "after that scan stopped"; both runs give the
//! after-a-disconnect case. `--target` picks the platform id; without it the first trainer
//! seen is used. Each connect is the app's own (`BleCentral`), so a trainer gets page 55 and
//! the page-54 request, as pairing does, and no mode or target page.
use crate::{
    checks::Outcome,
    cli::{Args, Check, write_summary},
    session::Result,
};
use crank_ble::{BleCentral, MonotonicClock, RideGate};
use crank_devices::{
    DeviceRef, Discovery, HeartRateDriver, RiderConfig, TrainerCommand, TrainerDriver,
    phase::{self, ConnectPhase, Via},
    release_events,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::{Instant, sleep, timeout};

/// Longest one connect may take, as the app's `CONNECT_TIMEOUT`.
const CONNECT: Duration = Duration::from_secs(15);

/// Run the `reuse` check.
pub async fn run(args: &Args) -> Result<Outcome> {
    let central = BleCentral::new().await?;
    let mut scan = central.discovery().scan();
    let wanted = |d: &crank_devices::DiscoveredDevice| match &args.target {
        Some(id) => d.device.0 == *id,
        None => d.trainer.is_some(),
    };
    let mut found = None;
    let _ = timeout(Duration::from_secs(args.scan_secs), async {
        while let Some(d) = scan.next().await {
            if found.is_none() && wanted(&d) {
                found = Some((d.device.clone(), d.trainer.is_some()));
            }
        }
    })
    .await;
    drop(scan);
    let Some((device, trainer)) = found else {
        return Ok(Outcome::Blocked(
            "the device was not seen in the scan: power it (and wear the strap), and keep other apps disconnected".into(),
        ));
    };
    sleep(Duration::from_secs(args.idle_secs)).await;
    let first = connect(&central, &device, trainer).await;
    let again = connect(&central, &device, trainer).await;
    let summary = json!({
        "device": if trainer { "trainer" } else { "strap" },
        "scan_secs": args.scan_secs,
        "idle_secs_after_scan": args.idle_secs,
        "first": first,
        "after_disconnect": again,
    });
    println!("first: {first}\nafter_disconnect: {again}");
    Ok(Outcome::Done(write_summary(
        &args.out,
        Check::Reuse,
        &summary,
    )?))
}

/// One connect of `device` through the app's drivers, then its release: how it was resolved,
/// how long it took, or why it failed.
async fn connect(central: &BleCentral, device: &DeviceRef, trainer: bool) -> Value {
    let (clock, gate) = (Arc::new(MonotonicClock::default()), RideGate::new(false));
    let via = Arc::new(Mutex::new(None));
    let sink = via.clone();
    let report = Arc::new(move |p| {
        if let ConnectPhase::Gatt(v) = p {
            *sink
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(v);
        }
    });
    let started = Instant::now();
    let attempt = phase::observe(report, async {
        if trainer {
            let rider = RiderConfig {
                rider_kg: 72.0,
                bike_kg: 9.0,
                wheel_m: 2.096,
            };
            let mut link = central
                .trainer_driver(clock, gate)
                .connect(device, rider)
                .await?;
            let ms = started.elapsed().as_millis();
            let _ = link.control.send(TrainerCommand::Disconnect).await;
            while link.events.recv().await.is_some() {}
            Ok(ms)
        } else {
            let mut link = central.hr_driver(clock, gate).connect(device).await?;
            let ms = started.elapsed().as_millis();
            release_events(&mut link.events).await;
            Ok::<_, crank_devices::DeviceError>(ms)
        }
    });
    let result = timeout(CONNECT, attempt).await;
    let via = via
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .map(Via::as_str);
    match result {
        Ok(Ok(ms)) => json!({ "via": via, "ms": ms }),
        Ok(Err(e)) => json!({ "via": via, "error": e.to_string() }),
        Err(_) => json!({ "via": via, "error": "connect timed out" }),
    }
}
