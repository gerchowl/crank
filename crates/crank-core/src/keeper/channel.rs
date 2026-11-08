//! The window's link channel (`links_open`, ADR-0002 §5.1.4 under ADR-0001 §4.3).
//!
//! Each window has at most one channel; opening another from the same window drops the old
//! one (a reload), and so do the window's destruction ([`Channels::close`]) and a failed send.
//! A frame is the keeper's complete state with its number ([`LinkFrame`]), sent when it
//! changes:
//!
//! - **Ack window of 1.** After a frame the channel sends nothing until the window
//!   acknowledges it (`links_ack`), so a stalled page queues at most one frame (ADR-0001
//!   §4.3: Tauri's channel has no backpressure of its own).
//! - **Latest wins.** Changes meanwhile are coalesced: the next frame is the state at the
//!   time it is sent. Every frame is complete, so a coalesced state loses nothing and there is
//!   no resync.
//! - **At most 10 Hz** (ADR-0001 §4.3's frame emission), paced on the clock's unscaled axis.
//! - **Bounded size.** A frame has no strings or lists, so its size has a fixed maximum, far
//!   under the 8 000-byte budget (measured by a test).
use crate::core::SinkClosed;
use crate::dto::{LinkFrame, LinksId};
use crate::error::{ApiError, ErrorCode};
use crank_devices::{Clock, MonoTime};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle};

/// Least time between two frames of a channel: at most 10 frames per second.
pub(crate) const SPACING: Duration = Duration::from_millis(100);

/// Where a window's link frames go.
pub trait LinkSink: Send + 'static {
    /// Deliver one frame; an error drops the channel.
    ///
    /// # Errors
    /// [`SinkClosed`] when the window can no longer receive.
    fn send(&self, frame: LinkFrame) -> Result<(), SinkClosed>;
}
impl<F: Fn(LinkFrame) -> Result<(), SinkClosed> + Send + 'static> LinkSink for F {
    fn send(&self, frame: LinkFrame) -> Result<(), SinkClosed> {
        self(frame)
    }
}

