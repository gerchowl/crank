//! The ride actor: state machine, tick assignment, loss rules, record (ADR-0001 §5).
use crate::control::{PolicyContext, PolicyStatus, RideActivity, TargetPolicy};
use crate::erg::{Compliance, Component, ErgPolicy, Machine, TrainerFollowing};
use crate::record::{METRICS, Mean, RecordError, RideRecorder, SecondRow};
use crate::stream::hub::Hub;
use crate::stream::types::StreamError;
use crate::stream::{
    LinkDevice, MetricId, StreamFrame, StreamId, StreamSnapshot, StreamStatus, SubId, SubscribeOpts,
};
use crate::ticks::{
    HR_HOLD_TICKS, Hold, LOSS_TICKS, STOPPED_TICKS, SessionTicks, TICKS_PER_SECOND,
    TRAINER_HOLD_TICKS, TickWindow, WorkoutTicks, ticks_to_seconds,
};
use crank_devices::{
    Clock, ControlMode, HeartRateLink, HrEvent, HrSample, LinkState, MonoTime, TrainerCommand,
    TrainerEvent, TrainerLink, TrainerObservation, TrainerSample,
};
use crank_domain::workout::{FlatBlock, Phase as BlockPhase};
use std::{
    collections::{BTreeMap, VecDeque},
    future::pending,
    sync::Arc,
};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot, watch};

/// Why a ride is paused (ADR-0001 §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseReason {
    /// The rider paused.
    User,
    /// Link lost, or 8 ticks without a trainer sample, and not yet re-applied.
    TrainerLost,
    /// Cadence and power were 0 for 3 s.
    StoppedPedalling,
}
/// Ride state: `idle → riding ⇄ paused(reason) → finished`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RideState {
    /// Nothing started (a ride may be loaded).
    Idle,
    /// Workout and session time advance.
    Riding,
    /// Session time advances, workout time and the block timer are frozen.
    Paused(PauseReason),
    /// Ended; the record is closed.
    Finished,
}
/// Engine status: the live cursor of the part that exists so far (stream frames: #219).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RideStatus {
    /// Current state.
    pub state: RideState,
    /// A ride is loaded.
    pub loaded: bool,
    /// Session time.
    pub session: SessionTicks,
    /// Workout time.
    pub workout: WorkoutTicks,
    /// Index of the current block, `None` without blocks or after the last one.
    pub block_index: Option<usize>,
    /// Workout ticks spent in the current block.
    pub block_elapsed: WorkoutTicks,
    /// Last reported trainer link state.
    pub trainer_link: Option<LinkState>,
    /// Last reported heart-rate link state.
    pub hr_link: Option<LinkState>,
    /// Samples that arrived after their tick had been processed (dropped).
    pub late_samples: u64,
    /// First recording failure of the ride; recording stops after it.
    pub record_error: Option<String>,
    /// Whether the trainer follows the commanded mode (ADR-0002 §4.4.1).
    pub trainer_following: TrainerFollowing,
    /// The required component that is `not_following` or `silent`, if any.
    pub not_following_component: Option<Component>,
    /// ERG is released for low cadence (ADR-0002 §7.4).
    pub erg_released: bool,
    /// Compliance tally (ADR-0002 §7.8).
    pub compliance: Compliance,
    /// The active health cap (DESIGN §15.2, §15.4); `None` when no cap applies.
    pub cap: Option<CapState>,
    /// The rider paused the ride and has not resumed. Independent of `state`: a trainer loss
    /// shows as `Paused(TrainerLost)` over a rider pause, and Resume is still the rider's to offer
    /// (`Pauses::reason` ranks the reasons for `state`, it does not clear the others).
    pub user_paused: bool,
    /// Session time at which the current trainer loss began; `None` while the trainer is not
    /// lost. A timestamp, not an elapsed time: it changes only at a transition, so the status
    /// does not change every tick, and a consumer works out "lost for how long" from the session
    /// time it already has (DESIGN §6.5, §6.6). A loss detected by silence is stamped when the
    /// engine decides it (8 silent ticks), not when the first sample was missing.
    pub trainer_lost_since: Option<SessionTicks>,
    /// The reconnect attempt the driver is on during the current loss (1-based, ADR-0002 §6);
    /// `None` while the trainer is not lost or before the first attempt of the loss.
    pub reconnect_attempt: Option<u32>,
    /// While a soft start after a recovered trainer loss is running (and until it is done):
    /// the target in watts the trainer is being eased back to. `None` otherwise, also for the
    /// soft start every ride begins with (#250) and for one owed on resume from a rider pause.
    pub soft_start_target_w: Option<u16>,
    /// How long the last trainer loss lasted, from the loss until the trainer was back (link
    /// ready and samples flowing). Set when it is back, cleared when the next loss begins.
    pub last_gap: Option<SessionTicks>,
    /// Session time at which the last loss ended (with `last_gap`): a consumer times "back
    /// after m:ss" from it.
    pub reconnected_at: Option<SessionTicks>,
}
/// The ride's health cap as reported in the status (DESIGN §15.4): under a cap only ERG
/// rides, and every ERG command is clamped to `cap_w`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapState {
    /// Cap in watts.
    pub cap_w: u16,
    /// Only ERG blocks are allowed.
    pub erg_only: bool,
}
/// A ride to run: its sequenced blocks and where its record goes.
pub struct RideSetup {
    /// Compiled blocks in order; empty for a free ride.
    pub blocks: Vec<FlatBlock>,
    /// Destination of the 1 Hz record.
    pub recorder: Box<dyn RideRecorder>,
    /// The ride's health cap in watts, already resolved by core from the profile and CP
    /// (DESIGN §15.2); `None` when no cap applies. The engine owns the clamp and the
    /// refusal, not the derivation (#244); detecting `CapExceeded` is #245.
    pub cap_w: Option<u16>,
}
/// The mode a ride ends in: 0 % resistance, one control page, the same mode as the user
/// pause (DESIGN §5.2) and the §7.4 release (ADR-0002 §7.9 as amended: "an FE-C trainer
/// keeps its last control page, so silence after ERG could leave the ERG target active",
/// #266). `Free` is two SIM pages and is not on the trainer until both are written.
const END_MODE: ControlMode = ControlMode::Resistance { pct: 0.0 };
/// How long the ride-active signal waits for the end mode to reach the trainer: 2 s, eight
/// writer ticks of the 4 Hz control slot (ADR-0002 §4.4).
const END_DELIVERY_US: u64 = 2_000_000;

