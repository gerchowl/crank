//! The command layer (ADR-0001 §4) on the simulator: a whole ride driven through the
//! Tauri command functions, with the managed `Core` and a window of the mock runtime,
//! and frames arriving on a real IPC `Channel`.
use crank_core::{
    Core, CoreConfig, DeviceLinkView, DeviceMode, DeviceRole, DslParse, EngineHost, ErrorCode,
    LinkFrame, LinkIntent, LinkPhase, ReleaseReason, RideEnding, RideOutcome, UiFrame,
};
use crank_desktop::commands as cmd;
use crank_devices::sim::SimConfig;
use crank_engine::{StreamFrame, StreamRideState, VirtualClock};
use serde_json::Value;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{
    Manager, State, WebviewUrl, WebviewWindowBuilder,
    ipc::{Channel, InvokeResponseBody},
    test::{MockRuntime, mock_builder, mock_context, noop_assets},
};

const SENSOR_CHECK: &str = "builtin-sensor-check";

type Frames = Arc<Mutex<Vec<StreamFrame>>>;
type UiFrames = Arc<Mutex<Vec<UiFrame>>>;

fn app(dir: &std::path::Path) -> tauri::App<MockRuntime> {
    let core = Core::open(CoreConfig {
        data_dir: dir.into(),
        devices: DeviceMode::Sim(SimConfig::default()),
        clock: Some(Arc::new(VirtualClock::new(1))),
        engine_host: EngineHost::Task,
    })
    .expect("core opens");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("mock app");
    app.manage(core);
    app.manage(crank_desktop::logging::Logging::default());
    for label in ["main", "other"] {
        // Linux defaults the webview data dir under $HOME, which a build sandbox does not own.
        WebviewWindowBuilder::new(&app, label, WebviewUrl::default())
            .data_directory(dir.join(format!("webview-{label}")))
            .build()
            .expect("window");
    }
    app
}
fn win(app: &tauri::App<MockRuntime>, label: &str) -> tauri::Window<MockRuntime> {
    app.get_webview_window(label)
        .expect("window")
        .as_ref()
        .window()
}
/// A channel that decodes what the core sends, as the page would.
fn channel(frames: &Frames) -> Channel<StreamFrame> {
    let frames = frames.clone();
    Channel::new(move |body| {
        let InvokeResponseBody::Json(json) = body else {
            panic!("frames are JSON");
        };
        frames
            .lock()
            .expect("test mutex")
            .push(serde_json::from_str(&json).expect("a StreamFrame"));
        Ok(())
    })
}

/// A channel that decodes the snapshot frames the core sends, as the page would.
fn ui_channel(frames: &UiFrames) -> Channel<UiFrame> {
    let frames = frames.clone();
    Channel::new(move |body| {
        let InvokeResponseBody::Json(json) = body else {
            panic!("frames are JSON");
        };
        frames
            .lock()
            .expect("test mutex")
            .push(serde_json::from_str(&json).expect("a UiFrame"));
        Ok(())
    })
}

