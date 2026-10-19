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
    /// The last block completes by itself; the handle is dropped afterwards.
    Natural,
    /// A §7.4 release lands on the last tick of the last block.
    ReleaseOnLastTick,
}
const PAGES: [u8; 4] = [48, 49, 50, 51];
fn last_control_page(fake: &Fake) -> Option<u8> {
    fake.writes
        .lock()
        .unwrap()
        .iter()
        .map(|(_, p)| p[4])
        .rfind(|page| PAGES.contains(page))
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
    let release_at = matches!(end, End::ReleaseOnLastTick).then_some(6);
    let (engine, handle) = Engine::new(
        clock,
        Some(link),
        None,
        Arc::new(move |active: bool| g.set_active(active)),
    );
    let engine = engine.with_policy_factory(move || Box::new(Scripted { release_at }));
    let task = tokio::spawn(engine.run());
    let blocks = match end {
        End::Natural | End::ReleaseOnLastTick => vec![block(2.0)],
        _ => vec![block(600.0)],
    };
    handle
        .send(EngineCmd::Load(RideSetup {
            blocks,
            recorder: Box::new(NullRecord),
            cap_w: None,
        }))
        .await
        .unwrap();
    handle.send(EngineCmd::Start).await.unwrap();
    match end {
        End::Natural | End::ReleaseOnLastTick => {
            for _ in 0..12 {
                super::tick().await;
            }
        }
        _ => {
            for _ in 0..6 {
                super::tick().await;
            }
            // the ride is in ERG on the trainer before the end begins
            assert_eq!(last_control_page(&fake), Some(49), "ERG is applied");
            match end {
                End::PauseThenDrop => {
                    handle.send(EngineCmd::Pause).await.unwrap();
                    // the engine queues 0 % at its next tick; the writer has not run yet
                    super::tick().await;
                }
                End::PauseStopDrop => {
                    handle.send(EngineCmd::Pause).await.unwrap();
                    super::tick().await;
                    handle.send(EngineCmd::Stop).await.unwrap();
                }
                End::StopThenDrop => handle.send(EngineCmd::Stop).await.unwrap(),
                _ => {}
            }
        }
    }
    drop(handle);
    // the engine shuts down only when the end mode is delivered, lost or timed out
    for _ in 0..16 {
        super::tick().await;
    }
    let finished = task.is_finished();
    assert!(!gate.is_active(), "the ride-active signal is cleared");
    (fake, finished)
}
async fn assert_ends_on_page_48(end: End, supervised: bool) {
    let (fake, finished) = run(end, supervised).await;
    assert!(finished, "{end:?}: the engine task ended");
    assert_eq!(
        last_control_page(&fake),
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
