//! ADR-0009 §2.8 "Core" for the athlete model (#715): the commands of §2.1.3 with every refusal of its table,
//! manual entry superseding open proposals (§2.2.4), and the snapshot frozen at `ride_load` (§2.1.4): the workout
//! compiles at its FTP, the plan and the record carry it, and a Settings change afterwards does not reach the ride.
use crank_core::{
    AthleteModel, Core, CoreConfig, DeviceMode, DeviceRole, EngineHost, ErrorCode, HistoryEntry,
};
use crank_devices::sim::SimConfig;
use crank_domain::athlete::{AthleteParam, METHOD_MANUAL, METHOD_PRIOR, Source, prior};
use crank_engine::{StreamRideState, VirtualClock};
use crank_store::{
    CompiledJson, EstimateOrigin, JsonColumn, NewProposal, NewRide, ProposalEvidenceJson,
    ProposalStatus, ProposedValue, RideAthleteJson, RideSummaryJson, RideWorkoutJson, Store,
};
use std::{path::Path, sync::Arc, time::Duration};

const SENSOR_CHECK: &str = "builtin-sensor-check";

fn config(dir: &Path) -> CoreConfig {
    CoreConfig {
        data_dir: dir.into(),
        devices: DeviceMode::Sim(SimConfig::default()),
        clock: Some(Arc::new(VirtualClock::new(1))),
        engine_host: EngineHost::Task,
    }
}

async fn pair_trainer(core: &Core) {
    let found = core.devices_scan(None).await.expect("scan");
    let trainer = found.iter().find(|d| d.trainer).expect("sim trainer");
    core.device_pair(trainer.id.clone(), DeviceRole::Trainer)
        .await
        .expect("pair trainer");
}

fn value_of(model: &AthleteModel, param: AthleteParam) -> Option<f64> {
    model
        .params
        .iter()
        .find(|p| p.param == param)
        .and_then(|p| p.resolved)
        .map(|r| r.value)
}

