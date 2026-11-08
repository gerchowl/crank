//! The link contract (ADR-0002 §5.1.7 step 3) through the core: `links_open` / `links_ack`,
//! `links_intent`'s answer, `ride_status().load` and `device_links().phase`, on the
//! simulator's latency model under paused tokio time. Each test names the §5.1.4 or §5.1.8
//! bullet it covers.
use super::tests::{TICK, fake, latency, latent_core, open};
use super::*;
use crate::dto::{DeviceLinkView, LinkFrame, LinkPhase, LinkSample, LinkView, LoadProgress};
use crate::keeper::{T_COOL, channel::SPACING};
use crank_devices::sim::SimLatency;
use tokio::{sync::mpsc, time::sleep};

const S: fn(u64) -> Duration = Duration::from_secs;
const fn likely(ready: bool) -> LinkIntent {
    LinkIntent::Likely { ready }
}

/// A page on a link channel: it records every frame and acks it (or stalls).
struct Page {
    frames: mpsc::UnboundedReceiver<LinkFrame>,
    id: LinksId,
    seen: Vec<LinkFrame>,
}
async fn page(core: &Core, window: &str) -> Page {
    let (tx, frames) = mpsc::unbounded_channel();
    let id = core
        .links_open(Some(window.into()), move |f| {
            tx.send(f).map_err(|_| SinkClosed)
        })
        .await
        .unwrap();
    Page {
        frames,
        id,
        seen: Vec::new(),
    }
}
impl Page {
    /// The frames that arrived since the last call, acked when `ack`.
    async fn take(&mut self, core: &Core, window: &str, ack: bool) -> Vec<LinkFrame> {
        let got: Vec<_> = std::iter::from_fn(|| self.frames.try_recv().ok()).collect();
        if ack && let Some(last) = got.last() {
            core.links_ack(Some(window), self.id, last.seq)
                .await
                .unwrap();
        }
        self.seen.extend(got.iter().copied());
        got
    }
    /// Run for `d`, acking every frame as it comes (a page that keeps up).
    async fn follow(&mut self, core: &Core, window: &str, d: Duration) {
        let end = tokio::time::Instant::now() + d;
        while tokio::time::Instant::now() < end {
            sleep(SPACING / 4).await;
            self.take(core, window, true).await;
        }
    }
    /// Each distinct trainer phase, in the order the frames showed them.
    fn trainer_phases(&self) -> Vec<LinkPhase> {
        let mut phases: Vec<LinkPhase> = Vec::new();
        for f in &self.seen {
            if let Some(t) = f.links.trainer
                && phases.last() != Some(&t.phase)
            {
                phases.push(t.phase);
            }
        }
        phases
    }
}
fn phase(view: LinkView) -> [Option<LinkPhase>; 2] {
    [view.trainer, view.strap].map(|d| d.map(|d| d.phase))
}

/// §5.1.4 "Phases outside a ride", with §5.1.8 "Warm start under 1 s": the channel's first
/// frame is the state at the open; a `Likely` then shows the trainer through every phase of
/// its attempt to `warm`, and the warm load hands it to the ride (`riding`).
#[tokio::test(start_paused = true)]
async fn the_link_channel_shows_every_phase_from_cold_to_riding() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) = latent_core(dir.path(), latency(1, 1, 1), latency(1, 1, 0)).await;
    let mut page = page(&core, "main").await;
    sleep(TICK).await;
    let first = page.take(&core, "main", true).await;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].seq, 1);
    assert_eq!(
        phase(first[0].links),
        [Some(LinkPhase::Cold), Some(LinkPhase::Cold)]
    );
    let answered = core.links_intent(likely(false)).await.unwrap();
    assert_eq!(
        phase(answered.links),
        [Some(LinkPhase::Searching), Some(LinkPhase::Searching)],
        "links_intent answers the state it applied"
    );
    page.follow(&core, "main", S(3) + TICK).await;
    core.ride_load(None).await.unwrap();
    page.follow(&core, "main", TICK).await;
    assert_eq!(
        page.trainer_phases(),
        [
            LinkPhase::Cold,
            LinkPhase::Searching,
            LinkPhase::Connecting,
            LinkPhase::Handshake,
            LinkPhase::Warm,
            LinkPhase::Riding
        ]
    );
    // the state's numbers: rising, and skipping only what the channel coalesced
    let seqs: Vec<u32> = page.seen.iter().map(|f| f.seq).collect();
    assert!(seqs.windows(2).all(|w| w[1] > w[0]), "{seqs:?}");
    assert!(
        page.seen.iter().any(|f| f == &answered),
        "the answer's state is a frame's"
    );
}