/// A whole ride through the commands, its live state on the snapshot channel (ADR-0001
/// §4.3.8): `ride_load` answers the ride id, `device_pair` and `ride_end` answer `()`, and
/// what they did is in the next frames; another window's ack is `NotFound`.
#[tokio::test(start_paused = true)]
async fn a_whole_ride_through_the_commands_streams_frames_and_lands_in_history() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let core = app.state::<Core>();
    let window = win(&app, "main");

    let info = cmd::core_ready(core.clone()).await.unwrap();
    assert!(info.sim);
    let frames: UiFrames = Arc::default();
    let channel = cmd::ui_open(core.clone(), window.clone(), ui_channel(&frames))
        .await
        .unwrap();
    // Apply and ack every frame as the page does (ADR-0001 §4.3.2), checking the series rule.
    let mut page = Page::default();
    let found = cmd::devices_scan(core.clone(), None).await.unwrap();
    let trainer = found.iter().find(|d| d.trainer).unwrap().id.clone();
    let strap = found.iter().find(|d| d.heart_rate).unwrap().id.clone();
    let () = cmd::device_pair(core.clone(), trainer.clone(), DeviceRole::Trainer)
        .await
        .unwrap();
    let () = cmd::device_pair(core.clone(), strap, DeviceRole::HeartRate)
        .await
        .unwrap();
    page.pump(&core, &window, &frames, 3).await;
    let paired = page.last().devices.trainer.paired.clone();
    assert_eq!(
        paired.map(|p| p.id),
        Some(trainer),
        "the pairing is in the frame"
    );
    let library = cmd::workouts_list(core.clone()).await.unwrap();
    assert!(library.iter().any(|w| w.id == SENSOR_CHECK));
    cmd::workout_get(core.clone(), SENSOR_CHECK.into())
        .await
        .unwrap();

    let id: String = cmd::ride_load(core.clone(), Some(SENSOR_CHECK.into()))
        .await
        .unwrap();
    cmd::ride_start(core.clone()).await.unwrap();
    page.pump(&core, &window, &frames, 20).await;
    {
        let last = page.last();
        let ride = last.ride.as_ref().expect("the ride is in the frame");
        assert_eq!(ride.id, id, "the frame's ride is the loaded one");
        assert_eq!(ride.status.state, StreamRideState::Riding);
        assert!(page.power.len() >= 20, "{} power points", page.power.len());
        assert!(page.power.iter().any(|&w| w > 0), "power points arrive");
    }

    // A window may only ack its own channel.
    let other = win(&app, "other");
    let e = cmd::ui_ack(core.clone(), other, channel, page.last().seq)
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);

    cmd::ride_pause(core.clone()).await.unwrap();
    cmd::ride_resume(core.clone()).await.unwrap();
    cmd::ride_skip(core.clone()).await.unwrap();
    let () = cmd::ride_end(core.clone(), app.state()).await.unwrap();
    page.pump(&core, &window, &frames, 3).await;
    let ended = page.last().ride.as_ref().map(|r| (r.id.clone(), r.ended));
    assert_eq!(ended, Some((id.clone(), Some(RideEnding::Finished))));

    let rides = cmd::rides_list(core.clone()).await.unwrap();
    assert_eq!(rides.len(), 1);
    assert_eq!(rides[0].outcome, RideOutcome::Finished);
    assert_eq!(cmd::ride_summary(core.clone(), id).await.unwrap(), rides[0]);
    cmd::device_forget(core.clone(), DeviceRole::HeartRate)
        .await
        .unwrap();
}

/// The page's side of the snapshot channel, by the client rule (ADR-0001 §4.3.2).
#[derive(Default)]
struct Page {
    applied: usize,
    frames: Vec<UiFrame>,
    power: Vec<u16>,
}
impl Page {
    /// For `secs` seconds, apply each new frame (asserting `from` is the page's length) and
    /// ack it.
    async fn pump(
        &mut self,
        core: &State<'_, Core>,
        window: &tauri::Window<MockRuntime>,
        frames: &UiFrames,
        secs: u32,
    ) {
        for _ in 0..secs * 10 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let new: Vec<UiFrame> = frames.lock().expect("test mutex")[self.applied..].to_vec();
            for frame in new {
                self.applied += 1;
                let p = &frame.series.power;
                if p.from == 0 {
                    self.power.clear();
                }
                assert_eq!(
                    p.from as usize,
                    self.power.len(),
                    "from is the page's length"
                );
                self.power.extend(&p.v);
                cmd::ui_ack(core.clone(), window.clone(), frame.channel, frame.seq)
                    .await
                    .expect("the page's own channel takes its ack");
                self.frames.push(frame);
            }
        }
    }
    fn last(&self) -> &UiFrame {
        self.frames.last().expect("a frame arrived")
    }
}

#[tokio::test(start_paused = true)]
async fn a_destroyed_window_loses_its_streams() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let core = app.state::<Core>();
    let window = win(&app, "main");
    for (id, role) in [("trainer", DeviceRole::Trainer)] {
        cmd::device_pair(core.clone(), id.into(), role)
            .await
            .unwrap();
    }
    cmd::ride_load(core.clone(), None).await.unwrap();
    let frames: Frames = Arc::default();
    let stream = cmd::open_stream(core.clone(), window.clone(), channel(&frames))
        .await
        .unwrap();
    core.close_window("main".into()).await;
    let e = cmd::stream_snapshot(core.clone(), window, stream)
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    cmd::ride_end(core.clone(), app.state()).await.unwrap();
}

