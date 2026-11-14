//! The windows' snapshot channels (`ui_open`, ADR-0001 §4.3), built on the run loop of the
//! link channel (`keeper/channel.rs`).
//!
//! [`Ui`] holds what a frame is built from under one `std` mutex: the loaded ride (its
//! status watch and its series store, which the engine appends to under the store's own
//! lock), whether a load is under way, the pairings, and each channel's series cursors. A
//! frame is the full state ([`UiState`]) plus up to [`CHUNK`] points per series from the
//! channel's cursor; building it moves the cursor (§4.3.4: sending moves the cursor, the ack
//! only opens the window).
//!
//! - **Ack window of 1.** After a frame the channel sends nothing until `ui_ack` names it.
//! - **Rate.** A frame is due when the state differs from the last one sent, when points are
//!   past the cursor, or [`HEARTBEAT`] after the last frame; it is sent once the last is acked
//!   and [`SPACING`] after it on the unscaled pacing axis (§4.3.3).
//! - **Lifecycle by id** (§4.3.5). A channel ends when a send fails, when its window starts a
//!   page load or is destroyed ([`Ui::close_window`], synchronous, so it closes only the ids
//!   the window holds at the event), and when the same window opens another.
use crate::core::SinkClosed;
use crate::dto::{
    DeviceLinkView, DeviceView, DevicesView, LinkFrame, LinkPhase, LiveValues, PairedDevice,
    Points, RideEnding, RideView, SeriesDelta, UiChannelId, UiFrame, UiState,
};
use crate::error::{ApiError, ErrorCode};
use crank_devices::{Clock, LinkState, MonoTime};
use crank_engine::{LinkStatus, RideState, RideStatus, SeriesStore, StreamStatus, Track};
use std::{
    collections::BTreeMap,
    future::pending,
    sync::{Arc, Mutex, MutexGuard, PoisonError, Weak},
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle};

/// Least time between two frames of a channel: at most 10 per second.
pub(crate) const SPACING: Duration = Duration::from_millis(100);
/// Longest time between two frames of an open channel whose page acks.
pub(crate) const HEARTBEAT: Duration = Duration::from_secs(1);
/// Most channels open in a process.
pub(crate) const MAX_CHANNELS: usize = 16;
/// Most points per series in one frame.
pub(crate) const CHUNK: usize = 256;
/// Longest string of a [`PairedDevice`] in a frame, in bytes.
const MAX_STRING_BYTES: usize = 64;

/// Where a window's snapshot frames go.
pub trait UiSink: Send + 'static {
    /// Deliver one frame; an error closes the channel.
    ///
    /// # Errors
    /// [`SinkClosed`] when the window can no longer receive.
    fn send(&self, frame: UiFrame) -> Result<(), SinkClosed>;
}
impl<F: Fn(UiFrame) -> Result<(), SinkClosed> + Send + 'static> UiSink for F {
    fn send(&self, frame: UiFrame) -> Result<(), SinkClosed> {
        self(frame)
    }
}

/// The loaded ride, or the last ended one, as the snapshot reads it.
pub(crate) struct RideSlot {
    id: String,
    workout_id: Option<String>,
    status: watch::Receiver<RideStatus>,
    series: SeriesStore,
    /// Set by the core when it ended the ride by force; otherwise derived from the status.
    ended: Option<RideEnding>,
    unresponsive: bool,
    /// When a read last found the status watch written, on the core clock's pacing axis;
    /// `None` until the slot is installed.
    seen: Option<MonoTime>,
}
impl RideSlot {
    pub(crate) fn new(
        id: String,
        workout_id: Option<String>,
        status: watch::Receiver<RideStatus>,
        series: SeriesStore,
    ) -> Self {
        Self {
            id,
            workout_id,
            status,
            series,
            ended: None,
            unresponsive: false,
            seen: None,
        }
    }
}