/// ADR-0001 §4.3 through the core (ack window of 1, round 2 of #470): a page that stalls
/// after the first frame gets nothing more while the devices warm up and samples arrive; when
/// it acks it gets exactly one frame, the state now.
#[tokio::test(start_paused = true)]
async fn a_stalled_page_receives_only_the_latest_state_when_it_acks() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) = latent_core(dir.path(), latency(1, 1, 1), latency(1, 1, 0)).await;
    let mut page = page(&core, "main").await;
    sleep(TICK).await;
    assert_eq!(page.take(&core, "main", false).await.len(), 1);
    core.links_intent(likely(false)).await.unwrap();
    sleep(S(10)).await;
    assert!(page.take(&core, "main", false).await.is_empty(), "stalled");
    core.links_ack(Some("main"), page.id, 1).await.unwrap();
    sleep(TICK).await;
    let got = page.take(&core, "main", false).await;
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].seq > 1, "a newer state: {got:?}");
    // the state when it was sent: both warm, with samples (that it is the latest is the
    // channel's own test; the samples move on at 4 Hz meanwhile)
    assert_eq!(
        phase(got[0].links),
        [Some(LinkPhase::Warm), Some(LinkPhase::Warm)]
    );
    assert!(
        got[0].links.strap.is_some_and(|s| s.sample.is_some()),
        "{got:?}"
    );
}

/// §5.1.3 "the latest sample per device, for the chips": a warm strap's frames carry its
/// heart rate; once it is let go its sample is gone with the phase. §5.1.8 "Idle cool-down
/// disconnects" as the channel shows it: warm at `T_cool` − 1 s, `cold` at `T_cool`.
#[tokio::test(start_paused = true)]
async fn warm_frames_carry_the_latest_sample_and_cold_ones_none() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    let mut page = page(&core, "main").await;
    core.links_intent(likely(false)).await.unwrap();
    core.links_intent(LinkIntent::Left).await.unwrap();
    page.follow(&core, "main", S(5)).await;
    let last = page.seen.last().copied().unwrap();
    assert!(
        matches!(
            last.links.strap,
            Some(DeviceLinkView {
                phase: LinkPhase::Warm,
                sample: Some(LinkSample::HeartRate { bpm: Some(_) })
            })
        ),
        "{last:?}"
    );
    assert!(
        last.links
            .trainer
            .is_some_and(|t| t.phase == LinkPhase::Warm
                && t.sample
                    .is_none_or(|s| matches!(s, LinkSample::Trainer { .. }))),
        "{last:?}"
    );
    page.follow(&core, "main", T_COOL.saturating_sub(S(6)))
        .await;
    let warm = page.seen.last().copied().unwrap();
    assert_eq!(phase(warm.links), [Some(LinkPhase::Warm); 2]);
    page.follow(&core, "main", S(2)).await;
    let cold = page.seen.last().copied().unwrap();
    assert_eq!(
        cold.links,
        LinkView {
            trainer: Some(DeviceLinkView {
                phase: LinkPhase::Cold,
                sample: None
            }),
            strap: Some(DeviceLinkView {
                phase: LinkPhase::Cold,
                sample: None
            }),
        }
    );
}