/// The link contract through the commands (ADR-0002 §5.1.4): `links_open` sends the keeper's
/// state on a real IPC channel, the next frame waits for `links_ack`, an intent's effect
/// arrives as the next frame, another window's ack is refused, and "Release trainer" is
/// refused in a started ride.
#[tokio::test(start_paused = true)]
async fn the_link_commands_carry_the_keepers_state() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let core = app.state::<Core>();
    let (main, other) = (win(&app, "main"), win(&app, "other"));
    cmd::device_pair(core.clone(), "trainer".into(), DeviceRole::Trainer)
        .await
        .unwrap();
    let frames: Arc<Mutex<Vec<LinkFrame>>> = Arc::default();
    let sink = frames.clone();
    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Json(json) = body else {
            panic!("frames are JSON");
        };
        sink.lock()
            .expect("test mutex")
            .push(serde_json::from_str(&json).expect("a LinkFrame"));
        Ok(())
    });
    let id = cmd::links_open(core.clone(), main.clone(), channel)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    let first = frames.lock().unwrap().clone();
    assert_eq!(first.len(), 1);
    let cold = Some(DeviceLinkView {
        phase: LinkPhase::Cold,
        sample: None,
    });
    assert_eq!(first[0].links.trainer, cold);
    assert_eq!(first[0].links.strap, None, "no strap paired");

    cmd::links_intent(core.clone(), LinkIntent::Likely { ready: false })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(frames.lock().unwrap().len(), 1, "one frame in flight");
    let refused = cmd::links_ack(core.clone(), other, id, first[0].seq)
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::NotFound);
    cmd::links_ack(core.clone(), main.clone(), id, first[0].seq)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    let latest = *frames.lock().unwrap().last().unwrap();
    assert!(latest.seq > first[0].seq, "the next frame is a newer state");
    assert_eq!(latest.links.trainer.map(|t| t.phase), Some(LinkPhase::Warm));

    cmd::ride_load(core.clone(), None).await.unwrap();
    cmd::ride_start(core.clone()).await.unwrap();
    let held = cmd::links_intent(
        core.clone(),
        LinkIntent::Release {
            reason: ReleaseReason::RiderRelease,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(held.code, ErrorCode::Conflict);
    cmd::ride_end(core.clone(), app.state()).await.unwrap();
}

/// `links_intent`'s answer as the IPC layer serialises it for the page.
async fn answered(core: State<'_, Core>, intent: LinkIntent) -> Value {
    let answer = cmd::links_intent(core, intent).await;
    serde_json::to_value(
        answer
            .as_ref()
            .unwrap_or_else(|e| panic!("{intent:?}: {e:?}")),
    )
    .expect("serialisable")
}

/// `links_intent` answers nothing (ADR-0002 §5.1.4, ADR-0001 §4.3.10 step 2, #597): through
/// the commands its answer is `null` on the wire for every intent, idle, over a loaded ride
/// (whose "Release trainer" discards it), in a started ride and after the quit. Its
/// refusals are unchanged: "Release trainer" in a started ride and `Likely` / `Retry` after
/// the quit are `Conflict`.
#[tokio::test(start_paused = true)]
async fn links_intent_answers_null_for_every_intent_and_keeps_its_refusals() {
    use ReleaseReason::{Devices, Hidden, RiderRelease, Sleep};
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let core = app.state::<Core>();
    cmd::device_pair(core.clone(), "trainer".into(), DeviceRole::Trainer)
        .await
        .unwrap();
    let mut every = vec![
        LinkIntent::Likely { ready: false },
        LinkIntent::Likely { ready: true },
        LinkIntent::Retry,
        LinkIntent::Left,
        LinkIntent::Background,
    ];
    every.extend(
        [Hidden, Sleep, Devices, RiderRelease].map(|reason| LinkIntent::Release { reason }),
    );
    let rider_release = LinkIntent::Release {
        reason: RiderRelease,
    };

    for &intent in &every {
        assert_eq!(
            answered(core.clone(), intent).await,
            Value::Null,
            "idle, {intent:?}"
        );
    }

    cmd::ride_load(core.clone(), None).await.unwrap();
    assert_eq!(
        answered(core.clone(), rider_release).await,
        Value::Null,
        "a discard"
    );
    assert!(!cmd::ride_status(core.clone()).await.unwrap().status.loaded);

    cmd::ride_load(core.clone(), None).await.unwrap();
    cmd::ride_start(core.clone()).await.unwrap();
    for &intent in &every {
        if intent == rider_release {
            let held = cmd::links_intent(core.clone(), intent).await.unwrap_err();
            assert_eq!(held.code, ErrorCode::Conflict);
            assert_eq!(
                held.message,
                "the ride holds the trainer: end the ride first"
            );
        } else {
            assert_eq!(
                answered(core.clone(), intent).await,
                Value::Null,
                "riding, {intent:?}"
            );
        }
    }
    cmd::ride_end(core.clone(), app.state()).await.unwrap();

    core.shutdown(Duration::from_secs(3)).await;
    for &intent in &every {
        if matches!(intent, LinkIntent::Likely { .. } | LinkIntent::Retry) {
            let quitting = cmd::links_intent(core.clone(), intent).await.unwrap_err();
            assert_eq!(quitting.code, ErrorCode::Conflict);
            assert_eq!(quitting.message, "the application is quitting");
        } else {
            assert_eq!(
                answered(core.clone(), intent).await,
                Value::Null,
                "quit, {intent:?}"
            );
        }
    }
}

/// The editor's commands (#111) on a library workout: its tree prints to its DSL, the DSL
/// parses back to a tree that prints the same, and compiling the tree gives the library's
/// compiled view; a parse answer is tagged by `outcome` on the wire.
#[tokio::test]
async fn the_editor_commands_agree_with_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let core = app.state::<Core>();
    for w in cmd::workouts_list(core.clone()).await.unwrap() {
        let d = cmd::workout_get(core.clone(), w.id.clone()).await.unwrap();
        let printed = cmd::print_dsl(core.clone(), d.workout.nodes.clone())
            .await
            .unwrap();
        assert_eq!(printed, d.dsl, "{}", w.id);
        let parsed = cmd::parse_dsl(core.clone(), d.dsl.clone(), d.workout.ftp)
            .await
            .unwrap();
        let wire = serde_json::to_value(&parsed).expect("serialisable");
        assert_eq!(wire["outcome"], "parsed", "{}", w.id);
        let DslParse::Parsed { nodes } = parsed else {
            unreachable!("tagged parsed")
        };
        assert_eq!(
            cmd::print_dsl(core.clone(), nodes).await.unwrap(),
            d.dsl,
            "{}",
            w.id
        );
        let compiled = cmd::compile_workout(core.clone(), d.workout.clone())
            .await
            .unwrap();
        assert_eq!(compiled, d.compiled, "{}", w.id);
    }
    let bad = cmd::parse_dsl(core.clone(), "10m".into(), 250.0)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&bad).expect("serialisable")["outcome"],
        "invalid"
    );
}

/// #668, #681: the test hooks `sim_fault` and `sim_latency` exist only in the `e2e` build. The desktop installs the
/// invoke handler of `crank_desktop::bindings`, and that handler and the exported function
/// list come from one `collect_commands!`: so this exports the registry, built as CI's
/// nextest builds it (without `e2e`), and looks for the command, with `core_ready` as the
/// control that the export lists the commands at all.
#[test]
fn only_the_e2e_build_registers_sim_fault() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("bindings.ts");
    crank_desktop::bindings()
        .export(specta_typescript::Typescript::default(), &out)
        .expect("export");
    let ts = std::fs::read_to_string(&out).expect("exported");
    assert!(ts.contains(r#"__TAURI_INVOKE("core_ready")"#), "{ts}");
    for hook in ["sim_fault", "sim_latency"] {
        assert_eq!(
            ts.contains(&format!(r#"__TAURI_INVOKE("{hook}""#)),
            cfg!(feature = "e2e"),
            "{hook} is registered exactly in the e2e build"
        );
    }
}
