//! Facade acceptance for devices and workouts (#34 B1, ADR-0001 §4) on the simulator.
use crank_core::{ApiError, Core, CoreConfig, DeviceMode, DeviceRole, ErrorCode};
use crank_devices::sim::SimConfig;
use crank_engine::VirtualClock;
use std::{path::Path, sync::Arc};

fn config(dir: &Path) -> CoreConfig {
    CoreConfig {
        data_dir: dir.into(),
        devices: DeviceMode::Sim(SimConfig::default()),
        clock: Some(Arc::new(VirtualClock::new(1))),
    }
}
fn code(e: &ApiError) -> ErrorCode {
    e.code
}

#[tokio::test(start_paused = true)]
async fn scan_pair_forget_and_links_on_the_sim() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path())).unwrap();
    let info = core.core_ready().await.unwrap();
    assert!(info.sim, "sim is a runtime config: no hardware is touched");
    assert!(info.schema_version >= 1);

    let found = core.devices_scan(None).await.unwrap();
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|d| !d.paired));
    let trainer = found.iter().find(|d| d.trainer).unwrap().clone();
    let strap = found.iter().find(|d| d.heart_rate).unwrap().clone();

    let t = core
        .device_pair(trainer.id.clone(), DeviceRole::Trainer)
        .await
        .unwrap();
    assert_eq!(
        (t.role, t.id.as_str()),
        (DeviceRole::Trainer, trainer.id.as_str())
    );
    // Pairing releases the device: it can be paired again and scanned again.
    core.device_pair(trainer.id.clone(), DeviceRole::Trainer)
        .await
        .unwrap();
    core.device_pair(strap.id.clone(), DeviceRole::HeartRate)
        .await
        .unwrap();
    assert!(
        core.devices_scan(None)
            .await
            .unwrap()
            .iter()
            .all(|d| d.paired)
    );

    let links = core.device_links().await.unwrap();
    assert_eq!(links.len(), 2);
    assert!(links.iter().all(|l| l.device.is_some() && l.link.is_none()));

    // Forgetting is per role.
    core.device_forget(DeviceRole::HeartRate).await.unwrap();
    let links = core.device_links().await.unwrap();
    assert!(links[0].device.is_some() && links[1].device.is_none());
    assert_eq!(
        code(&core.device_forget(DeviceRole::HeartRate).await.unwrap_err()),
        ErrorCode::NotFound
    );
}

#[tokio::test(start_paused = true)]
async fn pairing_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let core = Core::open(config(dir.path())).unwrap();
        core.device_pair("trainer".into(), DeviceRole::Trainer)
            .await
            .unwrap();
    }
    let core = Core::open(config(dir.path())).unwrap();
    let links = core.device_links().await.unwrap();
    assert_eq!(links[0].device.as_ref().unwrap().id, "trainer");
}

#[tokio::test(start_paused = true)]
async fn invalid_requests_are_typed_errors() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path())).unwrap();
    assert_eq!(
        code(
            &core
                .device_pair(String::new(), DeviceRole::Trainer)
                .await
                .unwrap_err()
        ),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        code(&core.workout_get("nope".into()).await.unwrap_err()),
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn built_in_workouts_list_and_compile() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path())).unwrap();
    let list = core.workouts_list().await.unwrap();
    assert!(list.len() >= 3);
    for w in &list {
        let d = core.workout_get(w.id.clone()).await.unwrap();
        assert_eq!(d.workout.id, w.id);
        assert!(!d.compiled.blocks.is_empty(), "{}", w.id);
        assert!((d.compiled.total_s - w.total_s).abs() < f64::EPSILON);
        assert!(!d.dsl.is_empty());
    }
    let check = core
        .workout_get("builtin-sensor-check".into())
        .await
        .unwrap();
    assert_eq!(check.compiled.blocks.len(), 3);
    assert!((check.compiled.total_s - 180.0).abs() < 1e-9);
}

#[test]
fn a_second_core_on_the_same_data_dir_gets_a_typed_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let _first = Core::open(config(dir.path())).unwrap();
    let err = Core::open(config(dir.path())).err().expect("lock is held");
    assert_eq!(err.code, ErrorCode::Conflict);
    assert!(err.message.contains("ride.lock"), "{}", err.message);
}

/// Child half of the two-process test: only runs when the parent sets the data dir.
#[test]
fn lock_contender_child() {
    let Ok(dir) = std::env::var("CRANK_LOCK_CHILD_DIR") else {
        return;
    };
    let err = Core::open(config(Path::new(&dir)))
        .err()
        .expect("lock is held");
    assert_eq!(err.code, ErrorCode::Conflict);
}
#[test]
fn another_process_cannot_take_ride_lock() {
    let dir = tempfile::tempdir().unwrap();
    let _owner = Core::open(config(dir.path())).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_contender_child", "--nocapture"])
        .env("CRANK_LOCK_CHILD_DIR", dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "child: {status}");
}