/// §5.1.4 "Load phases in `ride_status`": a cold load reports each device's phase while it
/// runs, `riding` once handed over, and `load` is `null` before and after.
#[tokio::test(start_paused = true)]
async fn ride_status_reports_the_phases_of_a_load_in_flight() {
    use LinkPhase::{Connecting, Handshake, Searching, Warm};
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) = latent_core(dir.path(), latency(1, 1, 1), latency(1, 0, 0)).await;
    assert_eq!(core.ride_status().await.unwrap().load, None);
    let load = {
        let core = core.clone();
        tokio::spawn(async move { core.ride_load(None).await })
    };
    let mut seen = Vec::new();
    for _ in 0..16 {
        sleep(TICK).await;
        if let Some(progress) = core.ride_status().await.unwrap().load
            && seen.last() != Some(&progress)
        {
            seen.push(progress);
        }
    }
    load.await.unwrap().unwrap();
    assert_eq!(core.ride_status().await.unwrap().load, None);
    let at = |trainer, strap| LoadProgress {
        trainer,
        strap: Some(strap),
    };
    assert_eq!(
        seen,
        [
            at(Searching, Searching),
            at(Connecting, Warm),
            at(Handshake, Warm),
        ],
        "a load answers once handed over, before a status can read `riding`"
    );
}

/// §5.1.4: a load with no strap paired reports `strap: null`.
#[tokio::test(start_paused = true)]
async fn a_load_with_no_strap_paired_reports_none_for_it() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) = latent_core(dir.path(), latency(1, 1, 1), SimLatency::default()).await;
    core.device_forget(DeviceRole::HeartRate).await.unwrap();
    let load = {
        let core = core.clone();
        tokio::spawn(async move { core.ride_load(None).await })
    };
    sleep(TICK).await;
    assert_eq!(
        core.ride_status().await.unwrap().load,
        Some(LoadProgress {
            trainer: LinkPhase::Searching,
            strap: None
        })
    );
    assert!(!load.await.unwrap().unwrap().hr_linked);
}

/// §5.1.4: `device_links` reports the keeper's phase when no ride holds the link, and
/// `riding` beside the ride's live link while one does.
#[tokio::test(start_paused = true)]
async fn device_links_reports_the_keepers_phase() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    let phases = |links: Vec<DeviceLinkInfo>| links.into_iter().map(|l| (l.phase, l.link));
    assert!(
        phases(core.device_links().await.unwrap()).eq([(Some(LinkPhase::Cold), None); 2]),
        "a paired device is `cold` before any intent"
    );
    core.links_intent(likely(false)).await.unwrap();
    sleep(TICK).await;
    assert!(phases(core.device_links().await.unwrap()).eq([(Some(LinkPhase::Warm), None); 2]));
    core.ride_load(None).await.unwrap();
    let live: Vec<_> = phases(core.device_links().await.unwrap()).collect();
    assert!(
        live.iter()
            .all(|(phase, link)| *phase == Some(LinkPhase::Riding) && link.is_some()),
        "{live:?}"
    );
    core.ride_end().await.unwrap();
    core.device_forget(DeviceRole::HeartRate).await.unwrap();
    let after: Vec<_> = phases(core.device_links().await.unwrap()).collect();
    assert_eq!(after, [(Some(LinkPhase::Cold), None), (None, None)]);
}

/// §5.1.3 "What `links_intent` answers" at the contract: during a started ride "Release
/// trainer" is `Conflict` and changes nothing; a loaded ride's devices answer `riding`.
#[tokio::test(start_paused = true)]
async fn links_intent_answers_riding_and_refuses_release_trainer_in_a_started_ride() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    core.links_intent(likely(false)).await.unwrap();
    sleep(TICK).await;
    core.ride_load(None).await.unwrap();
    let riding = core.links_intent(likely(true)).await.unwrap();
    assert_eq!(phase(riding.links), [Some(LinkPhase::Riding); 2]);
    core.ride_start().await.unwrap();
    let refused = core
        .links_intent(LinkIntent::Release {
            reason: ReleaseReason::RiderRelease,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict);
    assert_eq!(
        phase(core.links_intent(likely(false)).await.unwrap().links),
        [Some(LinkPhase::Riding); 2]
    );
}