/// Engine commands (ADR-0001 §5 sketch, the part of it this crate implements).
pub enum EngineCmd {
    /// Load a ride and mark the ride active.
    Load(RideSetup),
    /// Start the loaded ride.
    Start,
    /// Pause by the rider.
    Pause,
    /// End the rider's pause.
    Resume,
    /// Jump to the next block now.
    SkipBlock,
    /// Finish the ride, or unload it if it never started.
    Stop,
}
/// A command that cannot apply.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CommandError {
    /// No ride is loaded.
    #[error("no ride loaded")]
    NotLoaded,
    /// The command is not valid in the current state.
    #[error("not valid in this state")]
    WrongState,
    /// The engine has no trainer: a ride never starts without one (#95: "Start disabled
    /// until trainer: connected", "makes impossible: a ride starting without a
    /// connected trainer").
    #[error("no trainer")]
    NoTrainer,
    /// The trainer link is not `Ready` (connecting, lost or reconnecting).
    #[error("trainer not ready")]
    TrainerNotReady,
    /// A health cap is active and the ride has a `free`/SIM block or no blocks
    /// (`cap_requires_erg`, DESIGN §15.2).
    #[error("cap_requires_erg")]
    CapRequiresErg,
    /// A health cap is active and a block targets more than the cap
    /// (`cap_exceeded_target`, DESIGN §15.2).
    #[error("cap_exceeded_target")]
    CapExceededTarget,
    /// The record could not be opened.
    #[error(transparent)]
    Record(#[from] RecordError),
    /// The engine task has ended.
    #[error("engine stopped")]
    EngineStopped,
}
type Request = (EngineCmd, oneshot::Sender<Result<(), CommandError>>);
type Reply<T> = oneshot::Sender<Result<T, StreamError>>;

/// A stream request; the engine answers it between ticks and never waits on a consumer.
enum StreamReq {
    Open(
        Option<String>,
        Reply<(StreamId, mpsc::Receiver<StreamFrame>)>,
    ),
    Close(StreamId),
    CloseWindow(String),
    Subscribe(StreamId, MetricId, SubscribeOpts, Reply<SubId>),
    Unsubscribe(StreamId, SubId, Reply<()>),
    Ack(StreamId, u32),
    Snapshot(StreamId, Reply<StreamSnapshot>),
}