/// The last frame sent, and whether the window acknowledged it.
type Acks = watch::Sender<(u32, bool)>;
struct Channel {
    window: Option<String>,
    /// Per series (power, cadence, heart rate): the index of the first point not yet sent.
    cursors: [usize; 3],
    acks: Arc<Acks>,
    task: JoinHandle<()>,
}
impl Drop for Channel {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
struct Model {
    ride: Option<RideSlot>,
    loading: bool,
    /// Trainer, then strap.
    paired: [Option<PairedDevice>; 2],
    channels: BTreeMap<UiChannelId, Channel>,
}

/// The snapshot state and the channels of one core.
pub(crate) struct Ui {
    model: Mutex<Model>,
    keeper: watch::Receiver<LinkFrame>,
    /// Bumped by every change of the model.
    changed: watch::Sender<u64>,
    /// The last id handed out.
    last: Mutex<u32>,
    /// How long a ride in progress may leave its status unwritten before it is unresponsive.
    unresponsive_after: Duration,
    /// The core's clock: its unscaled pacing axis times the frames and the unresponsive
    /// timer, which, like the core's request bound, does not scale with simulation speed.
    clock: Arc<dyn Clock>,
}
impl Ui {
    pub(crate) fn new(
        keeper: watch::Receiver<LinkFrame>,
        clock: Arc<dyn Clock>,
        unresponsive_after: Duration,
    ) -> Self {
        Self {
            model: Mutex::default(),
            keeper,
            changed: watch::Sender::new(0),
            last: Mutex::new(0),
            unresponsive_after,
            clock,
        }
    }
    fn lock(&self) -> MutexGuard<'_, Model> {
        self.model.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// Change the model and wake every channel.
    fn change(&self, f: impl FnOnce(&mut Model)) {
        f(&mut self.lock());
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
    }
    /// A load starts: `loading`, no ride, and every cursor back to 0.
    pub(crate) fn load_started(&self) {
        self.change(|m| {
            m.loading = true;
            m.ride = None;
            for c in m.channels.values_mut() {
                c.cursors = [0; 3];
            }
        });
    }
    /// The load is over, installed or not.
    pub(crate) fn load_over(&self) {
        self.change(|m| m.loading = false);
    }
    /// The load installed its ride.
    pub(crate) fn install(&self, mut ride: RideSlot) {
        ride.seen = Some(self.clock.pacing_now());
        self.change(|m| m.ride = Some(ride));
    }
    /// Ride `id` was unloaded without starting.
    pub(crate) fn unload(&self, id: &str) {
        self.change(|m| {
            if m.ride.as_ref().is_some_and(|r| r.id == id) {
                m.ride = None;
            }
        });
    }
    /// The core ended ride `id` by force (or found it finished doing so).
    pub(crate) fn ended(&self, id: &str, ending: RideEnding) {
        self.change(|m| {
            if let Some(r) = m.ride.as_mut().filter(|r| r.id == id) {
                r.ended = Some(ending);
            }
        });
    }
    /// A bounded request to the loaded ride's engine timed out.
    pub(crate) fn unresponsive(&self) {
        self.change(|m| {
            if let Some(r) = m.ride.as_mut() {
                r.unresponsive = true;
            }
        });
    }
    /// The pairings changed: trainer, then strap.
    pub(crate) fn set_paired(&self, paired: [Option<PairedDevice>; 2]) {
        self.change(|m| m.paired = paired.map(|p| p.map(cut)));
    }
    /// The state now, without series.
    pub(crate) fn snapshot(&self) -> UiState {
        let m = &mut *self.lock();
        self.read(&mut m.ride, m.loading, &m.paired, None).0
    }