/// ADR-0001 §4.3's window cleanup for the link channel: another window's ack is `NotFound`, a
/// reload's new channel replaces the old one, and `close_window` drops it.
#[tokio::test(start_paused = true)]
async fn link_channels_belong_to_their_window() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    let first = page(&core, "main").await;
    let other = core
        .links_ack(Some("other"), first.id, 1)
        .await
        .unwrap_err();
    assert_eq!(other.code, ErrorCode::NotFound);
    let mut reloaded = page(&core, "main").await;
    let stale = core.links_ack(Some("main"), first.id, 1).await.unwrap_err();
    assert_eq!(stale.code, ErrorCode::NotFound, "the reload dropped it");
    sleep(TICK).await;
    assert_eq!(reloaded.take(&core, "main", true).await.len(), 1);
    core.close_window("main".into()).await;
    let closed = core
        .links_ack(Some("main"), reloaded.id, 1)
        .await
        .unwrap_err();
    assert_eq!(closed.code, ErrorCode::NotFound);
    core.links_intent(likely(false)).await.unwrap();
    sleep(TICK).await;
    assert!(
        reloaded.take(&core, "main", false).await.is_empty(),
        "no frame after the close"
    );
}

/// §5.1.4 "Which channel reports link state during a ride", per device: a strap that does not
/// advertise at the load stays out of the ride (`hr_linked: false`) and the keeper's frame
/// goes on reporting it (`not_found`) beside the ride's trainer (`riding`); at the ride's end
/// only the device the ride held goes `cold`.
#[tokio::test(start_paused = true)]
async fn each_device_has_one_source_an_unlinked_strap_stays_the_keepers() {
    let dir = tempfile::tempdir().unwrap();
    let absent = SimLatency {
        not_advertising: true,
        ..latency(1, 0, 0)
    };
    let (core, _radio) = latent_core(dir.path(), SimLatency::default(), absent).await;
    let mut page = page(&core, "main").await;
    assert!(!core.ride_load(None).await.unwrap().hr_linked);
    page.follow(&core, "main", TICK).await;
    let during = page.seen.last().copied().unwrap();
    assert_eq!(
        phase(during.links),
        [Some(LinkPhase::Riding), Some(LinkPhase::NotFound)]
    );
    core.ride_start().await.unwrap();
    core.ride_end().await.unwrap();
    page.follow(&core, "main", TICK).await;
    let after = page.seen.last().copied().unwrap();
    assert_eq!(
        phase(after.links),
        [Some(LinkPhase::Cold), Some(LinkPhase::NotFound)]
    );
}

/// §5.1.1 "A ride's end leaves the phase `cold`" as a stalled page sees it: frames held
/// across a load and the ride's end arrive, after the ack, as one frame of `cold` devices
/// with no sample: the keeper's state now, with its number.
#[tokio::test(start_paused = true)]
async fn a_page_stalled_across_a_load_and_its_end_gets_cold_after_its_ack() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    core.links_intent(likely(false)).await.unwrap();
    sleep(TICK).await;
    let mut page = page(&core, "main").await;
    sleep(TICK).await;
    let first = page.take(&core, "main", false).await;
    assert_eq!(phase(first[0].links), [Some(LinkPhase::Warm); 2]);
    core.ride_load(None).await.unwrap();
    core.ride_start().await.unwrap();
    core.ride_end().await.unwrap();
    sleep(S(1)).await;
    assert!(page.take(&core, "main", false).await.is_empty(), "stalled");
    core.links_ack(Some("main"), page.id, first[0].seq)
        .await
        .unwrap();
    sleep(TICK).await;
    let got = page.take(&core, "main", false).await;
    let cold = Some(DeviceLinkView {
        phase: LinkPhase::Cold,
        sample: None,
    });
    let now = *core.0.keeper.view().borrow();
    assert!(now.seq > first[0].seq);
    assert_eq!(
        got,
        [LinkFrame {
            seq: now.seq,
            links: LinkView {
                trainer: cold,
                strap: cold
            }
        }]
    );
}