/// Send commands to a running engine and observe its status.
#[derive(Clone)]
pub struct EngineHandle {
    cmds: mpsc::Sender<Request>,
    streams: mpsc::Sender<StreamReq>,
    status: watch::Receiver<RideStatus>,
}
impl EngineHandle {
    /// Apply `cmd` and wait for the engine's verdict.
    ///
    /// # Errors
    /// The engine's [`CommandError`], or `EngineStopped` if it has ended.
    pub async fn send(&self, cmd: EngineCmd) -> Result<(), CommandError> {
        let (tx, rx) = oneshot::channel();
        self.cmds
            .send((cmd, tx))
            .await
            .map_err(|_| CommandError::EngineStopped)?;
        rx.await.map_err(|_| CommandError::EngineStopped)?
    }
    async fn stream<T>(&self, req: impl FnOnce(Reply<T>) -> StreamReq) -> Result<T, StreamError> {
        let (tx, rx) = oneshot::channel();
        self.streams
            .send(req(tx))
            .await
            .map_err(|_| StreamError::EngineStopped)?;
        rx.await.map_err(|_| StreamError::EngineStopped)?
    }
    /// Open a stream: the frames arrive on the receiver (ADR-0001 §4.3). With a
    /// `window` label, that window's previous streams are dropped first (page reload).
    /// The consumer acknowledges frames with [`EngineHandle::stream_ack`].
    ///
    /// # Errors
    /// `TooManyStreams` beyond 16 live streams, or `EngineStopped`.
    pub async fn open_stream(
        &self,
        window: Option<String>,
    ) -> Result<(StreamId, mpsc::Receiver<StreamFrame>), StreamError> {
        self.stream(|r| StreamReq::Open(window, r)).await
    }
    /// Drop a stream and its subscriptions.
    ///
    /// # Errors
    /// `EngineStopped` if the engine has ended.
    pub async fn close_stream(&self, stream: StreamId) -> Result<(), StreamError> {
        self.streams
            .send(StreamReq::Close(stream))
            .await
            .map_err(|_| StreamError::EngineStopped)
    }
    /// Drop every stream of a window (its `Destroyed` event).
    ///
    /// # Errors
    /// `EngineStopped` if the engine has ended.
    pub async fn close_window(&self, window: String) -> Result<(), StreamError> {
        self.streams
            .send(StreamReq::CloseWindow(window))
            .await
            .map_err(|_| StreamError::EngineStopped)
    }
    /// Subscribe to a metric with its own filter chain and rate.
    ///
    /// # Errors
    /// Unknown stream, more than 64 subscriptions, a rate outside 0.1–10 Hz, an invalid
    /// filter parameter, or `EngineStopped`.
    pub async fn stream_subscribe(
        &self,
        stream: StreamId,
        metric: MetricId,
        opts: SubscribeOpts,
    ) -> Result<SubId, StreamError> {
        self.stream(|r| StreamReq::Subscribe(stream, metric, opts, r))
            .await
    }
    /// End a subscription.
    ///
    /// # Errors
    /// Unknown stream or subscription, or `EngineStopped`.
    pub async fn stream_unsubscribe(
        &self,
        stream: StreamId,
        sub: SubId,
    ) -> Result<(), StreamError> {
        self.stream(|r| StreamReq::Unsubscribe(stream, sub, r))
            .await
    }
    /// The consumer has applied every frame up to and including `seq`.
    ///
    /// # Errors
    /// `EngineStopped` if the engine has ended.
    pub async fn stream_ack(&self, stream: StreamId, seq: u32) -> Result<(), StreamError> {
        self.streams
            .send(StreamReq::Ack(stream, seq))
            .await
            .map_err(|_| StreamError::EngineStopped)
    }
    /// The state a window re-reads after `resync` or a `seq` gap.
    ///
    /// # Errors
    /// Unknown stream, or `EngineStopped`.
    pub async fn stream_snapshot(&self, stream: StreamId) -> Result<StreamSnapshot, StreamError> {
        self.stream(|r| StreamReq::Snapshot(stream, r)).await
    }
    /// Latest status.
    #[must_use]
    pub fn status(&self) -> RideStatus {
        self.status.borrow().clone()
    }
    /// A receiver that wakes on every status change.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<RideStatus> {
        self.status.clone()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Pauses {
    user: bool,
    lost: bool,
    stopped: bool,
}
impl Pauses {
    fn reason(self) -> Option<PauseReason> {
        if self.lost {
            Some(PauseReason::TrainerLost)
        } else if self.user {
            Some(PauseReason::User)
        } else if self.stopped {
            Some(PauseReason::StoppedPedalling)
        } else {
            None
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct TrainerValues {
    power: Option<u16>,
    cadence: Option<u8>,
    speed: Option<f32>,
}
/// Link transitions seen on one device link, in channel order.
///
/// Driver link events carry no timestamp, so each is stamped with the engine clock on
/// receipt and applied before the tick that contains the stamp. `received` counts
/// events in channel order and tags every sample that arrives after them; a sample
/// whose tag is below `applied` has an applied link transition after it and is stale.
#[derive(Default)]
struct LinkLog {
    received: u64,
    applied: u64,
    queue: VecDeque<(MonoTime, LinkItem)>,
}
/// What a link log queues: a link transition, or the driver starting a reconnect attempt.
/// Attempts ride the same queue as the transitions so they apply in the order they arrived
/// (attempt 1 comes after the `Lost` it belongs to); only transitions are counted in
/// `received`/`applied`, which tag samples.
#[derive(Clone, Copy)]
enum LinkItem {
    State(LinkState),
    Attempt(u32),
}
#[derive(Default)]
struct SecondAcc {
    power: Mean,
    cadence: Mean,
    speed: Mean,
    hr: Mean,
    riding: bool,
}
#[derive(PartialEq, Eq)]
enum Phase {
    Loaded,
    Active,
    Finished,
}
struct Session {
    phase: Phase,
    blocks: Vec<FlatBlock>,
    recorder: Box<dyn RideRecorder>,
    window: TickWindow,
    next_tick: u64,
    pending_trainer: BTreeMap<u64, (u64, TrainerSample)>,
    pending_hr: BTreeMap<u64, (u64, HrSample)>,
    trainer: Hold<TrainerValues>,
    hr: Hold<f64>,
    recent_power: VecDeque<(u64, f64)>,
    missed: u32,
    zero_run: u32,
    pauses: Pauses,
    clock_ticks: u64,
    ride_ticks: u64,
    block_idx: usize,
    block_elapsed: u64,
    last_sent: Option<ControlMode>,
    flow_start: Option<u64>,
    /// Counts every applied link transition and silence.
    link_epoch: u64,
    cap_w: Option<u16>,
    policy: Box<dyn TargetPolicy>,
    following: Machine,
    observations: BTreeMap<(u64, u64), TrainerObservation>,
    observed: u64,
    acc: SecondAcc,
    record_error: Option<String>,
    late: u64,
    /// Session tick at which the current trainer loss began.
    lost_at: Option<u64>,
    reconnect_attempt: Option<u32>,
    /// The trainer is back after a loss and its soft start has not finished yet.
    after_loss: bool,
    last_gap: Option<u64>,
    reconnected_at: Option<u64>,
}
impl Session {
    fn reset_control(&mut self) {
        self.last_sent = None;
    }
    /// The trainer is lost: pause for it and stamp the loss (the first stamp of a loss stands, also
    /// when a silence epoch begins while it is already lost).
    fn lose(&mut self) {
        self.pauses.lost = true;
        if self.lost_at.is_none() {
            self.lost_at = Some(self.clock_ticks);
            self.reconnect_attempt = None;
            self.after_loss = false;
            self.last_gap = None;
            self.reconnected_at = None;
        }
    }
    /// The trainer is back once its link is ready and a sample of the new epoch has arrived.
    fn recover_if_back(&mut self, link_ready: bool) {
        if self.lost_at.is_some() && link_ready && self.missed == 0 {
            self.recover();
        }
    }
    /// The soft start after a recovered loss is over once the policy owes none.
    fn end_after_loss(&mut self) {
        if self.after_loss && self.policy.status().soft_start_target_w.is_none() {
            self.after_loss = false;
        }
    }
    /// The trainer is back: link ready and samples flowing again.
    fn recover(&mut self) {
        if let Some(at) = self.lost_at.take() {
            self.last_gap = Some(self.clock_ticks.saturating_sub(at));
            self.reconnected_at = Some(self.clock_ticks);
            self.reconnect_attempt = None;
            self.after_loss = true;
        }
    }
    /// The engine's half of the single ride-start guard under a health cap (DESIGN §15.2):
    /// only ERG rides, and no target above the cap. The medical stop outranks it.
    fn check_cap(&self) -> Result<(), CommandError> {
        let Some(cap) = self.cap_w else {
            return Ok(());
        };
        if self.blocks.is_empty() || self.blocks.iter().any(|b| b.phase == BlockPhase::Free) {
            return Err(CommandError::CapRequiresErg);
        }
        if self
            .blocks
            .iter()
            .any(|b| b.power_from.max(b.power_to) > f64::from(cap) + 0.5)
        {
            return Err(CommandError::CapExceededTarget);
        }
        Ok(())
    }
    fn append_second(&mut self) {
        let acc = std::mem::take(&mut self.acc);
        let row = SecondRow {
            power: acc.power.mean(),
            cadence: acc.cadence.mean(),
            speed: acc.speed.mean(),
            hr: acc.hr.mean(),
            workout_s: acc.riding.then(|| {
                #[expect(clippy::cast_precision_loss, reason = "ride lengths fit f64 exactly")]
                let w = self.ride_ticks as f64;
                #[expect(clippy::cast_possible_truncation, reason = "the record is f32")]
                let v = (w / 4.0) as f32;
                v
            }),
            // The block of the last riding tick: `append_second` runs before the tick
            // advances the cursor, so a block's final second still belongs to it.
            block: acc
                .riding
                .then(|| u32::try_from(self.block_idx).ok())
                .flatten(),
        };
        let second = u32::try_from(self.clock_ticks / TICKS_PER_SECOND - 1).unwrap_or(u32::MAX);
        if self.record_error.is_none()
            && let Err(e) = self.recorder.append_second(second, &row)
        {
            self.record_error = Some(e.to_string());
        }
    }
    /// Forget everything the ramp learned on the previous epoch: the 3 s power window,
    /// the flow start and the origin, so pre-loss power is never an origin and its ramp
    /// never carries over (ADR-0002 §7.5, #231). Not in here, handled by the callers:
    /// the trainer hold (`apply_links`), the last command (`reset_control`) and the
    /// link epoch (`link_epoch`), which the policy repays with its soft start.
    fn reset_soft_start(&mut self) {
        self.recent_power.clear();
        self.flow_start = None;
    }
    /// Samples flow while a pedalling trainer value is present.
    fn track_flow(&mut self, k: u64, pedalling: bool) {
        if !pedalling {
            self.flow_start = None;
        } else if self.flow_start.is_none() {
            self.flow_start = Some(k);
        }
    }
    fn power_3s(&self) -> Option<f64> {
        let n = u32::try_from(self.recent_power.len())
            .ok()
            .filter(|n| *n > 0)?;
        Some(self.recent_power.iter().map(|(_, p)| p).sum::<f64>() / f64::from(n))
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "rounded, finite and at least 1 before the cast"
)]
fn block_ticks(dur_s: f64) -> u64 {
    if dur_s.is_finite() {
        (dur_s * 4.0).round().max(1.0) as u64
    } else {
        1
    }
}
async fn recv<T>(rx: &mut Option<mpsc::Receiver<T>>) -> Option<T> {
    match rx {
        Some(rx) => rx.recv().await,
        None => pending().await,
    }
}

/// The ride engine actor. Owns all ride state; no locks (ADR-0001 §2).
///
/// Time: tick `k` covers `[t0 + k·250 ms, t0 + (k+1)·250 ms)` from the instant of
/// `Start`; a tick is processed once the clock has passed its window, so after a stall
/// the missed ticks replay in order with their own samples (ADR-0001 §5).
pub struct Engine {
    clock: Arc<dyn Clock>,
    activity: Arc<dyn RideActivity>,
    /// Builds one policy per loaded ride: nothing a ride's policy learned can reach the
    /// next (a policy is owned by its [`Session`]).
    policy_factory: Box<dyn Fn() -> Box<dyn TargetPolicy> + Send>,
    trainer_events: Option<mpsc::Receiver<TrainerEvent>>,
    trainer_control: Option<mpsc::Sender<TrainerCommand>>,
    max_target_w: u16,
    hr_events: Option<mpsc::Receiver<HrEvent>>,
    trainer_link: Option<LinkState>,
    hr_link: Option<LinkState>,
    cmds: mpsc::Receiver<Request>,
    stream_reqs: Option<mpsc::Receiver<StreamReq>>,
    hub: Hub,
    pub(crate) status: watch::Sender<RideStatus>,
    trainer_log: LinkLog,
    hr_log: LinkLog,
    session: Option<Session>,
    /// The end mode was sent and the ride-active signal is held until the driver reports it
    /// delivered, the link is lost, or the deadline passes (#266).
    ending: Option<(ControlMode, MonoTime)>,
}
impl Engine {
    /// Build an engine over connected links. `activity` receives the ride-active
    /// signal ([`RideActivity`]); each ride gets a fresh [`ErgPolicy`] unless
    /// [`Engine::with_policy_factory`] says otherwise.
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        trainer: Option<TrainerLink>,
        hr: Option<HeartRateLink>,
        activity: Arc<dyn RideActivity>,
    ) -> (Self, EngineHandle) {
        let (cmd_tx, cmds) = mpsc::channel(16);
        let (stream_tx, stream_reqs) = mpsc::channel(64);
        let (trainer_link, hr_link) = (
            trainer.as_ref().map(|_| LinkState::Ready),
            hr.as_ref().map(|_| LinkState::Ready),
        );
        let (max_target_w, trainer_control, trainer_events) = match trainer {
            Some(t) => (t.caps.max_target_w, Some(t.control), Some(t.events)),
            None => (0, None, None),
        };
        let engine = Self {
            clock,
            activity,
            policy_factory: Box::new(|| Box::new(ErgPolicy::new())),
            trainer_events,
            trainer_control,
            max_target_w,
            hr_events: hr.map(|h| h.events),
            trainer_link,
            hr_link,
            cmds,
            stream_reqs: Some(stream_reqs),
            hub: Hub::new(),
            status: watch::channel(RideStatus {
                state: RideState::Idle,
                loaded: false,
                session: SessionTicks(0),
                workout: WorkoutTicks(0),
                block_index: None,
                block_elapsed: WorkoutTicks(0),
                trainer_link,
                hr_link,
                late_samples: 0,
                record_error: None,
                trainer_following: TrainerFollowing::Unknown,
                not_following_component: None,
                erg_released: false,
                compliance: Compliance::default(),
                cap: None,
                user_paused: false,
                trainer_lost_since: None,
                reconnect_attempt: None,
                soft_start_target_w: None,
                last_gap: None,
                reconnected_at: None,
            })
            .0,
            trainer_log: LinkLog::default(),
            hr_log: LinkLog::default(),
            session: None,
            ending: None,
        };
        let handle = EngineHandle {
            cmds: cmd_tx,
            streams: stream_tx,
            status: engine.status.subscribe(),
        };
        (engine, handle)
    }
    /// Replace the control policy: `factory` is called on every `Load`, so a ride's policy
    /// state cannot outlive the ride (see [`TargetPolicy`]).
    #[must_use]
    pub fn with_policy_factory(
        mut self,
        factory: impl Fn() -> Box<dyn TargetPolicy> + Send + 'static,
    ) -> Self {
        self.policy_factory = Box::new(factory);
        self
    }

    /// Run until every [`EngineHandle`] is dropped.
    pub async fn run(mut self) {
        enum Wake {
            Cmd(Option<Request>),
            Trainer(Option<TrainerEvent>),
            Hr(Option<HrEvent>),
            Stream(Option<StreamReq>),
            Timer,
        }
        loop {
            self.catch_up();
            // Ticks are due on the (possibly simulated-fast) engine clock; frames and
            // heartbeats on its unscaled pacing axis (ADR-0001 §4.3).
            let due = self.next_due();
            let pace_due = self
                .hub
                .next_due_us(self.clock.pacing_now().0)
                .map(MonoTime);
            let clock = self.clock.clone();
            let sleep = async move {
                match due {
                    Some(at) => clock.sleep_until(at).await,
                    None => pending().await,
                }
            };
            let clock = self.clock.clone();
            let pace = async move {
                match pace_due {
                    Some(at) => clock.pacing_sleep_until(at).await,
                    None => pending().await,
                }
            };
            let wake = tokio::select! {
                c = self.cmds.recv() => Wake::Cmd(c),
                e = recv(&mut self.trainer_events) => Wake::Trainer(e),
                e = recv(&mut self.hr_events) => Wake::Hr(e),
                r = recv(&mut self.stream_reqs) => Wake::Stream(r),
                () = sleep => Wake::Timer,
                () = pace => Wake::Timer,
            };
            match wake {
                Wake::Cmd(None) => {
                    // Every handle is gone: end the ride cleanly (record closed, gate
                    // cleared) instead of leaving a live ride record and an active gate.
                    self.catch_up();
                    self.end();
                    self.await_end().await;
                    return;
                }
                Wake::Cmd(Some((cmd, reply))) => {
                    // Ticks already due belong before the command's effect.
                    self.catch_up();
                    let _ = reply.send(self.command(cmd));
                }
                Wake::Trainer(Some(e)) => self.on_trainer_event(e),
                Wake::Trainer(None) => {
                    self.trainer_events = None;
                    self.on_trainer_event(TrainerEvent::Link(LinkState::Lost));
                }
                Wake::Hr(Some(e)) => self.on_hr_event(e),
                Wake::Hr(None) => {
                    self.hr_events = None;
                    self.on_hr_event(HrEvent::Link(LinkState::Lost));
                }
                Wake::Stream(Some(req)) => {
                    self.catch_up();
                    self.stream_request(req);
                }
                Wake::Stream(None) => self.stream_reqs = None,
                Wake::Timer => {}
            }
            self.publish();
        }
    }

    fn next_due(&self) -> Option<MonoTime> {
        let tick = self
            .session
            .as_ref()
            .filter(|s| s.phase == Phase::Active)
            .map(|s| s.window.end_of(s.next_tick));
        match (tick, self.ending) {
            (Some(t), Some((_, d))) => Some(t.min(d)),
            (t, e) => t.or(e.map(|(_, d)| d)),
        }
    }
    /// Wait, with every handle gone, until the end mode is delivered or given up on: the
    /// actor must not shut down with the command still queued (#266).
    async fn await_end(&mut self) {
        while let Some((_, deadline)) = self.ending {
            let clock = self.clock.clone();
            tokio::select! {
                e = recv(&mut self.trainer_events) => {
                    if let Some(e) = e {
                        self.on_trainer_event(e);
                    } else {
                        self.trainer_events = None;
                        self.on_trainer_event(TrainerEvent::Link(LinkState::Lost));
                    }
                }
                () = clock.sleep_until(deadline) => self.release_gate(),
            }
        }
    }
    /// The end mode is out of the way (or never will be): clear the ride-active signal.
    ///
    /// A ride loaded (or started) while the previous end mode was in flight keeps the signal
    /// up: it is that ride's own end that clears it (#279).
    fn release_gate(&mut self) {
        self.ending = None;
        if !self
            .session
            .as_ref()
            .is_some_and(|s| matches!(s.phase, Phase::Loaded | Phase::Active))
        {
            self.activity.set_active(false);
        }
    }
    /// A loaded ride is unloaded. The signal clears now unless a previous ride's end mode is
    /// still owed: then its delivery clears it (#279, ADR-0002 §7.9).
    fn unload(&mut self) {
        self.session = None;
        if self.ending.is_none() {
            self.activity.set_active(false);
        }
    }
    pub(crate) fn catch_up(&mut self) {
        let now = self.clock.now();
        self.catch_up_to(now);
    }
    fn catch_up_to(&mut self, now: MonoTime) {
        self.drain();
        if self.ending.is_some_and(|(_, deadline)| now >= deadline) {
            self.release_gate();
        }
        while self
            .session
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Active)
            && self.next_due().is_some_and(|due| now >= due)
        {
            self.process_tick();
        }
        self.publish();
    }
    pub(crate) fn drain(&mut self) {
        use mpsc::error::TryRecvError::{Disconnected, Empty};
        while let Some(rx) = self.trainer_events.as_mut() {
            match rx.try_recv() {
                Ok(e) => self.on_trainer_event(e),
                Err(Empty) => break,
                Err(Disconnected) => {
                    self.trainer_events = None;
                    self.on_trainer_event(TrainerEvent::Link(LinkState::Lost));
                }
            }
        }
        while let Some(rx) = self.hr_events.as_mut() {
            match rx.try_recv() {
                Ok(e) => self.on_hr_event(e),
                Err(Empty) => break,
                Err(Disconnected) => {
                    self.hr_events = None;
                    self.on_hr_event(HrEvent::Link(LinkState::Lost));
                }
            }
        }
    }
    /// Samples keep their driver timestamp; a link event has none and is stamped with
    /// the engine clock on receipt (see [`LinkLog`]).
    pub(crate) fn on_trainer_event(&mut self, event: TrainerEvent) {
        let now = self.clock.now();
        let active = self.active();
        match event {
            TrainerEvent::Sample(sample) => {
                let tag = self.trainer_log.received;
                if let Some(s) = self.session.as_mut().filter(|_| active) {
                    match s.window.tick_of(sample.at) {
                        Some(k) if k >= s.next_tick => {
                            s.pending_trainer.insert(k, (tag, sample));
                        }
                        Some(_) => s.late += 1,
                        None => {}
                    }
                }
            }
            TrainerEvent::Reconnecting { attempt } => {
                // Only a running ride tracks a loss; the attempt is applied in order with the
                // link transitions around it.
                if active {
                    self.trainer_log
                        .queue
                        .push_back((now, LinkItem::Attempt(attempt)));
                }
            }
            TrainerEvent::Link(state) => {
                self.trainer_log.received += 1;
                if active {
                    self.trainer_log
                        .queue
                        .push_back((now, LinkItem::State(state)));
                } else {
                    self.trainer_log.applied = self.trainer_log.received;
                    self.trainer_link = Some(state);
                    // The end mode can no longer be delivered on a link that is gone.
                    if state != LinkState::Ready && self.ending.is_some() {
                        self.release_gate();
                    }
                }
            }
            TrainerEvent::Delivered { mode } => {
                if self.ending.is_some_and(|(m, _)| m == mode) {
                    self.release_gate();
                }
            }
            // Page-71 echoes and page-25 flags feed the following machine, in tick order
            // like samples. A reply that predates an applied link transition needs no tag:
            // the transition deactivates the machine before the tick's replies are fed in
            // `follow`, and it is only active again once `control` has re-sent the mode.
            TrainerEvent::Observed(obs) => {
                if let Some(s) = self.session.as_mut().filter(|_| active) {
                    // A late reply still counts, at the start of the next unprocessed tick.
                    let at = match s.window.tick_of(obs.at) {
                        Some(k) if k >= s.next_tick => obs.at.0,
                        Some(_) => s.window.end_of(s.next_tick.saturating_sub(1)).0,
                        None => return,
                    };
                    s.observed += 1;
                    s.observations.insert((at, s.observed), obs);
                }
            }
            // Delivery and faults are not following inputs (ADR-0002 §4.4.1).
            _ => {}
        }
    }
    fn on_hr_event(&mut self, event: HrEvent) {
        let now = self.clock.now();
        let active = self.active();
        match event {
            HrEvent::Sample(sample) => {
                let tag = self.hr_log.received;
                if let Some(s) = self.session.as_mut().filter(|_| active) {
                    match s.window.tick_of(sample.at) {
                        Some(k) if k >= s.next_tick => {
                            s.pending_hr.insert(k, (tag, sample));
                        }
                        Some(_) => s.late += 1,
                        None => {}
                    }
                }
            }
            HrEvent::Link(state) => {
                self.hr_log.received += 1;
                if active {
                    self.hr_log.queue.push_back((now, LinkItem::State(state)));
                } else {
                    self.hr_log.applied = self.hr_log.received;
                    self.hr_link = Some(state);
                }
            }
            _ => {}
        }
    }
    fn active(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Active)
    }
    /// Apply the link transitions stamped at or before tick `k`'s window.
    fn apply_links(&mut self, s: &mut Session, k: u64) {
        let window = s.window;
        let due = |at: MonoTime| window.tick_of(at).is_none_or(|t| t <= k);
        while let Some(&(at, item)) = self.trainer_log.queue.front() {
            if !due(at) {
                break;
            }
            self.trainer_log.queue.pop_front();
            let state = match item {
                LinkItem::Attempt(n) => {
                    if s.lost_at.is_some() {
                        s.reconnect_attempt = Some(n);
                    }
                    continue;
                }
                LinkItem::State(state) => state,
            };
            self.trainer_log.applied += 1;
            self.trainer_link = Some(state);
            // The driver never restores a mode (ADR-0002 §6): after any transition the
            // engine re-issues control from scratch, and nothing sampled on the old
            // link may be held across it.
            s.reset_control();
            s.trainer.clear();
            s.reset_soft_start();
            // Any applied transition owes a soft start (#231) and never re-engages a
            // release: the policy handles both off `link_epoch`.
            s.link_epoch += 1;
            if state == LinkState::Lost {
                s.lose();
            }
        }
        while let Some(&(at, item)) = self.hr_log.queue.front() {
            if !due(at) {
                break;
            }
            self.hr_log.queue.pop_front();
            let LinkItem::State(state) = item else {
                continue;
            };
            self.hr_log.applied += 1;
            self.hr_link = Some(state);
            s.hr.clear();
        }
    }

