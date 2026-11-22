//! The sleep observer's handler (ADR-0002 §5.1.3, #679): the Mac about to sleep is
//! `Release(Sleep)`, which discards a loaded ride that has not started and leaves a started one.
//! `on_will_sleep` is the handler `crank_desktop::run` registers; `will_sleep` is what it spawns.
use crank_core::{Core, CoreConfig, DeviceMode, DeviceRole, EngineHost, LinkPhase};
use crank_desktop::sleep;
use crank_devices::sim::SimConfig;
use crank_engine::VirtualClock;
use std::{sync::Arc, time::Duration};
use tauri::{
    Manager,
    test::{mock_builder, mock_context, noop_assets},
};

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

/// The registered handler on an app that manages the core: the unstarted ride is discarded and
/// its trainer released.
#[tokio::test(start_paused = true)]
async fn on_will_sleep_releases_through_the_managed_core() {
    let dir = tempfile::tempdir().unwrap();
    let core = core(dir.path()).await;
    core.ride_load(None).await.expect("loaded");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("mock app");
    app.manage(core.clone());

    sleep::on_will_sleep(app.handle());

    // The intent runs on Tauri's runtime (real time) while the core's timers follow this test's
    // paused clock: advance the clock and give the other runtime real time, until the ride is
    // gone and its trainer released (bounded, 10 s of real time), as `tests/lifecycle.rs` waits
    // for the quit. Both are awaited: the ride's unload is visible before the release that the
    // intent answers after (`links_intent`, ADR-0002 §5.1.3) has made the trainer `cold`.
    for _ in 0..2000 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if !core.ride_status().await.unwrap().status.loaded
            && trainer(&core).await == Some(LinkPhase::Cold)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
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

/// Without a managed core (it failed to open) the handler has nothing to release.
#[test]
fn on_will_sleep_without_a_core_does_nothing() {
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("mock app");
    sleep::on_will_sleep(app.handle());
    assert!(app.try_state::<Core>().is_none());
}