/// The last frame sent, and whether the window acknowledged it.
type Acks = watch::Sender<(u32, bool)>;
struct Open {
    id: LinksId,
    acks: Arc<Acks>,
    task: JoinHandle<()>,
}
impl Drop for Open {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The link channels of one core, by window.
pub(crate) struct Channels {
    next: AtomicU32,
    open: Mutex<HashMap<Option<String>, Open>>,
}
impl Channels {
    pub(crate) fn new() -> Self {
        Self {
            next: AtomicU32::new(1),
            open: Mutex::new(HashMap::new()),
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Option<String>, Open>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// Open `window`'s channel over `view`, dropping its previous one; the first frame, the
    /// state now, goes out at once.
    pub(crate) fn open(
        &self,
        window: Option<String>,
        view: watch::Receiver<LinkFrame>,
        clock: Arc<dyn Clock>,
        sink: impl LinkSink,
    ) -> LinksId {
        let id = loop {
            let n = self.next.fetch_add(1, Ordering::Relaxed);
            if n != 0 {
                break LinksId(n);
            }
        };
        let acks = Arc::new(watch::Sender::new((0, true)));
        let task = tokio::spawn(run(view, acks.subscribe(), acks.clone(), clock, sink));
        self.lock().insert(window, Open { id, acks, task });
        id
    }
    /// `window` has applied frame `seq` of channel `id`. Only the last frame sent is awaited:
    /// an ack of any other `seq` is ignored.
    ///
    /// # Errors
    /// `NotFound` for a channel `window` does not have open.
    pub(crate) fn ack(&self, window: Option<&str>, id: LinksId, seq: u32) -> Result<(), ApiError> {
        let open = self.lock();
        let channel = open
            .get(&window.map(str::to_owned))
            .filter(|c| c.id == id && !c.task.is_finished())
            .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "unknown link channel"))?;
        channel.acks.send_if_modified(|(sent, acked)| {
            let fresh = *sent == seq && !*acked;
            *acked |= fresh;
            fresh
        });
        Ok(())
    }
    /// `window` is gone or reloads: drop its channel.
    pub(crate) fn close(&self, window: &str) {
        drop(self.lock().remove(&Some(window.to_owned())));
    }
}

/// One channel: send the state, await its ack and a change, keep the spacing, repeat.
async fn run(
    mut view: watch::Receiver<LinkFrame>,
    mut acked: watch::Receiver<(u32, bool)>,
    acks: Arc<Acks>,
    clock: Arc<dyn Clock>,
    sink: impl LinkSink,
) {
    loop {
        let frame = *view.borrow_and_update();
        let seq = frame.seq;
        acks.send_replace((seq, false));
        if sink.send(frame).is_err() {
            return;
        }
        let sent = clock.pacing_now();
        if acked.wait_for(|&(s, a)| s == seq && a).await.is_err() || view.changed().await.is_err() {
            return;
        }
        let spacing = u64::try_from(SPACING.as_micros()).unwrap_or(u64::MAX);
        clock
            .pacing_sleep_until(MonoTime(sent.0.saturating_add(spacing)))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{DeviceLinkView, LinkPhase, LinkSample, LinkView};
    use crank_engine::VirtualClock;
    use tokio::{sync::mpsc, time::sleep};

    fn state(phase: LinkPhase, bpm: Option<u8>) -> LinkView {
        LinkView {
            trainer: Some(DeviceLinkView {
                phase,
                sample: None,
            }),
            strap: Some(DeviceLinkView {
                phase,
                sample: bpm.map(|bpm| LinkSample::HeartRate { bpm: Some(bpm) }),
            }),
        }
    }
    struct Rig {
        channels: Channels,
        view: watch::Sender<LinkFrame>,
        frames: mpsc::UnboundedReceiver<LinkFrame>,
        id: LinksId,
    }
    fn rig() -> Rig {
        let channels = Channels::new();
        let view = watch::Sender::new(LinkFrame {
            seq: 1,
            links: state(LinkPhase::Cold, None),
        });
        let (tx, frames) = mpsc::unbounded_channel();
        let id = channels.open(
            Some("main".into()),
            view.subscribe(),
            Arc::new(VirtualClock::new(1)),
            move |f| tx.send(f).map_err(|_| SinkClosed),
        );
        Rig {
            channels,
            view,
            frames,
            id,
        }
    }
    /// The keeper's state changes to `links`: the next `seq`.
    fn set(view: &watch::Sender<LinkFrame>, links: LinkView) {
        view.send_modify(|frame| {
            frame.seq += 1;
            frame.links = links;
        });
    }
    /// Long enough for every frame that is due, spacing included.
    const SETTLE: Duration = Duration::from_millis(150);
    fn drain(frames: &mut mpsc::UnboundedReceiver<LinkFrame>) -> Vec<LinkFrame> {
        std::iter::from_fn(|| frames.try_recv().ok()).collect()
    }

    /// ADR-0002 §5.1.4 (round 2 of #470): the first frame is the state at the open, at once.
    #[tokio::test(start_paused = true)]
    async fn the_first_frame_is_the_state_at_the_open() {
        let mut rig = rig();
        sleep(Duration::from_millis(1)).await;
        assert_eq!(
            drain(&mut rig.frames),
            [LinkFrame {
                seq: 1,
                links: state(LinkPhase::Cold, None)
            }]
        );
    }

    /// ADR-0001 §4.3 under the ack window of 1: a stalled page gets nothing past the one
    /// frame it has not acknowledged, however many changes come, and when it acks it gets
    /// exactly one frame, the latest state.
    #[tokio::test(start_paused = true)]
    async fn a_stalled_page_gets_only_the_latest_state_once_it_acks() {
        let mut rig = rig();
        sleep(SETTLE).await;
        assert_eq!(drain(&mut rig.frames).len(), 1);
        for bpm in 60..160 {
            set(&rig.view, state(LinkPhase::Warm, Some(bpm)));
            sleep(Duration::from_millis(10)).await;
        }
        sleep(Duration::from_mins(1)).await;
        assert!(drain(&mut rig.frames).is_empty(), "nothing past the window");
        rig.channels.ack(Some("main"), rig.id, 1).unwrap();
        sleep(SETTLE).await;
        assert_eq!(
            drain(&mut rig.frames),
            [LinkFrame {
                seq: 101,
                links: state(LinkPhase::Warm, Some(159))
            }],
            "one frame: the latest state, with its number"
        );
    }

    /// An ack of a frame that is not the last one sent (stale, or never sent) opens nothing;
    /// a channel of another window, or an id this window does not hold, is `NotFound`.
    #[tokio::test(start_paused = true)]
    async fn only_the_ack_of_the_last_frame_opens_the_window() {
        let mut rig = rig();
        sleep(SETTLE).await;
        drain(&mut rig.frames);
        set(&rig.view, state(LinkPhase::Searching, None));
        for seq in [0, 2, 7] {
            rig.channels.ack(Some("main"), rig.id, seq).unwrap();
        }
        sleep(SETTLE).await;
        assert!(drain(&mut rig.frames).is_empty());
        let other = rig.channels.ack(Some("other"), rig.id, 1).unwrap_err();
        assert_eq!(other.code, ErrorCode::NotFound);
        let wrong = rig.channels.ack(Some("main"), LinksId(rig.id.0 + 1), 1);
        assert_eq!(wrong.unwrap_err().code, ErrorCode::NotFound);
        rig.channels.ack(Some("main"), rig.id, 1).unwrap();
        sleep(SETTLE).await;
        assert_eq!(drain(&mut rig.frames)[0].seq, 2);
    }

    /// A page that acks at once gets at most one frame per `SPACING` (10 Hz), each the state
    /// when it was sent, and no frame without a change.
    #[tokio::test(start_paused = true)]
    async fn frames_are_spaced_and_sent_only_on_change() {
        let mut rig = rig();
        let mut got = Vec::new();
        for bpm in 0..40 {
            set(&rig.view, state(LinkPhase::Warm, Some(bpm)));
            sleep(Duration::from_millis(25)).await;
            for f in drain(&mut rig.frames) {
                rig.channels.ack(Some("main"), rig.id, f.seq).unwrap();
                got.push((tokio::time::Instant::now(), f));
            }
        }
        // 40 changes over 1 s: at most one frame per 100 ms, plus the first
        assert!(got.len() <= 11, "{} frames", got.len());
        for pair in got.windows(2) {
            assert!(pair[1].0 - pair[0].0 >= SPACING);
        }
        assert!(got.windows(2).all(|p| p[1].1.seq > p[0].1.seq));
        let last = got.last().unwrap().1.seq;
        sleep(Duration::from_secs(5)).await;
        let after = drain(&mut rig.frames);
        assert!(after.len() <= 1, "the last change, then nothing");
        assert!(
            after
                .iter()
                .all(|f| *f == *rig.view.borrow() && f.seq > last)
        );
        rig.channels
            .ack(Some("main"), rig.id, after.last().map_or(last, |f| f.seq))
            .unwrap();
        sleep(Duration::from_secs(5)).await;
        assert!(drain(&mut rig.frames).is_empty(), "no change, no frame");
    }

    /// A reopen from the same window (a reload) and `close` drop the channel; a failed send
    /// drops it too.
    #[tokio::test(start_paused = true)]
    async fn reopen_close_and_a_failed_send_drop_the_channel() {
        let rig = rig();
        let (tx, mut second) = mpsc::unbounded_channel();
        let id = rig.channels.open(
            Some("main".into()),
            rig.view.subscribe(),
            Arc::new(VirtualClock::new(1)),
            move |f| tx.send(f).map_err(|_| SinkClosed),
        );
        sleep(SETTLE).await;
        let old = rig.channels.ack(Some("main"), rig.id, 1).unwrap_err();
        assert_eq!(old.code, ErrorCode::NotFound, "the reload's old channel");
        assert_eq!(drain(&mut second)[0].seq, 1);
        rig.channels.close("main");
        let closed = rig.channels.ack(Some("main"), id, 1).unwrap_err();
        assert_eq!(closed.code, ErrorCode::NotFound);

        let failing = rig.channels.open(
            Some("w".into()),
            rig.view.subscribe(),
            Arc::new(VirtualClock::new(1)),
            |_| Err(SinkClosed),
        );
        sleep(SETTLE).await;
        let gone = rig.channels.ack(Some("w"), failing, 1).unwrap_err();
        assert_eq!(gone.code, ErrorCode::NotFound);
        drop(rig.frames);
    }

    /// ADR-0001 §4.3's byte budget: the largest frame (both devices, every field at its
    /// widest value) serializes far under 8 000 bytes. A frame holds no string or list, so no
    /// state is larger.
    #[test]
    fn the_largest_frame_is_under_the_byte_budget() {
        let widest = |sample| {
            Some(DeviceLinkView {
                phase: LinkPhase::Connecting,
                sample: Some(sample),
            })
        };
        let frame = LinkFrame {
            seq: u32::MAX,
            links: LinkView {
                trainer: widest(LinkSample::Trainer {
                    power_w: Some(u16::MAX),
                    cadence_rpm: Some(u8::MAX),
                }),
                strap: widest(LinkSample::HeartRate { bpm: Some(u8::MAX) }),
            },
        };
        let bytes = serde_json::to_vec(&frame).unwrap().len();
        assert!(bytes <= 8_000, "{bytes} bytes");
        assert!(bytes < 300, "{bytes} bytes: a fixed, small shape");
    }
}
