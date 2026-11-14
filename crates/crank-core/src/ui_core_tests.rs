//! The snapshot channel through the facade (ADR-0001 §4.3.5, §4.3.9), on paused tokio time.
use super::tests::{fake, open, riding, stall_engine};
use super::*;
use crate::dto::{RideEnding, UiFrame};
use tokio::{sync::mpsc, time::sleep};

fn sink() -> (impl UiSink, mpsc::UnboundedReceiver<UiFrame>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (move |f| tx.send(f).map_err(|_| SinkClosed), rx)
}

/// §4.3.9 "lifecycle by id", the #564 reproduction: the page-load event issues the close, the
/// new page opens its channel, then the close's future runs. The new channel is alive and acks
/// (on `dev` the new page's stream answers `NotFound`).
#[tokio::test(start_paused = true)]
async fn a_close_issued_before_the_new_page_opens_never_closes_its_channel() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path(), &fake(vec![], false));
    let (old_sink, _old) = sink();
    let old = core.ui_open(Some("main".into()), old_sink).await.unwrap();
    let close = core.close_window("main".into());
    let (new_sink, mut frames) = sink();
    let new = core.ui_open(Some("main".into()), new_sink).await.unwrap();
    close.await;
    let gone = core.ui_ack(Some("main"), old, 0).await.unwrap_err();
    assert_eq!(gone.code, ErrorCode::NotFound);
    sleep(Duration::from_millis(1)).await;
    let first = frames.try_recv().unwrap();
    assert_eq!(first.channel, new);
    core.ui_ack(Some("main"), new, first.seq).await.unwrap();
    sleep(Duration::from_millis(1_100)).await;
    assert_eq!(
        frames.try_recv().unwrap().seq,
        first.seq + 1,
        "alive after the close ran"
    );
}

/// §4.3.9 "a forced end survives a reload": a channel opened after the core ended a stuck ride
/// by force reads `ride.ended = forced` (on `dev`, `forcedEnds.ts` loses it on a reload).
#[tokio::test(start_paused = true)]
async fn a_forced_end_is_visible_to_a_new_channel() {
    let dir = tempfile::tempdir().unwrap();
    let radio = fake(vec![], false);
    let core = riding(&radio, dir.path()).await;
    stall_engine(&core);
    let end = core.ride_end_outcome().await.unwrap();
    assert!(matches!(end, RideEnd::ForcedEnd(_)), "{end:?}");
    let (s, mut frames) = sink();
    core.ui_open(Some("main".into()), s).await.unwrap();
    sleep(Duration::from_millis(1)).await;
    let ride = frames.try_recv().unwrap().ride.unwrap();
    assert_eq!(ride.ended, Some(RideEnding::Forced));
    assert_eq!(Some(ride.id.as_str()), end.ride_id());
    assert!(
        !ride.engine_unresponsive,
        "an ended ride is not shown as unresponsive"
    );
    assert_eq!(
        core.snapshot().ride.unwrap().ended,
        Some(RideEnding::Forced)
    );
    core.0.stall.release();
}

/// §4.3.5 "engine unresponsive", the request path (#351): a bounded request that times out
/// marks the loaded ride `engine_unresponsive`. The ride is loaded and not started, so it does
/// not tick and the silent-status path cannot be what sets the flag; it stays until the ride
/// ends.
#[tokio::test(start_paused = true)]
async fn a_request_that_times_out_marks_the_ride_unresponsive() {
    let dir = tempfile::tempdir().unwrap();
    let radio = fake(vec![], false);
    let core = open(dir.path(), &radio);
    core.device_pair("neo".into(), DeviceRole::Trainer)
        .await
        .unwrap();
    core.ride_load(None).await.unwrap();
    let flag = |core: &Core| core.snapshot().ride.unwrap().engine_unresponsive;
    sleep(Duration::from_secs(5)).await;
    assert!(!flag(&core), "an unstarted ride is not timed by its status");
    stall_engine(&core);
    let refused = core.ride_start().await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::EngineUnresponsive);
    assert!(flag(&core), "the timed-out request marks the ride");
    core.0.stall.release();
    sleep(Duration::from_secs(1)).await;
    assert!(flag(&core), "kept until the ride ends");
}
