//! #266 end to end: the real `Engine` over the real `FecTrainer` and the `Fake` transport.
//! What matters is the last control page that *reached the transport*, not what the engine
//! queued: ADR-0002 §7.9, "silence after ERG could leave the ERG target active".
use super::*;
use crank_domain::workout::{FlatBlock, Phase};
use crank_engine::{
    Engine, EngineCmd, PolicyContext, RecordError, RideRecorder, RideSetup, SecondRow, TargetPolicy,
};

/// ERG 200 W, 0 % resistance while the rider's pause is in force or from tick `release_at`
/// (a §7.4 release): the commands a ride's end finds on the trainer.
struct Scripted {
    release_at: Option<u64>,
}
impl TargetPolicy for Scripted {
    fn desired(&mut self, ctx: &PolicyContext<'_>) -> Option<ControlMode> {
        if ctx.user_paused || self.release_at.is_some_and(|t| ctx.tick >= t) {
            Some(ControlMode::Resistance { pct: 0.0 })
        } else {
            Some(ControlMode::Erg { target_w: 200 })
        }
    }
}
struct NullRecord;
impl RideRecorder for NullRecord {
    fn start(&mut self, _: &[&str]) -> Result<(), RecordError> {
        Ok(())
    }
    fn append_second(&mut self, _: u32, _: &SecondRow) -> Result<(), RecordError> {
        Ok(())
    }
    fn finish(&mut self) -> Result<(), RecordError> {
        Ok(())
    }
}
fn block(dur_s: f64) -> FlatBlock {
    FlatBlock {
        path: vec![],
        iteration: vec![],
        node_id: String::new(),
        start_s: 0.0,
        dur_s,
        power_from: 200.0,
        power_to: 200.0,
        phase: Phase::Work,
        dur_range_s: None,
    }
}
#[derive(Clone, Copy, Debug)]
enum End {
    /// Every handle dropped mid-ERG.
    Drop,
    /// `Stop`, then every handle dropped.
    StopThenDrop,
    /// `Pause`, then every handle dropped before the writer tick that would write 0 %.
    PauseThenDrop,
    /// `Pause`, `Stop`, every handle dropped.
    PauseStopDrop,
    /// The last block completes by itself on a busy radio; the handle is dropped at once.
    /// Kills "the end sends nothing" and "shutdown does not await the end mode". It cannot
    /// kill the `last_sent == END_MODE` shortcut: the last command is sent a whole engine
    /// tick before the finishing tick, and the end mode is `Erg` (not `Resistance`) there.
    Natural,
    /// A §7.4 release lands on the last tick of the last block, on a busy radio. Same mutants
    /// as `Natural`; the finishing tick sends nothing, so the release's own page is always
    /// one engine tick older than the end.
    ReleaseOnLastTick,
    /// `Stop`, `Load` of the next ride within the same writer tick, every handle dropped (#279).
    StopLoadDrop,
    /// `Stop`, `Load`, `Stop` of the unstarted ride, every handle dropped (#279).
    StopLoadStopDrop,
    /// `Stop`, `Load`, `Start` of the next ride, which runs in ERG (#279).
    StopLoadStart,
}
const PAGES: [u8; 4] = [48, 49, 50, 51];
fn control_pages(fake: &Fake) -> Vec<u8> {
    fake.writes
        .lock()
        .unwrap()
        .iter()
        .map(|(_, p)| p[4])
        .filter(|page| PAGES.contains(page))
        .collect()
}
fn last_control_page(fake: &Fake) -> Option<u8> {
    control_pages(fake).last().copied()
}
/// One writer tick; the ride-active signal may clear only once page 48 is on the trainer.
async fn step(gate: &RideGate, fake: &Fake) {
    super::tick().await;
    assert!(
        gate.is_active() || last_control_page(fake) == Some(48),
        "the gate cleared with {:?} the last control page written",
        last_control_page(fake)
    );
}
async fn load(handle: &crank_engine::EngineHandle, dur_s: f64) {
    handle
        .send(EngineCmd::Load(RideSetup {
            blocks: vec![block(dur_s)],
            recorder: Box::new(NullRecord),
            cap_w: None,
        }))
        .await
        .unwrap();
}
async fn run(end: End, supervised: bool) -> (Arc<Fake>, bool) {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let fake = Fake::new(clock.clone());
    let inner = FecTrainer::new(Arc::new(fake.clone()), clock.clone());
    let gate = RideGate::new(false);
    let link = if supervised {
        Reconnecting::new(inner, clock.clone(), gate.clone())
            .connect(&DeviceRef("fake".into()), rider())
            .await
            .unwrap()
    } else {
        inner
            .connect(&DeviceRef("fake".into()), rider())
            .await
            .unwrap()
    };
    settle().await;
    let g = gate.clone();
    // the last control page on the transport at each `set_active(false)`, read inside the
    // callback: `step` only looks once per writer tick, which cannot see an unload that
    // clears the gate while the end mode is still owed
    let at_clear: Arc<Mutex<Vec<Option<u8>>>> = Arc::default();
    let (cb_fake, cb_log) = (fake.clone(), at_clear.clone());
    let release_at = matches!(end, End::ReleaseOnLastTick).then_some(6);
    let (engine, handle) = Engine::new(
        clock,
        Some(link),
        None,
        Arc::new(move |active: bool| {
            if !active {
                cb_log.lock().unwrap().push(last_control_page(&cb_fake));
            }
            g.set_active(active);
        }),
    );
    let engine = engine.with_policy_factory(move || Box::new(Scripted { release_at }));
    let task = tokio::spawn(engine.run());
    let short = matches!(end, End::Natural | End::ReleaseOnLastTick);
    // a busy radio: the next ride's first command is queued before page 48 is written, and
    // the writer would coalesce it away
    if short || matches!(end, End::StopLoadStart) {
        // a busy radio: a page queued on the last tick is still unwritten when the ride ends
        fake.write_delay_ms.store(400, Ordering::SeqCst);
    }
    load(&handle, if short { 2.0 } else { 600.0 }).await;
    handle.send(EngineCmd::Start).await.unwrap();
    if short {
        // the end is the moment the ride finishes by itself: the handle goes at once
        while handle.status().state != crank_engine::RideState::Finished {
            step(&gate, &fake).await;
        }
    } else {
        for _ in 0..6 {
            step(&gate, &fake).await;
        }
        // the ride is in ERG on the trainer before the end begins
        assert_eq!(last_control_page(&fake), Some(49), "ERG is applied");
        match end {
            End::PauseThenDrop => {
                handle.send(EngineCmd::Pause).await.unwrap();
                // the engine queues 0 % at its next tick; the writer has not run yet
                step(&gate, &fake).await;
            }
            End::PauseStopDrop => {
                handle.send(EngineCmd::Pause).await.unwrap();
                step(&gate, &fake).await;
                handle.send(EngineCmd::Stop).await.unwrap();
            }
            End::StopThenDrop => handle.send(EngineCmd::Stop).await.unwrap(),
            End::StopLoadDrop | End::StopLoadStopDrop | End::StopLoadStart => {
                handle.send(EngineCmd::Stop).await.unwrap();
                load(&handle, 600.0).await;
                match end {
                    End::StopLoadStopDrop => handle.send(EngineCmd::Stop).await.unwrap(),
                    End::StopLoadStart => {
                        let before = control_pages(&fake).len();
                        handle.send(EngineCmd::Start).await.unwrap();
                        for _ in 0..8 {
                            step(&gate, &fake).await;
                        }
                        // page 48 of the first ride's end is on the trainer before any ERG
                        // page of the second (the engine holds it back until delivery)
                        let pages = control_pages(&fake)[before..].to_vec();
                        let first_48 = pages.iter().position(|p| *p == 48);
                        let first_49 = pages.iter().position(|p| *p == 49);
                        assert!(first_48.is_some(), "{pages:?}");
                        assert!(first_49 > first_48, "{pages:?}");
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    drop(handle);
    // the engine shuts down only when the end mode is delivered, lost or timed out
    for _ in 0..16 {
        step(&gate, &fake).await;
    }
    let finished = task.is_finished();
    assert!(!gate.is_active(), "the ride-active signal is cleared");
    let at_clear = at_clear.lock().unwrap().clone();
    assert!(!at_clear.is_empty(), "{end:?}: the signal was cleared");
    assert!(
        at_clear.iter().all(|p| *p == Some(48)),
        "{end:?}: the last control page at each set_active(false): {at_clear:?}"
    );
    (fake, finished)
}
async fn assert_ends_on_page_48(end: End, supervised: bool) {
    let (fake, finished) = run(end, supervised).await;
    assert!(finished, "{end:?}: the engine task ended");
    let last = last_control_page(&fake);
    // the second ride of `StopLoadStart` is still riding in ERG when its handle drops: its
    // own end follows page 48 as well
    assert_eq!(
        last,
        Some(48),
        "{end:?} supervised={supervised}: the last control page written is 0 % resistance"
    );
}
/// With and without the `Reconnecting` supervisor.
async fn both(end: End) {
    assert_ends_on_page_48(end, false).await;
    assert_ends_on_page_48(end, true).await;
}
#[tokio::test(start_paused = true)]
async fn dropping_every_handle_mid_erg_leaves_page_48_last() {
    both(End::Drop).await;
}
#[tokio::test(start_paused = true)]
async fn stop_then_drop_leaves_page_48_last() {
    both(End::StopThenDrop).await;
}
#[tokio::test(start_paused = true)]
async fn pause_then_drop_within_a_writer_tick_leaves_page_48_last() {
    both(End::PauseThenDrop).await;
}
#[tokio::test(start_paused = true)]
async fn pause_stop_drop_leaves_page_48_last() {
    both(End::PauseStopDrop).await;
}
#[tokio::test(start_paused = true)]
async fn a_natural_finish_leaves_page_48_last() {
    both(End::Natural).await;
}
#[tokio::test(start_paused = true)]
async fn a_release_on_the_last_tick_leaves_page_48_last() {
    both(End::ReleaseOnLastTick).await;
}
#[tokio::test(start_paused = true)]
async fn stop_load_drop_writes_page_48_before_the_gate_clears() {
    both(End::StopLoadDrop).await;
}
#[tokio::test(start_paused = true)]
async fn stop_load_stop_drop_writes_page_48_before_the_gate_clears() {
    both(End::StopLoadStopDrop).await;
}
#[tokio::test(start_paused = true)]
async fn stop_load_start_writes_page_48_before_any_erg_page_of_the_new_ride() {
    both(End::StopLoadStart).await;
}
