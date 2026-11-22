//! The sleep observer's handler (ADR-0002 §5.1.3, #679): the Mac about to sleep is
//! `Release(Sleep)`, which discards a loaded ride that has not started and leaves a started one.
use crank_core::{Core, CoreConfig, DeviceMode, DeviceRole, EngineHost, LinkPhase};
use crank_desktop::sleep;
use crank_devices::sim::SimConfig;
use crank_engine::VirtualClock;
use std::{sync::Arc, time::Duration};

async fn core(dir: &std::path::Path) -> Core {
    let core = Core::open(CoreConfig {
        data_dir: dir.into(),
        devices: DeviceMode::Sim(SimConfig::default()),
        clock: Some(Arc::new(VirtualClock::new(1))),
        engine_host: EngineHost::Task,
    })
    .expect("core opens");
    core.device_pair("trainer".into(), DeviceRole::Trainer)
        .await
        .expect("paired");
    core
}

/// The trainer's keeper phase.
async fn trainer(core: &Core) -> Option<LinkPhase> {
    core.device_links()
        .await
        .expect("links")
        .into_iter()
        .find(|d| d.role == DeviceRole::Trainer)
        .and_then(|d| d.phase)
}

#[tokio::test(start_paused = true)]
async fn will_sleep_discards_an_unstarted_ride_and_releases_its_trainer() {
    let dir = tempfile::tempdir().unwrap();
    let core = core(dir.path()).await;
    core.ride_load(None).await.expect("loaded");
    assert!(core.ride_status().await.unwrap().status.loaded);

    sleep::will_sleep(&core).await.expect("applied");

    assert!(
        !core.ride_status().await.unwrap().status.loaded,
        "the unstarted ride is discarded"
    );
    assert_eq!(
        trainer(&core).await,
        Some(LinkPhase::Cold),
        "its trainer is released"
    );
    core.shutdown(Duration::from_secs(3)).await;
}

#[tokio::test(start_paused = true)]
async fn will_sleep_leaves_a_started_ride_alone() {
    let dir = tempfile::tempdir().unwrap();
    let core = core(dir.path()).await;
    core.ride_load(None).await.expect("loaded");
    core.ride_start().await.expect("started");

    sleep::will_sleep(&core).await.expect("applied");

    assert!(core.ride_status().await.unwrap().status.loaded);
    assert_eq!(trainer(&core).await, Some(LinkPhase::Riding));
    core.shutdown(Duration::from_secs(3)).await;
}