    /// Open `window`'s channel, closing that window's older ones; its first frame, the state
    /// now, goes out at once.
    pub(crate) fn open(
        self: &Arc<Self>,
        window: Option<String>,
        sink: impl UiSink,
    ) -> Result<UiChannelId, ApiError> {
        let mut m = self.lock();
        let others = m.channels.values().filter(|c| c.window != window).count();
        if others >= MAX_CHANNELS {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "too many snapshot channels open",
            ));
        }
        let id = {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            *last = last
                .checked_add(1)
                .ok_or_else(|| ApiError::new(ErrorCode::Internal, "channel ids exhausted"))?;
            UiChannelId(*last)
        };
        let older: Vec<UiChannelId> = m
            .channels
            .iter()
            .filter(|(_, c)| c.window == window)
            .map(|(id, _)| *id)
            .collect();
        for old in &older {
            m.channels.remove(old);
        }
        if !older.is_empty() {
            tracing::info!(target: "crank_core::ui", ?older, new = id.0, "a reopen closed older channels");
        }
        let acks = Arc::new(watch::Sender::new((0, true)));
        let task = tokio::spawn(run(
            Arc::downgrade(self),
            id,
            acks.subscribe(),
            self.clock.clone(),
            sink,
        ));
        m.channels.insert(
            id,
            Channel {
                window,
                cursors: [0; 3],
                acks,
                task,
            },
        );
        Ok(id)
    }
    /// `window` applied frame `seq` of channel `id`. Only the last frame sent is awaited: an
    /// ack of any other `seq` is ignored.
    ///
    /// # Errors
    /// `NotFound` for a channel `window` does not hold.
    pub(crate) fn ack(
        &self,
        window: Option<&str>,
        id: UiChannelId,
        seq: u32,
    ) -> Result<(), ApiError> {
        let m = self.lock();
        let channel = m
            .channels
            .get(&id)
            .filter(|c| c.window.as_deref() == window && !c.task.is_finished())
            .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "unknown snapshot channel"))?;
        channel.acks.send_if_modified(|(sent, acked)| {
            let fresh = *sent == seq && !*acked;
            *acked |= fresh;
            fresh
        });
        Ok(())
    }
    /// `window` starts a page load or is destroyed: close the channels it holds now.
    pub(crate) fn close_window(&self, window: &str) {
        self.lock()
            .channels
            .retain(|_, c| c.window.as_deref() != Some(window));
    }
    fn close(&self, id: UiChannelId) {
        drop(self.lock().channels.remove(&id));
    }

    /// The state now and, with `cursors`, the points from them, moving them.
    fn read(
        &self,
        ride: &mut Option<RideSlot>,
        loading: bool,
        paired: &[Option<PairedDevice>; 2],
        mut cursors: Option<&mut [usize; 3]>,
    ) -> (UiState, SeriesDelta) {
        let now = self.clock.pacing_now();
        let mut series = SeriesDelta::default();
        let mut links = (None, None);
        let view = match ride.as_mut() {
            None => {
                // no ride, no series: the next one starts from 0
                if let Some(c) = cursors.as_mut() {
                    **c = [0; 3];
                }
                None
            }
            Some(slot) => {
                let store = slot.series.lock().unwrap_or_else(PoisonError::into_inner);
                if slot.status.has_changed().unwrap_or(false) {
                    slot.seen = Some(now);
                }
                let status = slot.status.borrow_and_update().clone();
                let wire = StreamStatus::from(&status);
                let silent_us = slot.seen.map_or(0, |seen| now.0.saturating_sub(seen.0));
                let after_us =
                    u64::try_from(self.unresponsive_after.as_micros()).unwrap_or(u64::MAX);
                if wire.in_progress() && silent_us >= after_us {
                    slot.unresponsive = true;
                }
                if let Some(c) = cursors {
                    series = SeriesDelta {
                        power: chunk(&store.power, &mut c[0]),
                        cadence: chunk(&store.cadence, &mut c[1]),
                        hr: chunk(&store.hr, &mut c[2]),
                    };
                }
                let live = store.live;
                drop(store);
                links = (status.trainer_link, status.hr_link);
                let ended = slot
                    .ended
                    .or((status.state == RideState::Finished).then_some(RideEnding::Finished));
                Some(RideView {
                    id: slot.id.clone(),
                    workout_id: slot.workout_id.clone(),
                    status: wire,
                    live: LiveValues {
                        power_w: live.power_w,
                        cadence_rpm: live.cadence_rpm,
                        hr_bpm: live.hr_bpm,
                    },
                    engine_unresponsive: slot.unresponsive && ended.is_none(),
                    ended,
                })
            }
        };
        let kept = self.keeper.borrow().links;
        let devices = DevicesView {
            trainer: device(paired[0].as_ref(), kept.trainer, links.0),
            strap: device(paired[1].as_ref(), kept.strap, links.1),
        };
        (
            UiState {
                ride: view,
                loading,
                devices,
            },
            series,
        )
    }
    /// The next frame of `id`, with the cursors moved past its points; `None` for a closed
    /// channel.
    fn build(&self, id: UiChannelId, seq: u32) -> Option<UiFrame> {
        let m = &mut *self.lock();
        let channel = m.channels.get_mut(&id)?;
        channel.acks.send_replace((seq, false));
        let (state, series) = self.read(
            &mut m.ride,
            m.loading,
            &m.paired,
            Some(&mut channel.cursors),
        );
        Some(UiFrame {
            channel: id,
            seq,
            ride: state.ride,
            loading: state.loading,
            devices: state.devices,
            series,
        })
    }
    /// Whether a frame of `id` is due after one with `last`: the state differs, or points are
    /// past a cursor. `None` for a closed channel.
    fn due(&self, id: UiChannelId, last: &UiState) -> Option<bool> {
        let m = &mut *self.lock();
        let cursors = m.channels.get(&id)?.cursors;
        let (state, _) = self.read(&mut m.ride, m.loading, &m.paired, None);
        let points = m.ride.as_ref().is_some_and(|r| {
            let s = r.series.lock().unwrap_or_else(PoisonError::into_inner);
            [s.power.len(), s.cadence.len(), s.hr.len()]
                .iter()
                .zip(cursors)
                .any(|(len, c)| *len > c)
        });
        Some(points || state != *last)
    }
    /// What a waiting channel wakes on, each marked seen: the model, the keeper and the ride's
    /// status.
    fn watchers(
        &self,
    ) -> (
        watch::Receiver<u64>,
        watch::Receiver<LinkFrame>,
        Option<watch::Receiver<RideStatus>>,
    ) {
        let status = self.lock().ride.as_ref().map(|r| {
            let mut rx = r.status.clone();
            rx.borrow_and_update();
            rx
        });
        let mut keeper = self.keeper.clone();
        keeper.borrow_and_update();
        (self.changed.subscribe(), keeper, status)
    }
}

