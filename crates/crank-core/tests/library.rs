//! The workout library's save loop (#110, DESIGN §5.7): a new draft, save, update, list,
//! delete, persistence across a reopen, the refusals, and riding a saved workout by its id.
use crank_core::{
    Core, CoreConfig, DeviceMode, DeviceRole, DslParse, EngineHost, ErrorCode, WorkoutDraft,
    WorkoutNode,
};
use crank_devices::sim::SimConfig;
use crank_domain::workout::Severity;
use crank_engine::VirtualClock;
use std::{path::Path, sync::Arc};

const SENSOR_CHECK: &str = "builtin-sensor-check";

fn config(dir: &Path) -> CoreConfig {
    CoreConfig {
        data_dir: dir.into(),
        devices: DeviceMode::Sim(SimConfig::default()),
        clock: Some(Arc::new(VirtualClock::new(1))),
        engine_host: EngineHost::Task,
    }
}

async fn nodes(core: &Core, dsl: &str) -> Vec<WorkoutNode> {
    match core
        .parse_dsl(dsl.into(), 250.0)
        .await
        .expect("parse_dsl answers")
    {
        DslParse::Parsed { nodes } => nodes,
        DslParse::Invalid { problem } => panic!("{dsl}: {problem:?}"),
    }
}

async fn draft(core: &Core, name: &str, dsl: &str) -> WorkoutDraft {
    WorkoutDraft {
        name: name.into(),
        ftp: 250.0,
        nodes: nodes(core, dsl).await,
    }
}

#[tokio::test(start_paused = true)]
async fn save_update_list_delete_and_survive_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path())).unwrap();
    let builtins = core.workouts_list().await.unwrap();
    assert!(builtins.iter().all(|w| w.builtin));

    // A new workout starts empty, at the reference FTP; empty is not saveable.
    let new = core.workout_new().await.unwrap();
    assert!(new.nodes.is_empty());
    assert_eq!(new.name, "New workout");
    let empty = core.workout_save(None, new).await.unwrap_err();
    assert_eq!(empty.code, ErrorCode::InvalidInput);
    assert_eq!(empty.message, "Workout is empty");

    // Save as new: listed after the built-ins, the name trimmed, its tree and text kept.
    let id = core
        .workout_save(
            None,
            draft(&core, "  Openers  ", "\"Go\" 2m 70%, \"Easy\" 1m 50%").await,
        )
        .await
        .unwrap();
    assert!(!id.starts_with("builtin-"), "{id}");
    let list = core.workouts_list().await.unwrap();
    assert_eq!(list.len(), builtins.len() + 1);
    let mine = list.last().unwrap();
    assert_eq!(
        (mine.id.as_str(), mine.name.as_str()),
        (id.as_str(), "Openers")
    );
    assert!(!mine.builtin);
    assert!((mine.total_s - 180.0).abs() < 1e-9);
    let d = core.workout_get(id.clone()).await.unwrap();
    assert!(!d.builtin);
    assert_eq!(d.workout.id, id);
    assert_eq!(d.dsl, "\"Go\" 2m 70%\n\"Easy\" 1m 50%");
    assert_eq!(d.compiled.blocks.len(), 2);

    // Save (an update) keeps the id and the place in the list.
    let again = core
        .workout_save(
            Some(id.clone()),
            draft(&core, "Openers v2", "\"Go\" 3m 70%").await,
        )
        .await
        .unwrap();
    assert_eq!(again, id);
    // A second save as new is a second workout.
    let other = core
        .workout_save(None, draft(&core, "Openers v2", "\"Go\" 3m 70%").await)
        .await
        .unwrap();
    assert_ne!(other, id);
    let names: Vec<_> = core.workouts_list().await.unwrap()[builtins.len()..]
        .iter()
        .map(|w| (w.id.clone(), w.name.clone()))
        .collect();
    assert_eq!(
        names,
        [
            (id.clone(), "Openers v2".into()),
            (other.clone(), "Openers v2".into())
        ]
    );

    // Delete one; it is gone everywhere, and a second delete is not found.
    core.workout_delete(other.clone()).await.unwrap();
    assert_eq!(
        core.workout_get(other.clone()).await.unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        core.workout_delete(other.clone()).await.unwrap_err().code,
        ErrorCode::NotFound
    );
    // Updating a workout that is gone does not bring it back.
    assert_eq!(
        core.workout_save(Some(other.clone()), draft(&core, "Back", "1m 50%").await)
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );

    // Reopen on the same data dir: the saved workout is there, as saved.
    drop(core);
    let core = Core::open(config(dir.path())).unwrap();
    let list = core.workouts_list().await.unwrap();
    assert_eq!(list.len(), builtins.len() + 1);
    let d = core.workout_get(id.clone()).await.unwrap();
    assert_eq!(d.workout.name, "Openers v2");
    assert_eq!(d.dsl, "\"Go\" 3m 70%");
}