/// ADR-0002 §5.1.3 "What `links_intent` answers": an intent that discards the loaded ride is
/// answered with the state after the ride's links were released (`cold`), and an applied
/// intent with the state its own step published (pins "publish, then answer").
#[tokio::test(start_paused = true)]
async fn links_intent_answers_the_state_its_resolving_step_published() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    let started = core.links_intent(likely(false)).await.unwrap();
    assert_eq!(phase(started.links), [Some(LinkPhase::Searching); 2]);
    sleep(TICK).await;
    core.ride_load(None).await.unwrap();
    let discarded = core
        .links_intent(LinkIntent::Release {
            reason: ReleaseReason::Sleep,
        })
        .await
        .unwrap();
    assert_eq!(phase(discarded.links), [Some(LinkPhase::Cold); 2]);
    assert!(!core.ride_status().await.unwrap().status.loaded);
}

/// The reload race (#561 review): the page-load event issues the close of the old page's
/// channel, and the new page may open its own before that close runs. A close drops only
/// what was open when it was issued: the new page's channel and its acks are untouched.
#[tokio::test(start_paused = true)]
async fn a_close_issued_before_a_reopen_never_drops_the_new_channel() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    let _old = page(&core, "main").await;
    let close = core.close_window("main".into());
    let mut new = page(&core, "main").await;
    close.await;
    sleep(TICK).await;
    let first = new.take(&core, "main", true).await;
    assert_eq!(
        first.len(),
        1,
        "the new channel is open and its ack is taken"
    );
    core.links_intent(likely(false)).await.unwrap();
    sleep(TICK).await;
    assert!(
        !new.take(&core, "main", true).await.is_empty(),
        "and it keeps sending"
    );
}

/// §5.1.3 "Release trainer" after the ride's end (#561 review): `ride_end` waits for the reap,
/// so the ended ride holds nothing and the release is accepted, both devices `cold`.
#[tokio::test(start_paused = true)]
async fn release_trainer_after_the_end_of_a_started_ride_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    core.ride_load(None).await.unwrap();
    core.ride_start().await.unwrap();
    core.ride_end().await.unwrap();
    let released = core
        .links_intent(LinkIntent::Release {
            reason: ReleaseReason::RiderRelease,
        })
        .await
        .unwrap();
    assert_eq!(phase(released.links), [Some(LinkPhase::Cold); 2]);
}

/// §5.1.3 with #269 (#561 review): a ride that finished on its own (its last block skipped)
/// keeps its session until the reaper takes it, once the engine drops the ride-active gate
/// after the end mode (delivered here 1 s late). Until then the ride holds the trainer:
/// "Release trainer" is `Conflict` and the trainer is `riding`; after the reap it is `cold`
/// and the release is accepted.
#[tokio::test(start_paused = true)]
async fn a_ride_that_finished_on_its_own_holds_its_devices_until_the_reap() {
    let dir = tempfile::tempdir().unwrap();
    let mut radio = fake(vec![], false);
    radio.mode_delay = S(1);
    let core = open(dir.path(), &radio);
    core.device_pair("neo".into(), DeviceRole::Trainer)
        .await
        .unwrap();
    core.ride_load(Some("builtin-sensor-check".into()))
        .await
        .unwrap();
    core.ride_start().await.unwrap();
    for _ in 0..3 {
        sleep(S(2)).await;
        core.ride_skip().await.unwrap();
    }
    assert_eq!(
        core.ride_status().await.unwrap().status.state,
        crank_engine::StreamRideState::Finished
    );
    let release = || {
        core.links_intent(LinkIntent::Release {
            reason: ReleaseReason::RiderRelease,
        })
    };
    let held = release().await.unwrap_err();
    assert_eq!(
        (held.code, held.message.as_str()),
        (
            ErrorCode::Conflict,
            "the ride holds the trainer: end the ride first"
        )
    );
    assert_eq!(phase(core.0.keeper.links())[0], Some(LinkPhase::Riding));
    sleep(S(3)).await;
    assert_eq!(
        phase(core.0.keeper.links())[0],
        Some(LinkPhase::Cold),
        "reaped: the hold is gone with the session"
    );
    let released = release().await.unwrap();
    assert_eq!(phase(released.links), [Some(LinkPhase::Cold), None]);
}