/// The points of `track` from `cursor`, at most [`CHUNK`], moving the cursor past them.
fn chunk(track: &Track, cursor: &mut usize) -> Points {
    let from = (*cursor).min(track.len());
    let end = track.len().min(from + CHUNK);
    *cursor = end;
    // A series holds at most one point per 4 Hz tick of one ride, so u32 holds 34 years of it:
    // no process lives that long, and a larger index would be a core bug.
    debug_assert!(
        u32::try_from(from).is_ok(),
        "series index {from} exceeds u32"
    );
    #[expect(
        clippy::cast_possible_truncation,
        reason = "bounded by the ride's length: see the debug_assert"
    )]
    let from_u32 = from as u32;
    Points {
        from: from_u32,
        t: track.t[from..end].to_vec(),
        w: track.w[from..end].to_vec(),
        v: track.v[from..end].to_vec(),
    }
}

/// One role (§4.3.6): the keeper's phase when paired, overlaid with the ride's link while the
/// ride holds the device.
fn device(
    paired: Option<&PairedDevice>,
    kept: Option<DeviceLinkView>,
    ride_link: Option<LinkState>,
) -> DeviceView {
    // The keeper learns of a pairing by a note it may not have taken yet; a device it has not
    // seen paired has no attempt running, so it is `cold`.
    let phase = paired.map(|_| kept.map_or(LinkPhase::Cold, |d| d.phase));
    let riding = phase == Some(LinkPhase::Riding);
    DeviceView {
        paired: paired.cloned(),
        phase,
        link: riding.then(|| ride_link.map_or(LinkStatus::Connecting, Into::into)),
        sample: kept
            .and_then(|d| d.sample)
            .filter(|_| phase == Some(LinkPhase::Warm)),
    }
}

/// `p` with each string cut to [`MAX_STRING_BYTES`] on a character boundary.
fn cut(mut p: PairedDevice) -> PairedDevice {
    let cut = |s: &mut String| {
        let mut end = s.len().min(MAX_STRING_BYTES);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    };
    cut(&mut p.id);
    for s in [&mut p.name, &mut p.serial, &mut p.model]
        .into_iter()
        .flatten()
    {
        cut(s);
    }
    p
}

/// Wait for a change of `rx`; never, without one or once its sender is gone.
async fn changed<T>(rx: Option<&mut watch::Receiver<T>>) {
    if let Some(rx) = rx
        && rx.changed().await.is_ok()
    {
        return;
    }
    pending::<()>().await;
}

/// One channel: send the state, await its ack, keep the spacing, await a due frame, repeat.
async fn run(
    ui: Weak<Ui>,
    id: UiChannelId,
    mut acked: watch::Receiver<(u32, bool)>,
    clock: Arc<dyn Clock>,
    sink: impl UiSink,
) {
    let micros = |d: Duration| u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
    let mut seq = 0_u32;
    loop {
        let Some(core) = ui.upgrade() else { return };
        let Some(frame) = core.build(id, seq) else {
            return;
        };
        let last = UiState {
            ride: frame.ride.clone(),
            loading: frame.loading,
            devices: frame.devices.clone(),
        };
        if sink.send(frame).is_err() {
            core.close(id);
            return;
        }
        drop(core);
        let sent = clock.pacing_now();
        if acked.wait_for(|&(s, a)| s == seq && a).await.is_err() {
            return;
        }
        clock
            .pacing_sleep_until(MonoTime(sent.0.saturating_add(micros(SPACING))))
            .await;
        let heartbeat = MonoTime(sent.0.saturating_add(micros(HEARTBEAT)));
        loop {
            let Some(core) = ui.upgrade() else { return };
            let (mut model, mut keeper, mut status) = core.watchers();
            match core.due(id, &last) {
                None => return,
                Some(true) => break,
                Some(false) => {}
            }
            drop(core);
            tokio::select! {
                () = changed(Some(&mut model)) => {}
                () = changed(Some(&mut keeper)) => {}
                () = changed(status.as_mut()) => {}
                () = clock.pacing_sleep_until(heartbeat) => break,
            }
        }
        seq = seq.wrapping_add(1);
    }
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