    fn process_tick(&mut self) {
        let Some(mut s) = self.session.take() else {
            return;
        };
        let k = s.next_tick;
        s.next_tick += 1;
        self.apply_links(&mut s, k);
        // The 3 s power window moves with tick time, not with arrivals.
        while s.recent_power.front().is_some_and(|(at, _)| k - at >= 12) {
            s.recent_power.pop_front();
        }
        match s.pending_trainer.remove(&k) {
            Some((tag, t)) if tag >= self.trainer_log.applied => {
                s.missed = 0;
                s.trainer.push(
                    k,
                    TrainerValues {
                        power: t.power_w,
                        cadence: t.cadence_rpm,
                        speed: t.speed_mps,
                    },
                );
                if let Some(p) = t.power_w {
                    s.recent_power.push_back((k, f64::from(p)));
                }
            }
            // none, or sampled before a link transition that has since been applied
            _ => s.missed = s.missed.saturating_add(1),
        }
        if let Some((_, h)) = s
            .pending_hr
            .remove(&k)
            .filter(|(tag, _)| *tag >= self.hr_log.applied)
            && let Some(bpm) = h.bpm
        {
            s.hr.push(k, f64::from(bpm));
        }
        let (trainer, hr) = (s.trainer.get(k), s.hr.get(k));
        // Samples are flowing while a pedalling trainer value is present. The soft start
        // runs only then and restarts, from the current 3 s power, whenever they return.
        s.track_flow(k, trainer.and_then(|t| t.cadence).is_some_and(|c| c > 0));
        // The tick is workout time iff nothing paused the ride when it began; a loss
        // detected on this tick takes effect on the next (no rollback, ADR-0001 §5).
        let riding = s.pauses.reason().is_none();
        s.clock_ticks += 1;
        if riding {
            s.ride_ticks += 1;
            s.block_elapsed += 1;
            s.acc.riding = true;
            s.acc
                .power
                .add(trainer.and_then(|t| t.power).map(f64::from));
            s.acc
                .cadence
                .add(trainer.and_then(|t| t.cadence).map(f64::from));
            s.acc
                .speed
                .add(trainer.and_then(|t| t.speed).map(f64::from));
            s.acc.hr.add(hr);
        }
        self.publish_source(&s, riding, trainer, hr);
        if s.clock_ticks % TICKS_PER_SECOND == 0 {
            s.append_second();
        }
        let mut finished = false;
        if riding
            && s.blocks
                .get(s.block_idx)
                .is_some_and(|b| s.block_elapsed >= block_ticks(b.dur_s))
        {
            s.block_idx += 1;
            s.block_elapsed = 0;
            finished = s.block_idx >= s.blocks.len();
        }
        // Every silence of LOSS_TICKS ticks is a new epoch, also one that starts while
        // the ride is already paused for a loss: power sampled before it is no origin.
        if s.missed == LOSS_TICKS {
            s.lose();
            s.link_epoch += 1;
            s.reset_control();
            s.reset_soft_start();
        }
        s.recover_if_back(self.trainer_link == Some(LinkState::Ready));
        let cadence = trainer.and_then(|t| t.cadence);
        if trainer.is_some_and(|t| t.power == Some(0) && t.cadence == Some(0)) {
            s.zero_run = s.zero_run.saturating_add(1);
        } else {
            s.zero_run = 0;
        }
        if riding && s.zero_run >= STOPPED_TICKS {
            s.pauses.stopped = true;
        }
        if s.pauses.stopped && cadence.is_some_and(|c| c > 0) {
            s.pauses.stopped = false;
            s.zero_run = 0;
        }
        if finished {
            // The finishing tick is a ride tick: account it (no command is sent).
            self.control(&mut s, k, false);
        } else {
            self.control(&mut s, k, true);
            // ADR-0001 §5: resume once the mode is re-applied (`control` sends only on
            // a ready link) and the rider pedals.
            if s.pauses.lost && s.last_sent.is_some() && cadence.is_some_and(|c| c > 0) {
                s.pauses.lost = false;
            }
            s.end_after_loss();
        }
        self.session = Some(s);
        if finished {
            self.finish();
        }
    }
    /// Feed the tick's values to the streams: the record's values, `null` while paused.
    fn publish_source(
        &mut self,
        s: &Session,
        riding: bool,
        trainer: Option<TrainerValues>,
        hr: Option<f64>,
    ) {
        let values = if riding {
            [
                trainer.and_then(|t| t.power).map(f64::from),
                trainer.and_then(|t| t.cadence).map(f64::from),
                trainer.and_then(|t| t.speed).map(f64::from),
                hr,
                Some(ticks_to_seconds(s.ride_ticks)),
            ]
        } else {
            [None; 5]
        };
        self.hub.on_source(ticks_to_seconds(s.clock_ticks), &values);
    }
    /// Feed the following machine with what happened up to the end of tick `k`: the
    /// link and desired mode as they stand, then the replies stamped in the tick.
    fn follow(&self, s: &mut Session, k: u64) {
        let end = s.window.end_of(k).0;
        let key = s.last_sent.and_then(|m| m.grid_key().ok());
        s.following
            .set_link(self.trainer_link == Some(LinkState::Ready), end);
        s.following.set_desired(key, end);
        let due: Vec<_> = s
            .observations
            .range(..(end, 0))
            .map(|(key, _)| *key)
            .collect();
        for key in due {
            if let Some(obs) = s.observations.remove(&key) {
                s.following.observe(&obs, key.0);
            }
        }
        s.following.tick(end);
    }
    /// Ask the policy about tick `k` and send what it wants. `send = false` is the
    /// finishing tick: the policy accounts for it (compliance) against the last block, and
    /// nothing is sent.
    fn control(&mut self, s: &mut Session, k: u64, send: bool) {
        self.follow(s, k);
        let last = s.blocks.len().saturating_sub(1);
        let ctx = PolicyContext {
            block: s.blocks.get(if send { s.block_idx } else { last }),
            block_index: if send { s.block_idx } else { last },
            block_elapsed_ticks: if send {
                s.block_elapsed
            } else {
                s.blocks.get(last).map_or(0, |b| block_ticks(b.dur_s))
            },
            tick: k,
            link_epoch: s.link_epoch,
            flow_start: s.flow_start,
            power_3s_w: s.power_3s(),
            cadence_rpm: s.trainer.get(k).and_then(|t| t.cadence),
            last_sent: s.last_sent,
            riding: s.pauses.reason().is_none(),
            user_paused: s.pauses.user,
            following: s.following.verdict(),
            max_target_w: self.max_target_w,
            cap_w: s.cap_w,
        };
        if !send {
            let _ = s.policy.desired(&ctx);
            return;
        }
        let control = self
            .trainer_control
            .as_ref()
            .filter(|_| self.trainer_link == Some(LinkState::Ready))
            // the previous ride's end mode is still owed: nothing is queued behind it, or
            // the writer's coalescing could replace page 48 with the new ride's first page
            .filter(|_| self.ending.is_none());
        let Some(control) = control else {
            s.policy.skipped(&ctx);
            return;
        };
        if let Some(mode) = s.policy.desired(&ctx)
            && s.last_sent != Some(mode)
            && control.try_send(TrainerCommand::SetMode(mode)).is_ok()
        {
            s.last_sent = Some(mode);
            self.follow(s, k);
        }
    }
    /// End whatever is loaded: finish a started ride, unload one that never started.
    fn end(&mut self) {
        match self.session.as_ref().map(|s| &s.phase) {
            Some(Phase::Active) => self.finish(),
            Some(Phase::Loaded) => self.unload(),
            _ => {}
        }
        self.publish();
    }
    /// Send [`END_MODE`] so the last ERG target is not left on the trainer (#266). `true`
    /// when the command is queued and the ride-active signal must wait for its delivery.
    fn send_end_mode(&mut self) -> bool {
        let Some(s) = self.session.as_mut() else {
            return false;
        };
        let control = self
            .trainer_control
            .as_ref()
            .filter(|_| self.trainer_link == Some(LinkState::Ready));
        let Some(control) = control else {
            return false;
        };
        // Always sent, even when the last command is already `END_MODE`: that command (a pause,
        // a §7.4 release) may still be queued, and only its delivery is waited for.
        // The control channel holds 64 commands and the engine sends at most one per tick,
        // so a full channel means a driver that is not running: nothing to wait for.
        if control.try_send(TrainerCommand::SetMode(END_MODE)).is_err() {
            return false;
        }
        s.last_sent = Some(END_MODE);
        let now = self.clock.now().0;
        self.ending = Some((END_MODE, MonoTime(now.saturating_add(END_DELIVERY_US))));
        true
    }
    fn finish(&mut self) {
        let mut ended = false;
        if let Some(s) = self.session.as_mut().filter(|s| s.phase == Phase::Active) {
            // Only complete seconds are recorded; a partial last second is dropped.
            s.phase = Phase::Finished;
            for (log, link) in [
                (&mut self.trainer_log, &mut self.trainer_link),
                (&mut self.hr_log, &mut self.hr_link),
            ] {
                if let Some(state) = log.queue.iter().rev().find_map(|(_, item)| match item {
                    LinkItem::State(state) => Some(*state),
                    LinkItem::Attempt(_) => None,
                }) {
                    *link = Some(state);
                }
                log.queue.clear();
                log.applied = log.received;
            }
            if s.record_error.is_none()
                && let Err(e) = s.recorder.finish()
            {
                s.record_error = Some(e.to_string());
            }
            ended = true;
        }
        // ADR-0002 §7.9 / #266: command the trainer out of ERG *before* the ride-active
        // signal clears (which lets the reconnect loop stand down and core release the
        // devices), and keep the signal until the driver confirms the write.
        if ended && self.send_end_mode() {
            return;
        }
        self.release_gate();
    }