#[tokio::test(start_paused = true)]
async fn save_and_delete_refuse_what_the_library_does_not_take() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path())).unwrap();
    let refused = |r: Result<String, crank_core::ApiError>| r.unwrap_err();

    for name in ["", "   "] {
        let e = refused(
            core.workout_save(None, draft(&core, name, "1m 50%").await)
                .await,
        );
        assert_eq!(
            (e.code, e.message.as_str()),
            (ErrorCode::InvalidInput, "a workout needs a name")
        );
    }
    let long = "x".repeat(121);
    let e = refused(
        core.workout_save(None, draft(&core, &long, "1m 50%").await)
            .await,
    );
    assert_eq!(e.code, ErrorCode::InvalidInput);
    // 120 characters (not bytes) is the limit.
    core.workout_save(None, draft(&core, &"é".repeat(120), "1m 50%").await)
        .await
        .unwrap();

    // Built-ins are read-only.
    let e = refused(
        core.workout_save(
            Some(SENSOR_CHECK.into()),
            draft(&core, "Mine", "1m 50%").await,
        )
        .await,
    );
    assert_eq!(e.code, ErrorCode::InvalidInput);
    assert!(e.message.contains("read-only"), "{}", e.message);
    assert_eq!(
        core.workout_delete(SENSOR_CHECK.into())
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        core.workout_get(SENSOR_CHECK.into())
            .await
            .unwrap()
            .workout
            .name,
        "Sensor check (3 min)"
    );

    // An error-severity issue is refused with its message, as ride_load refuses it; a
    // warning is saved.
    let mut bad = draft(&core, "No FTP", "1m 50%").await;
    bad.ftp = 0.0;
    let e = refused(core.workout_save(None, bad).await);
    assert_eq!(
        (e.code, e.message.as_str()),
        (ErrorCode::InvalidInput, "FTP must be positive")
    );
    let warned = draft(&core, "Long, no cool-down", "25m 60%").await;
    let id = core.workout_save(None, warned).await.unwrap();
    let issues = core.workout_get(id).await.unwrap().compiled.issues;
    assert!(
        !issues.is_empty() && issues.iter().all(|i| i.severity == Severity::Warning),
        "{issues:?}"
    );

    // A tree that does not compile at all (a nonfinite FTP) is refused.
    let mut nan = draft(&core, "NaN", "1m 50%").await;
    nan.ftp = f64::NAN;
    assert_eq!(
        refused(core.workout_save(None, nan).await).code,
        ErrorCode::InvalidInput
    );

    // Nothing refused was stored: the list has the built-ins and the two saved above.
    assert_eq!(
        core.workouts_list()
            .await
            .unwrap()
            .iter()
            .filter(|w| !w.builtin)
            .count(),
        2
    );
}

#[tokio::test(start_paused = true)]
async fn ride_this_loads_the_saved_workout_by_its_id() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path())).unwrap();
    let found = core.devices_scan(None).await.unwrap();
    let trainer = found.iter().find(|d| d.trainer).unwrap();
    core.device_pair(trainer.id.clone(), DeviceRole::Trainer)
        .await
        .unwrap();
    let id = core
        .workout_save(None, draft(&core, "Short", "\"Go\" 1m 60%").await)
        .await
        .unwrap();
    let loaded = core.ride_load(Some(id.clone())).await.unwrap();
    assert_eq!(loaded.workout_id.as_deref(), Some(id.as_str()));
    assert!(core.ride_status().await.unwrap().status.loaded);
}

/// #110: a loaded ride runs, and shows, the workout as loaded: editing or deleting the library
/// workout afterwards leaves the ride's plan unchanged, before and after Start.
#[tokio::test(start_paused = true)]
async fn editing_or_deleting_the_workout_leaves_the_loaded_ride_alone() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path())).unwrap();
    let found = core.devices_scan(None).await.unwrap();
    let trainer = found.iter().find(|d| d.trainer).unwrap();
    core.device_pair(trainer.id.clone(), DeviceRole::Trainer)
        .await
        .unwrap();
    let id = core
        .workout_save(
            None,
            draft(&core, "Two", "\"A\" 1m 60%, \"B\" 1m 70%").await,
        )
        .await
        .unwrap();
    let ride = core.ride_load(Some(id.clone())).await.unwrap().ride_id;
    let plan = core.ride_plan(ride.clone()).await.unwrap();
    assert_eq!(
        (plan.ride_id.as_str(), plan.workout.name.as_str()),
        (ride.as_str(), "Two")
    );
    assert_eq!(plan.compiled.blocks.len(), 2);

    // Loaded, not started: an edit changes the library, not the ride.
    core.workout_save(
        Some(id.clone()),
        draft(&core, "Three", "\"A\" 1m 60%, \"B\" 1m 70%, \"C\" 1m 50%").await,
    )
    .await
    .unwrap();
    assert_eq!(
        core.workout_get(id.clone())
            .await
            .unwrap()
            .compiled
            .blocks
            .len(),
        3
    );
    assert_eq!(core.ride_plan(ride.clone()).await.unwrap(), plan);

    // Started, then the workout deleted: the ride still has its plan.
    core.ride_start().await.unwrap();
    core.workout_delete(id.clone()).await.unwrap();
    assert_eq!(core.ride_plan(ride.clone()).await.unwrap(), plan);
    // Another id is not found.
    assert_eq!(
        core.ride_plan("ride-other".into()).await.unwrap_err().code,
        ErrorCode::NotFound
    );
}