/// Equal up to rounding: a value that went through a compile or a JSON round trip.
fn same(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// The sensor check is 1 min at 50 %, 1 min at 80 %, 1 min at 40 % of FTP.
const SENSOR_CHECK_PCT: [f64; 3] = [0.5, 0.8, 0.4];

/// A library workout compiles at the snapshot's FTP, and the snapshot frozen at the load reaches the ride's plan,
/// its compile and its record; a Settings change after the load reaches neither, while previews follow the model.
#[tokio::test(start_paused = true)]
async fn the_ride_keeps_the_athlete_model_it_was_loaded_with() {
    let dir = tempfile::tempdir().expect("temp dir");
    let core = Core::open(config(dir.path())).expect("core");
    pair_trainer(&core).await;
    core.athlete_set("ftp".into(), 300., None)
        .await
        .expect("set FTP");
    core.athlete_set("lthr".into(), 165., Some(4.))
        .await
        .expect("set LTHR");
    core.athlete_set("cp".into(), 310., Some(10.))
        .await
        .expect("set CP");

    let ride_id = core
        .ride_load(Some(SENSOR_CHECK.into()))
        .await
        .expect("load")
        .ride_id;
    let plan = core.ride_plan(ride_id.clone()).await.expect("plan");
    assert!(same(plan.workout.ftp, 300.));
    assert!(same(plan.compiled.ftp_w, 300.));
    for (block, pct) in plan.compiled.blocks.iter().zip(SENSOR_CHECK_PCT) {
        assert!((block.power_from - pct * 300.).abs() < 1e-9, "{block:?}");
    }
    let snapshot = plan.athlete;
    let ftp = snapshot.ftp.expect("FTP");
    assert_eq!(
        (ftp.value, ftp.sd, ftp.source),
        (300., 60., Source::SelfReport)
    );
    assert_eq!(snapshot.lthr.map(|l| l.value), Some(165.));
    assert_eq!(snapshot.cp.map(|c| c.value), Some(310.));
    // W′ was never set: its prior, from no row.
    let w_prime = snapshot.w_prime.expect("W′ prior");
    assert_eq!(
        (w_prime.value, w_prime.source, w_prime.estimate_id),
        (prior::W_PRIME_J, Source::Prior, None)
    );

    // Settings changes while the ride is loaded, then while it runs: the ride keeps its model.
    core.athlete_set("ftp".into(), 200., None)
        .await
        .expect("set FTP");
    core.ride_start().await.expect("start");
    core.athlete_set("lthr".into(), 150., Some(4.))
        .await
        .expect("set LTHR");
    core.athlete_clear("cp".into()).await.expect("clear CP");
    assert_eq!(core.ride_plan(ride_id.clone()).await.expect("plan"), plan);

    // Previews follow the model, labelled with the FTP they assume.
    let preview = core.workout_get(SENSOR_CHECK.into()).await.expect("detail");
    assert_eq!((preview.workout.ftp, preview.compiled.ftp_w), (200., 200.));
    assert!((preview.compiled.blocks[0].power_from - 100.).abs() < 1e-9);
    let edited = core
        .compile_workout(preview.workout.clone())
        .await
        .expect("compile");
    assert!(same(edited.ftp_w, 200.));
    assert!(same(core.workout_new().await.expect("draft").ftp, 200.));
    let listed = core.workouts_list().await.expect("list");
    assert!(listed.iter().all(|w| same(w.ftp, 200.)), "{listed:?}");

    tokio::time::sleep(Duration::from_secs(185)).await;
    assert_eq!(
        core.ride_status().await.expect("status").status.state,
        StreamRideState::Finished
    );
    let ended = core.ride_end().await.expect("end").expect("a ride");
    assert_eq!(ended, ride_id);
    let summary = core
        .ride_summary(ride_id.clone())
        .await
        .expect("summary")
        .summary
        .expect("finished");
    for (block, pct) in summary.blocks.iter().zip(SENSOR_CHECK_PCT) {
        assert!((block.planned_w - pct * 300.).abs() < 1e-9, "{block:?}");
    }

    // The record holds the snapshot of the load, next to the workout it was ridden at.
    let store = Store::open_read_only(dir.path()).expect("reader");
    let ride = store.ride(&ride_id).expect("read").expect("stored");
    let stored = *ride.athlete.expect("athlete_json").snapshot();
    assert_eq!(stored.ftp.map(|f| f.value), Some(300.));
    assert_eq!(stored.lthr.map(|l| l.value), Some(165.));
    assert_eq!(stored.cp.map(|c| c.value), Some(310.));
    assert!(same(ride.workout.workout().ftp, 300.));
}

/// The free ride is loaded with the snapshot too: its plan and record carry it.
#[tokio::test(start_paused = true)]
async fn a_free_ride_carries_the_snapshot() {
    let dir = tempfile::tempdir().expect("temp dir");
    let core = Core::open(config(dir.path())).expect("core");
    pair_trainer(&core).await;
    core.athlete_set("weight".into(), 72., Some(1.))
        .await
        .expect("set weight");
    let ride_id = core.ride_load(None).await.expect("load").ride_id;
    let plan = core.ride_plan(ride_id).await.expect("plan");
    assert_eq!(plan.athlete.weight.map(|w| w.value), Some(72.));
    assert!(same(plan.workout.ftp, prior::FTP_W));
    assert!(same(plan.compiled.ftp_w, prior::FTP_W));
}

/// `athlete_model` resolves each parameter; set, clear and revert append rows that `athlete_history` lists newest
/// first.
#[tokio::test(start_paused = true)]
async fn set_history_revert_and_clear() {
    let dir = tempfile::tempdir().expect("temp dir");
    let core = Core::open(config(dir.path())).expect("core");
    let fresh = core.athlete_model().await.expect("model");
    assert_eq!(
        fresh.params.iter().map(|p| p.param).collect::<Vec<_>>(),
        AthleteParam::ALL
    );
    let ftp = &fresh.params[0];
    assert_eq!(ftp.resolved.map(|r| r.source), Some(Source::Prior));
    assert_eq!(value_of(&fresh, AthleteParam::Ftp), Some(prior::FTP_W));
    assert!(ftp.estimate.is_none());
    assert_eq!(value_of(&fresh, AthleteParam::Lthr), None);
    assert!(fresh.proposals.is_empty());

    core.athlete_set("ftp".into(), 287., None)
        .await
        .expect("set");
    let model = core.athlete_model().await.expect("model");
    let set = model.params[0].estimate.clone().expect("the row");
    assert_eq!(
        (set.value, set.sd, set.source, set.method.as_str()),
        (287., 287. * 0.2, Source::SelfReport, METHOD_MANUAL)
    );
    assert_eq!(
        model.params[0].resolved.and_then(|r| r.estimate_id),
        Some(set.id)
    );

    core.athlete_set("ftp".into(), 300., Some(15.))
        .await
        .expect("set");
    core.athlete_revert(set.id).await.expect("revert");
    assert_eq!(
        value_of(
            &core.athlete_model().await.expect("model"),
            AthleteParam::Ftp
        ),
        Some(287.)
    );
    core.athlete_clear("ftp".into()).await.expect("clear");
    let history = core.athlete_history("ftp".into()).await.expect("history");
    let rows: Vec<(f64, Source, &str)> = history
        .iter()
        .map(|h| match h {
            HistoryEntry::Estimate(e) => (e.value, e.source, e.method.as_str()),
            HistoryEntry::Cleared { .. } => panic!("FTP has a prior: {h:?}"),
        })
        .collect();
    assert_eq!(
        rows,
        [
            (prior::FTP_W, Source::Prior, METHOD_PRIOR),
            (287., Source::SelfReport, METHOD_MANUAL),
            (300., Source::SelfReport, METHOD_MANUAL),
            (287., Source::SelfReport, METHOD_MANUAL),
        ]
    );

    // A parameter with no prior clears to unset.
    core.athlete_set("lthr".into(), 160., None)
        .await
        .expect("set");
    core.athlete_clear("lthr".into()).await.expect("clear");
    assert_eq!(
        value_of(
            &core.athlete_model().await.expect("model"),
            AthleteParam::Lthr
        ),
        None
    );
    let lthr = core.athlete_history("lthr".into()).await.expect("history");
    assert!(matches!(lthr[0], HistoryEntry::Cleared { .. }), "{lthr:?}");
}

/// Every refusal of ADR-0009 §2.1.3's table.
#[tokio::test(start_paused = true)]
async fn the_commands_refuse_what_the_model_refuses() {
    let dir = tempfile::tempdir().expect("temp dir");
    let core = Core::open(config(dir.path())).expect("core");
    let refused = |r: Result<(), crank_core::ApiError>| r.expect_err("refused").code;

    // athlete_set: unknown param, bounds (never clamped), non-finite, sd, LTHR ≥ HR max.
    assert_eq!(
        refused(core.athlete_set("vo2".into(), 60., None).await),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        refused(core.athlete_set("ftp".into(), 49., None).await),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        refused(core.athlete_set("ftp".into(), 601., None).await),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        refused(core.athlete_set("ftp".into(), f64::NAN, None).await),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        refused(
            core.athlete_set("ftp".into(), 250., Some(f64::INFINITY))
                .await
        ),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        refused(core.athlete_set("ftp".into(), 250., Some(0.)).await),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        refused(core.athlete_set("ftp".into(), 250., Some(126.)).await),
        ErrorCode::InvalidInput
    );
    core.athlete_set("hr_max".into(), 180., Some(3.))
        .await
        .expect("set");
    assert_eq!(
        refused(core.athlete_set("lthr".into(), 180., Some(4.)).await),
        ErrorCode::InvalidInput
    );
    core.athlete_set("lthr".into(), 170., Some(4.))
        .await
        .expect("set");
    assert_eq!(
        refused(core.athlete_set("hr_max".into(), 170., Some(3.)).await),
        ErrorCode::InvalidInput
    );
    // Nothing refused was written.
    let model = core.athlete_model().await.expect("model");
    assert_eq!(value_of(&model, AthleteParam::Ftp), Some(prior::FTP_W));
    assert_eq!(value_of(&model, AthleteParam::HrMax), Some(180.));
    assert_eq!(value_of(&model, AthleteParam::Lthr), Some(170.));

    // athlete_history and athlete_clear: unknown param.
    assert_eq!(
        core.athlete_history("vo2".into())
            .await
            .expect_err("refused")
            .code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        refused(core.athlete_clear("vo2".into()).await),
        ErrorCode::InvalidInput
    );

    // athlete_revert: an unknown row; a prior row (going back to the prior is Clear); a cleared row.
    assert_eq!(
        refused(core.athlete_revert(9_999).await),
        ErrorCode::NotFound
    );
    core.athlete_clear("ftp".into()).await.expect("clear");
    let prior_row = match &core.athlete_history("ftp".into()).await.expect("history")[0] {
        HistoryEntry::Estimate(e) if e.source == Source::Prior => e.id,
        h => panic!("expected the prior row: {h:?}"),
    };
    assert_eq!(
        refused(core.athlete_revert(prior_row).await),
        ErrorCode::InvalidInput
    );
    core.athlete_clear("weight".into()).await.expect("clear");
    let cleared = match &core
        .athlete_history("weight".into())
        .await
        .expect("history")[0]
    {
        HistoryEntry::Cleared { id, .. } => *id,
        h @ HistoryEntry::Estimate(_) => panic!("expected a cleared row: {h:?}"),
    };
    assert_eq!(
        refused(core.athlete_revert(cleared).await),
        ErrorCode::InvalidInput
    );
}

/// `athlete_set`, `athlete_revert` and `athlete_clear` mark the parameter's open proposals `superseded` (ADR-0009
/// §2.2.4 "Manual entry wins"), and only that parameter's.
#[tokio::test(start_paused = true)]
async fn manual_entry_supersedes_the_parameters_open_proposals() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (hr_max_row, proposals) = {
        let store = Store::open_owner(dir.path()).expect("owner");
        let workout = r#"{"v":1,"id":"w","name":"Ride","ftp":250,"nodes":[]}"#;
        store
            .start_ride(NewRide {
                id: "r".into(),
                started_at: 100,
                workout: RideWorkoutJson::parse(workout).expect("workout"),
                compiled: CompiledJson::parse(r#"{"v":1}"#).expect("compiled"),
                athlete: RideAthleteJson::new(&store.athlete_snapshot().expect("model"))
                    .expect("snapshot"),
                trainer_device_id: None,
                hr_device_id: None,
                telemetry_source: "sim".into(),
                metrics: vec!["power".into()],
            })
            .expect("ride");
        store
            .finish_ride(
                "r",
                160,
                RideSummaryJson::parse(r#"{"v":1}"#).expect("summary"),
            )
            .expect("finish");
        let hr_max_row = store
            .append_athlete(
                EstimateOrigin::Set {
                    param: AthleteParam::HrMax,
                    value: 185.,
                    sd: 3.,
                },
                200,
            )
            .expect("HR max")[0]
            .id();
        let raise = |method: &str, param: AthleteParam, value: f64, sd: f64| {
            store
                .insert_proposal(NewProposal {
                    ride_id: "r".into(),
                    method: method.into(),
                    generation: 1,
                    values: vec![ProposedValue { param, value, sd }],
                    evidence: ProposalEvidenceJson::parse(r#"{"v":1}"#).expect("evidence"),
                    analysis_version: 1,
                    created_at: 300,
                })
                .expect("proposal")
        };
        let proposals = [
            raise("ramp_v1_0.75", AthleteParam::Ftp, 270., 21.6),
            raise("peak_hr_v1", AthleteParam::HrMax, 192., 2.),
            raise("lthr_erg_steady_v1", AthleteParam::Lthr, 168., 5.),
            raise("peak_hr_v1_other", AthleteParam::Weight, 70., 1.),
        ];
        (hr_max_row, proposals)
    };
    let core = Core::open(config(dir.path())).expect("core");
    let open = || async {
        core.athlete_model()
            .await
            .expect("model")
            .proposals
            .iter()
            .map(|p| i64::from(p.id))
            .collect::<Vec<_>>()
    };
    assert_eq!(open().await, proposals);

    core.athlete_set("ftp".into(), 280., None)
        .await
        .expect("set");
    assert_eq!(open().await, proposals[1..]);
    core.athlete_revert(u32::try_from(hr_max_row).expect("small id"))
        .await
        .expect("revert");
    assert_eq!(open().await, proposals[2..]);
    core.athlete_clear("lthr".into()).await.expect("clear");
    assert_eq!(open().await, proposals[3..]);

    let reader = Store::open_read_only(dir.path()).expect("reader");
    for id in &proposals[..3] {
        assert_eq!(
            reader.proposal(*id).expect("read").expect("kept").status,
            ProposalStatus::Superseded,
            "proposal {id}"
        );
    }
    assert_eq!(
        reader
            .proposal(proposals[3])
            .expect("read")
            .expect("kept")
            .status,
        ProposalStatus::Open
    );
}