    pub(crate) fn command(&mut self, cmd: EngineCmd) -> Result<(), CommandError> {
        let active = self
            .session
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Active);
        match cmd {
            EngineCmd::Load(setup) => {
                if active {
                    return Err(CommandError::WrongState);
                }
                // The previous ride's end mode (`self.ending`) stays owed across a `Load`: its
                // delivery or its deadline settle it, and a link loss only while no ride is
                // running (a link event of an active ride is queued; #279, #288). Until then
                // the new ride sends nothing (see `control`), so no torque can overtake page 48.
                self.session = Some(Session {
                    phase: Phase::Loaded,
                    blocks: setup.blocks,
                    recorder: setup.recorder,
                    window: TickWindow::new(MonoTime(0)),
                    next_tick: 0,
                    pending_trainer: BTreeMap::new(),
                    pending_hr: BTreeMap::new(),
                    trainer: Hold::new(TRAINER_HOLD_TICKS),
                    hr: Hold::new(HR_HOLD_TICKS),
                    recent_power: VecDeque::new(),
                    missed: 0,
                    zero_run: 0,
                    pauses: Pauses::default(),
                    clock_ticks: 0,
                    ride_ticks: 0,
                    block_idx: 0,
                    block_elapsed: 0,
                    last_sent: None,
                    flow_start: None,
                    link_epoch: 0,
                    cap_w: setup.cap_w,
                    policy: (self.policy_factory)(),
                    following: Machine::new(),
                    observations: BTreeMap::new(),
                    observed: 0,
                    acc: SecondAcc::default(),
                    record_error: None,
                    late: 0,
                    lost_at: None,
                    reconnect_attempt: None,
                    after_loss: false,
                    last_gap: None,
                    reconnected_at: None,
                });
                self.activity.set_active(true);
            }
            EngineCmd::Start => {
                let now = self.clock.now();
                let ready = self.trainer_link == Some(LinkState::Ready);
                let has_trainer = self.trainer_control.is_some();
                let s = self.session.as_mut().ok_or(CommandError::NotLoaded)?;
                if s.phase != Phase::Loaded {
                    return Err(CommandError::WrongState);
                }
                if !has_trainer {
                    return Err(CommandError::NoTrainer);
                }
                if !ready {
                    return Err(CommandError::TrainerNotReady);
                }
                s.check_cap()?;
                s.recorder.start(&METRICS)?;
                s.window = TickWindow::new(now);
                s.phase = Phase::Active;
            }
            EngineCmd::Pause | EngineCmd::Resume | EngineCmd::SkipBlock if !active => {
                return Err(if self.session.is_some() {
                    CommandError::WrongState
                } else {
                    CommandError::NotLoaded
                });
            }
            EngineCmd::Pause => {
                if let Some(s) = self.session.as_mut() {
                    s.pauses.user = true;
                }
            }
            EngineCmd::Resume => {
                let s = self.session.as_mut().filter(|s| s.pauses.user);
                s.ok_or(CommandError::WrongState)?.pauses.user = false;
            }
            EngineCmd::SkipBlock => {
                let s = self.session.as_mut().ok_or(CommandError::NotLoaded)?;
                if s.block_idx >= s.blocks.len() {
                    return Err(CommandError::WrongState);
                }
                s.block_idx += 1;
                s.block_elapsed = 0;
                if s.block_idx >= s.blocks.len() {
                    self.finish();
                }
            }
            EngineCmd::Stop => match self.session.as_ref().map(|s| &s.phase) {
                Some(Phase::Active) => self.finish(),
                Some(Phase::Loaded) => self.unload(),
                _ => return Err(CommandError::WrongState),
            },
        }
        self.publish();
        Ok(())
    }