/// §5.1.4, one source per device, as a stalled page sees it (#561 review): with a strap that
/// missed the load, a page stalled across the ride gets one frame after its ack, the trainer
/// `cold` and the strap still `not_found`.
#[tokio::test(start_paused = true)]
async fn a_page_stalled_across_a_ride_with_an_unlinked_strap_gets_one_frame() {
    let dir = tempfile::tempdir().unwrap();
    let absent = SimLatency {
        not_advertising: true,
        ..latency(1, 0, 0)
    };
    let (core, _radio) = latent_core(dir.path(), SimLatency::default(), absent).await;
    let mut page = page(&core, "main").await;
    sleep(TICK).await;
    let first = page.take(&core, "main", false).await;
    assert!(!core.ride_load(None).await.unwrap().hr_linked);
    core.ride_start().await.unwrap();
    core.ride_end().await.unwrap();
    sleep(S(1)).await;
    assert!(page.take(&core, "main", false).await.is_empty(), "stalled");
    core.links_ack(Some("main"), page.id, first[0].seq)
        .await
        .unwrap();
    sleep(TICK).await;
    let got = page.take(&core, "main", false).await;
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(
        phase(got[0].links),
        [Some(LinkPhase::Cold), Some(LinkPhase::NotFound)]
    );
}

/// §5.1.4 "answers are ordered by `seq`" (#561 review): an answer that waits for its releases
/// can arrive after the frame of a newer state. Every answer carries its state's `seq`, the
/// number a frame of that state carries, so the rule "apply an answer only if its `seq` is at
/// least the last frame's" keeps the newest state whatever the arrival order.
#[tokio::test(start_paused = true)]
async fn an_answer_and_a_frame_of_the_same_state_share_its_seq_and_a_newer_frame_outranks_it() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    let mut page = page(&core, "main").await;
    core.links_intent(likely(false)).await.unwrap();
    page.follow(&core, "main", S(1)).await;
    assert_eq!(
        phase(page.seen.last().unwrap().links),
        [Some(LinkPhase::Warm); 2]
    );
    // the release is applied first and answers once the links are let go; the `Likely` after
    // it is applied next and answers at once
    let release = tokio::spawn({
        let core = core.clone();
        async move {
            core.links_intent(LinkIntent::Release {
                reason: ReleaseReason::Devices,
            })
            .await
        }
    });
    tokio::task::yield_now().await;
    let newer = core.links_intent(likely(false)).await.unwrap();
    let older = release.await.unwrap().unwrap();
    assert!(older.seq < newer.seq, "{older:?} {newer:?}");
    assert_eq!(phase(older.links), [Some(LinkPhase::Cold); 2]);
    assert_eq!(phase(newer.links), [Some(LinkPhase::Searching); 2]);
    page.follow(&core, "main", TICK).await;
    for answer in [older, newer] {
        assert!(
            page.seen
                .iter()
                .filter(|f| f.seq == answer.seq)
                .all(|f| *f == answer),
            "one seq, one state"
        );
    }
    let last = *page.seen.last().unwrap();
    assert!(last.seq >= newer.seq);
    let applied = [last, older, newer]
        .into_iter()
        .reduce(|kept, next| if next.seq >= kept.seq { next } else { kept })
        .unwrap();
    assert_eq!(applied, last, "the frame is authoritative");
}