    fn stream_request(&mut self, req: StreamReq) {
        let now = self.clock.pacing_now().0;
        // A dropped reply means the requester gave up; the request still applied.
        match req {
            StreamReq::Open(window, reply) => {
                let _ = reply.send(self.hub.open(window, now));
            }
            StreamReq::Close(id) => self.hub.close(id),
            StreamReq::CloseWindow(w) => self.hub.close_window(&w),
            StreamReq::Subscribe(id, metric, opts, reply) => {
                let _ = reply.send(self.hub.subscribe(id, metric, opts));
            }
            StreamReq::Unsubscribe(id, sub, reply) => {
                let _ = reply.send(self.hub.unsubscribe(id, sub));
            }
            StreamReq::Ack(id, seq) => {
                // An ack for a dropped stream is not an error worth surfacing.
                let _ = self.hub.ack(id, seq);
            }
            StreamReq::Snapshot(id, reply) => {
                let _ = reply.send(self.hub.snapshot(id));
            }
        }
    }

    fn publish(&mut self) {
        let s = self.session.as_ref();
        let policy: PolicyStatus = s.map_or_else(PolicyStatus::default, |s| s.policy.status());
        let state = match s {
            Some(s) if s.phase == Phase::Finished => RideState::Finished,
            Some(s) if s.phase == Phase::Active => s
                .pauses
                .reason()
                .map_or(RideState::Riding, RideState::Paused),
            _ => RideState::Idle,
        };
        let status = RideStatus {
            state,
            loaded: s.is_some(),
            session: SessionTicks(s.map_or(0, |s| s.clock_ticks)),
            workout: WorkoutTicks(s.map_or(0, |s| s.ride_ticks)),
            block_index: s.and_then(|s| (s.block_idx < s.blocks.len()).then_some(s.block_idx)),
            block_elapsed: WorkoutTicks(s.map_or(0, |s| s.block_elapsed)),
            trainer_link: self.trainer_link,
            hr_link: self.hr_link,
            late_samples: s.map_or(0, |s| s.late),
            record_error: s.and_then(|s| s.record_error.clone()),
            trainer_following: s.map_or(TrainerFollowing::Unknown, |s| s.following.verdict()),
            not_following_component: s.and_then(|s| s.following.blocking()),
            erg_released: policy.released,
            compliance: policy.compliance,
            cap: s.and_then(|s| s.cap_w).map(|cap_w| CapState {
                cap_w,
                erg_only: true,
            }),
            user_paused: s.is_some_and(|s| s.pauses.user),
            trainer_lost_since: s.and_then(|s| s.lost_at).map(SessionTicks),
            reconnect_attempt: s.and_then(|s| s.reconnect_attempt),
            soft_start_target_w: s.filter(|s| s.after_loss).and(policy.soft_start_target_w),
            last_gap: s.and_then(|s| s.last_gap).map(SessionTicks),
            reconnected_at: s.and_then(|s| s.reconnected_at).map(SessionTicks),
        };
        let now = self.clock.pacing_now().0;
        self.hub.on_status(StreamStatus::from(&status), now);
        for (device, link) in [
            (LinkDevice::Trainer, self.trainer_link),
            (LinkDevice::Hr, self.hr_link),
        ] {
            if let Some(link) = link {
                self.hub.on_link(device, link.into());
            }
        }
        self.hub.poll(now);
        self.status.send_replace(status);
    }
}
