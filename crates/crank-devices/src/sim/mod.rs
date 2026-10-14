//! Deterministic simulated trainer and heart-rate strap (ADR-0002 §3, port of the
//! trainer/rider part of `shared/src/sim.ts`, #67).
//!
//! [`SimRig`] owns one seeded rider + trainer model ([`SimCore`]) and hands out a
//! [`SimTrainer`] (`TrainerDriver`) and a [`SimHeart`] (`HeartRateDriver`) over it. A
//! single task advances the model on the injected [`Clock`] in fixed 250 ms ticks and
//! stamps every event with its grid time (`start + k·250 ms`), so the output depends
//! only on the seed, the commands and the fault plan, never on scheduling.
//!
//! Observable behaviour follows the FE-C driver in `crank-ble`: a `Sample` and its
//! page-25 `Observed` per tick, `Delivered` per written component, a page-71 echo
//! every `T_poll` = 1 s while a controlled mode is active (SIM alternates wind and
//! track), `SetRider` on a live link is a protocol fault, and a lost link recovers
//! with `Connecting`, `Ready` and no control mode (the engine re-issues `SetMode`,
//! ADR-0002 §6). Writer tick allocation and keep-alives (#43) are not modelled.
//!
//! Known limits: derived metrics, the metric bus, coach hooks and full `RideSim`
//! parity (`simulator.json` beyond the raw power, cadence, speed, HR and RR streams)
//! are covered by #203. Trainer power in ERG is independent of cadence except in the
//! injected [`SimFault::ErgSpiral`]; non-ERG modes use the rider's
//! [`SimRider::free_power_w`] (`Resistance` scales it by `0.5 + pct/100`).
mod core;
mod rng;
pub use self::core::{DT, Drive, HrReading, SimCore, SimRider, Tick};
pub use self::rng::Rng;

use crate::{
    Clock, DeviceError, DeviceInfo, DeviceRef, DriverKind, HeartRateDriver, HeartRateLink, HrEvent,
    HrSample, LinkState, MonoTime, ObservationSource, ObservedControl, ProtocolErrorKind,
    RiderConfig, TargetLimit, TrainerCaps, TrainerCommand, TrainerDriver, TrainerEvent,
    TrainerLink, TrainerObservation, TrainerSample, TrainerStatus,
};
use futures_util::future::BoxFuture;
use smallvec::SmallVec;
use std::{
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};
use tokio::sync::mpsc;

const TICK_US: u64 = 250_000;
const POLL_TICKS: u64 = 4;
/// Cadence below which an ERG target spirals (ADR-0002 §7.4 release threshold), rpm.
pub const SPIRAL_ONSET_RPM: f64 = 40.0;
const DIP: Duration = Duration::from_secs(4);

/// One control component the trainer echoes on page 71 (ADR-0002 §4.4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimComponent {
    /// Page 49.
    Erg,
    /// Page 48.
    Resistance,
    /// Page 50.
    Wind,
    /// Page 51.
    Track,
}
impl SimComponent {
    fn of(c: ObservedControl) -> Option<Self> {
        Some(match c {
            ObservedControl::Erg { .. } => Self::Erg,
            ObservedControl::Resistance { .. } => Self::Resistance,
            ObservedControl::Wind { .. } => Self::Wind,
            ObservedControl::Track { .. } => Self::Track,
            ObservedControl::NotControl(_) => return None,
        })
    }
}

/// How a component's page-71 echo misbehaves inside a fault window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoFault {
    /// The echo reports the value applied this long ago.
    Lag(Duration),
    /// The echo repeats the value applied when the window opened.
    Stuck,
    /// No echo is produced for the component.
    Silent,
    /// Control writes for the component are not applied; the echo reports the old value.
    DropWrites,
}

/// Scheduled test fault; times are measured from the rig's first connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SimFault {
    /// Trainer link lost at `at` for `down`; recovers with no control mode.
    Dropout {
        /// Start.
        at: Duration,
        /// Outage length.
        down: Duration,
    },
    /// Component echo fault in `[from, until)`.
    Echo {
        /// Affected component.
        component: SimComponent,
        /// Behaviour.
        fault: EchoFault,
        /// Window start.
        from: Duration,
        /// Window end.
        until: Duration,
    },
    /// Rider stops pedalling in `[from, until)`: cadence and power go to zero.
    CadenceStall {
        /// Window start.
        from: Duration,
        /// Window end.
        until: Duration,
    },
    /// Cadence is forced to `dip_rpm` for four seconds at `from`. In ERG with a positive
    /// target, a dip below [`SPIRAL_ONSET_RPM`] spirals (constant torque, falling
    /// cadence) until the mode leaves ERG (ADR-0002 §7).
    ErgSpiral {
        /// Dip start.
        from: Duration,
        /// Cadence during the dip.
        dip_rpm: f64,
    },
    /// Heart-rate strap link lost at `at` for `down`.
    HrLoss {
        /// Start.
        at: Duration,
        /// Outage length.
        down: Duration,
    },
}

/// Simulator configuration.
#[derive(Clone, Debug)]
pub struct SimConfig {
    /// Rider physiology.
    pub rider: SimRider,
    /// Mass used until a trainer connection supplies [`RiderConfig`].
    pub default_rider: RiderConfig,
    /// RNG seed.
    pub seed: u32,
    /// Capabilities advertised on connect.
    pub caps: TrainerCaps,
    /// Faults to inject.
    pub faults: Vec<SimFault>,
}
impl Default for SimConfig {
    fn default() -> Self {
        Self {
            rider: SimRider::default(),
            default_rider: RiderConfig {
                rider_kg: 72.0,
                bike_kg: 9.0,
                wheel_m: 2.096,
            },
            seed: 1,
            caps: TrainerCaps {
                erg: true,
                sim: true,
                resistance: true,
                max_resistance_n: Some(1000),
                max_target_w: 16383,
            },
            faults: Vec::new(),
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "FE-C reports f32; narrowing is the wire format"
)]
fn to_f32(x: f64) -> f32 {
    x as f32
}
fn us(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct TrainerSide {
    /// Link generation; commands and closes from an older link are ignored.
    generation: u64,
    tx: Option<mpsc::Sender<TrainerEvent>>,
    history: [Vec<(u64, ObservedControl)>; 4],
    active: Option<SimComponent>,
    poll_next: Option<u64>,
    polls: u64,
    pending: Vec<TrainerCommand>,
    was_down: bool,
    acc_power: u32,
    dip_until: Option<u64>,
    spiral_basis: Option<f64>,
    spiral_done: bool,
}
#[derive(Default)]
struct HeartSide {
    generation: u64,
    tx: Option<mpsc::Sender<HrEvent>>,
    rr: SmallVec<[u16; 4]>,
    was_down: bool,
}
struct World {
    cfg: SimConfig,
    core: SimCore,
    start: Option<MonoTime>,
    ticks: u64,
    running: bool,
    next_generation: u64,
    trainer: TrainerSide,
    heart: HeartSide,
}
#[derive(Default)]
struct Out {
    trainer: Vec<TrainerEvent>,
    heart: Vec<HrEvent>,
}

impl World {
    fn new(cfg: SimConfig) -> Self {
        let d = cfg.default_rider;
        Self {
            core: SimCore::new(
                cfg.rider,
                f64::from(d.rider_kg) + f64::from(d.bike_kg),
                cfg.seed,
            ),
            cfg,
            start: None,
            ticks: 0,
            running: false,
            next_generation: 0,
            trainer: TrainerSide::default(),
            heart: HeartSide::default(),
        }
    }
    fn window(&self, rel: u64, pick: impl Fn(&SimFault) -> Option<(u64, u64)>) -> bool {
        self.cfg
            .faults
            .iter()
            .filter_map(pick)
            .any(|(from, until)| (from..until).contains(&rel))
    }
    fn trainer_down(&self, rel: u64) -> bool {
        self.window(rel, |f| match *f {
            SimFault::Dropout { at, down } => Some((us(at), us(at + down))),
            _ => None,
        })
    }
    fn hr_down(&self, rel: u64) -> bool {
        self.window(rel, |f| match *f {
            SimFault::HrLoss { at, down } => Some((us(at), us(at + down))),
            _ => None,
        })
    }
    fn echo_fault(&self, c: SimComponent, rel: u64) -> Option<(EchoFault, u64)> {
        self.cfg.faults.iter().find_map(|f| match *f {
            SimFault::Echo {
                component,
                fault,
                from,
                until,
            } if component == c && (us(from)..us(until)).contains(&rel) => Some((fault, us(from))),
            _ => None,
        })
    }
    fn latest(&self, c: SimComponent) -> Option<ObservedControl> {
        self.trainer.history[c as usize].last().map(|(_, v)| *v)
    }
    fn echo_value(&self, c: SimComponent, rel: u64) -> Option<ObservedControl> {
        let at_or_before = |t: u64| {
            self.trainer.history[c as usize]
                .iter()
                .rev()
                .find(|(ts, _)| *ts <= t)
                .map(|(_, v)| *v)
        };
        match self.echo_fault(c, rel) {
            None | Some((EchoFault::DropWrites, _)) => self.latest(c),
            Some((EchoFault::Lag(d), _)) => rel.checked_sub(us(d)).and_then(at_or_before),
            Some((EchoFault::Stuck, from)) => at_or_before(from),
            Some((EchoFault::Silent, _)) => None,
        }
    }
    fn apply(&mut self, cmd: TrainerCommand, rel: u64, out: &mut Out) {
        match cmd {
            TrainerCommand::SetMode(mode) => match mode.grid_key() {
                Err(e) => out.trainer.push(TrainerEvent::Fault(e)),
                Ok(key) => {
                    for control in key {
                        let Some(c) = SimComponent::of(control) else {
                            continue;
                        };
                        let dropped =
                            matches!(self.echo_fault(c, rel), Some((EchoFault::DropWrites, _)));
                        if !dropped {
                            self.trainer.history[c as usize].push((rel, control));
                            self.trainer.active = Some(c);
                        }
                        out.trainer.push(TrainerEvent::Delivered { mode });
                    }
                    self.trainer
                        .poll_next
                        .get_or_insert(rel / TICK_US + POLL_TICKS);
                }
            },
            TrainerCommand::SetRider(_) => {
                out.trainer.push(TrainerEvent::Fault(DeviceError::Protocol {
                    kind: ProtocolErrorKind::UnexpectedPage,
                    detail: "rider changes require a new connection (#43)".into(),
                }));
            }
            TrainerCommand::Disconnect => {
                out.trainer.push(TrainerEvent::Link(LinkState::Lost));
                self.trainer = TrainerSide::default();
            }
        }
    }
    /// Physical drive derived from what the trainer has applied (not from echoes).
    fn drive(&mut self, rel: u64) -> Drive {
        let rider = self.cfg.rider;
        let (mut target_w, mut grade_pct) = (rider.free_power_w, 0.0);
        match self.trainer.active {
            Some(SimComponent::Erg) => {
                if let Some(ObservedControl::Erg { quarter_w }) = self.latest(SimComponent::Erg) {
                    target_w = f64::from(quarter_w) / 4.0;
                }
            }
            Some(SimComponent::Resistance) => {
                if let Some(ObservedControl::Resistance { half_pct }) =
                    self.latest(SimComponent::Resistance)
                {
                    target_w = rider.free_power_w * (0.5 + f64::from(half_pct) / 200.0);
                }
            }
            Some(SimComponent::Wind | SimComponent::Track) => {
                if let Some(ObservedControl::Track {
                    grade_centi_pct, ..
                }) = self.latest(SimComponent::Track)
                {
                    grade_pct = f64::from(grade_centi_pct) / 100.0;
                }
            }
            None => {}
        }
        let erg = self.trainer.active == Some(SimComponent::Erg) && target_w > 0.0;
        let mut cadence_force = None;
        let stalled = self.window(rel, |f| match *f {
            SimFault::CadenceStall { from, until } => Some((us(from), us(until))),
            _ => None,
        });
        if stalled {
            target_w = 0.0;
            cadence_force = Some(0.0);
        }
        // injected dip, then spiral if the ERG torque cannot be turned over
        let dip = self.cfg.faults.iter().find_map(|f| match *f {
            SimFault::ErgSpiral { from, dip_rpm } if rel >= us(from) => Some((us(from), dip_rpm)),
            _ => None,
        });
        if let Some((from, dip_rpm)) = dip {
            let until = *self.trainer.dip_until.get_or_insert(from + us(DIP));
            if rel < until {
                cadence_force = Some(dip_rpm);
            } else if !erg {
                self.trainer.spiral_basis = None;
            } else if self.trainer.spiral_basis.is_none()
                && !self.trainer.spiral_done
                && self.core.cadence() < SPIRAL_ONSET_RPM
            {
                self.trainer.spiral_basis = Some(self.core.cadence().max(1.0));
            }
            if let Some(basis) = self.trainer.spiral_basis {
                if erg {
                    cadence_force = Some(0.0);
                    target_w *= (self.core.cadence() / basis).min(1.0);
                } else {
                    self.trainer.spiral_basis = None;
                    self.trainer.spiral_done = true;
                }
            }
        }
        Drive {
            target_w,
            grade_pct,
            cadence_pref: None,
            cadence_force,
        }
    }
    fn tick(&mut self, k: u64, start: MonoTime, out: &mut Out) {
        let (rel, at) = (k * TICK_US, MonoTime(start.0 + k * TICK_US));
        let down = self.trainer_down(rel);
        if self.trainer.tx.is_some() {
            if down && !self.trainer.was_down {
                out.trainer.push(TrainerEvent::Link(LinkState::Lost));
            }
            if !down && self.trainer.was_down {
                // the trainer fell back to standalone: the engine must re-issue SetMode
                let (tx, acc) = (self.trainer.tx.take(), self.trainer.acc_power);
                self.trainer = TrainerSide {
                    generation: self.trainer.generation,
                    tx,
                    acc_power: acc,
                    ..TrainerSide::default()
                };
                out.trainer.push(TrainerEvent::Link(LinkState::Connecting));
                out.trainer.push(TrainerEvent::Link(LinkState::Ready));
            }
            self.trainer.was_down = down;
            for cmd in std::mem::take(&mut self.trainer.pending) {
                if !down || cmd == TrainerCommand::Disconnect {
                    self.apply(cmd, rel, out);
                }
            }
        }
        let drive = self.drive(rel);
        let tick = self.core.step(drive);
        let target_limit = match (self.trainer.active, drive.cadence_force) {
            (Some(SimComponent::Erg), Some(_)) => TargetLimit::SpeedTooLow,
            (Some(SimComponent::Erg), None) => TargetLimit::AtTarget,
            _ => TargetLimit::Undetermined,
        };
        if self.trainer.tx.is_some() && !down {
            self.emit_trainer(k, at, &tick, target_limit, out);
        }
        self.emit_heart(rel, at, &tick, out);
    }
    fn emit_trainer(
        &mut self,
        k: u64,
        at: MonoTime,
        tick: &Tick,
        target_limit: TargetLimit,
        out: &mut Out,
    ) {
        let rel = k * TICK_US;
        if let (Some(p), Some(c), Some(s)) = (tick.power_w, tick.cadence_rpm, tick.speed_kmh) {
            let power = core::saturate_u16(p);
            self.trainer.acc_power = self.trainer.acc_power.wrapping_add(u32::from(power)) & 0xFFFF;
            out.trainer.push(TrainerEvent::Sample(TrainerSample {
                at,
                power_w: Some(power),
                cadence_rpm: Some(u8::try_from(core::saturate_u16(c).min(254)).unwrap_or(u8::MAX)),
                speed_mps: Some(to_f32(s / 3.6)),
                distance_m: Some(to_f32((tick.distance_km * 1000.0) % 256.0)),
                accumulated_power_w: Some(self.trainer.acc_power),
                event_count: Some(k.to_le_bytes()[0]),
                status: TrainerStatus::default(),
                target_limit,
            }));
            out.trainer.push(TrainerEvent::Observed(TrainerObservation {
                at,
                source: ObservationSource::Page25Flags,
                control: None,
                status: None,
                target_limit: Some(target_limit),
            }));
        }
        if self.trainer.poll_next.is_some_and(|due| k >= due) {
            self.trainer.poll_next = Some(k + POLL_TICKS);
            let order = match self.trainer.active {
                Some(SimComponent::Wind | SimComponent::Track) => {
                    [SimComponent::Wind, SimComponent::Track].to_vec()
                }
                Some(c) => vec![c],
                None => vec![],
            };
            if !order.is_empty() {
                let c = order[usize::try_from(self.trainer.polls).unwrap_or(0) % order.len()];
                self.trainer.polls += 1;
                if let Some(control) = self.echo_value(c, rel) {
                    out.trainer.push(TrainerEvent::Observed(TrainerObservation {
                        at,
                        source: ObservationSource::Page71Echo,
                        control: Some(control),
                        status: Some(crate::CommandStatus::Pass),
                        target_limit: None,
                    }));
                }
            }
        }
    }
    fn emit_heart(&mut self, rel: u64, at: MonoTime, tick: &Tick, out: &mut Out) {
        if self.heart.tx.is_some() {
            let hr_down = self.hr_down(rel);
            if hr_down && !self.heart.was_down {
                out.heart.push(HrEvent::Link(LinkState::Lost));
                self.heart.rr.clear();
            }
            if !hr_down && self.heart.was_down {
                out.heart.push(HrEvent::Link(LinkState::Connecting));
                out.heart.push(HrEvent::Link(LinkState::Ready));
            }
            self.heart.was_down = hr_down;
            if !hr_down {
                self.heart.rr.extend(tick.beats.iter().map(|b| b.1));
                if let Some(HrReading { bpm }) = tick.hr {
                    out.heart.push(HrEvent::Sample(HrSample {
                        at,
                        bpm,
                        rr_ms: std::mem::take(&mut self.heart.rr),
                        contact: Some(bpm.is_some()),
                    }));
                }
            }
        }
    }
}

/// One seeded rider shared by a simulated trainer and a simulated strap.
#[derive(Clone)]
pub struct SimRig {
    world: Arc<Mutex<World>>,
    clock: Arc<dyn Clock>,
}
impl SimRig {
    /// Build a rig; nothing runs until a driver connects.
    #[must_use]
    pub fn new(cfg: SimConfig, clock: Arc<dyn Clock>) -> Self {
        Self {
            world: Arc::new(Mutex::new(World::new(cfg))),
            clock,
        }
    }
    /// Trainer driver over this rig.
    #[must_use]
    pub fn trainer(&self) -> SimTrainer {
        SimTrainer(self.clone())
    }
    /// Heart-rate driver over this rig.
    #[must_use]
    pub fn heart(&self) -> SimHeart {
        SimHeart(self.clone())
    }
    /// Register a side and make sure the stepping task runs. Returns the epoch.
    fn attach(
        &self,
        f: impl FnOnce(&mut World) -> Result<(), DeviceError>,
    ) -> Result<(), DeviceError> {
        let spawn = {
            let mut w = lock(&self.world);
            f(&mut w)?;
            if w.start.is_none() {
                w.start = Some(self.clock.now());
            }
            !std::mem::replace(&mut w.running, true)
        };
        if spawn {
            tokio::spawn(run(self.world.clone(), self.clock.clone()));
        }
        Ok(())
    }
}

async fn run(world: Arc<Mutex<World>>, clock: Arc<dyn Clock>) {
    loop {
        let wake = {
            let w = lock(&world);
            let start = w.start.map_or(0, |s| s.0);
            MonoTime(start + (w.ticks + 1) * TICK_US)
        };
        clock.sleep_until(wake).await;
        let now = clock.now();
        let (out, trainer_tx, heart_tx, generations) = {
            let mut w = lock(&world);
            // senders captured first: a Disconnect inside the batch still delivers `Lost`
            let (trainer_tx, heart_tx) = (w.trainer.tx.clone(), w.heart.tx.clone());
            let generations = (w.trainer.generation, w.heart.generation);
            let start = w.start.unwrap_or_default();
            let mut out = Out::default();
            while start.0 + (w.ticks + 1) * TICK_US <= now.0 {
                w.ticks += 1;
                let k = w.ticks;
                w.tick(k, start, &mut out);
            }
            (out, trainer_tx, heart_tx, generations)
        };
        let mut closed = (trainer_tx.is_none(), heart_tx.is_none());
        if let Some(tx) = trainer_tx {
            for e in out.trainer {
                if tx.send(e).await.is_err() {
                    closed.0 = true;
                    break;
                }
            }
        }
        if let Some(tx) = heart_tx {
            for e in out.heart {
                if tx.send(e).await.is_err() {
                    closed.1 = true;
                    break;
                }
            }
        }
        let mut w = lock(&world);
        // a receiver dropped by an older link must not tear down its successor
        if closed.0 && w.trainer.generation == generations.0 {
            w.trainer = TrainerSide::default();
        }
        if closed.1 && w.heart.generation == generations.1 {
            w.heart = HeartSide::default();
        }
        if w.trainer.tx.is_none() && w.heart.tx.is_none() {
            w.running = false;
            return;
        }
    }
}

fn info(model: &str) -> DeviceInfo {
    DeviceInfo {
        name: Some(format!("crank sim {model}")),
        model: Some(model.into()),
        serial: Some("0".into()),
        firmware: Some(env!("CARGO_PKG_VERSION").into()),
    }
}

/// Simulated FE-C-like trainer. One live link at a time.
pub struct SimTrainer(SimRig);
impl TrainerDriver for SimTrainer {
    fn kind(&self) -> DriverKind {
        DriverKind::Sim
    }
    fn connect(
        &self,
        _dev: &DeviceRef,
        rider: RiderConfig,
    ) -> BoxFuture<'static, Result<TrainerLink, DeviceError>> {
        let rig = self.0.clone();
        Box::pin(async move {
            let (events, receiver) = mpsc::channel(256);
            let (commands, mut control) = mpsc::channel(64);
            let caps = lock(&rig.world).cfg.caps.clone();
            let mut generation = 0;
            for s in [LinkState::Connecting, LinkState::Ready] {
                events
                    .try_send(TrainerEvent::Link(s))
                    .map_err(|e| DeviceError::Gatt(e.to_string()))?;
            }
            rig.attach(|w| {
                if w.trainer.tx.as_ref().is_some_and(|tx| !tx.is_closed()) {
                    return Err(DeviceError::ConnectFailed(
                        "driver already connected".into(),
                    ));
                }
                w.core
                    .set_mass_kg(f64::from(rider.rider_kg) + f64::from(rider.bike_kg));
                w.next_generation += 1;
                generation = w.next_generation;
                w.trainer = TrainerSide {
                    generation,
                    tx: Some(events),
                    ..TrainerSide::default()
                };
                Ok(())
            })?;
            let world = rig.world.clone();
            tokio::spawn(async move {
                // Only this link's generation may feed the trainer: after a newer
                // connect, a late command or the drop of this sender is stale.
                let push = |cmd| {
                    let mut w = lock(&world);
                    if w.trainer.generation == generation {
                        w.trainer.pending.push(cmd);
                    }
                };
                while let Some(cmd) = control.recv().await {
                    push(cmd);
                }
                push(TrainerCommand::Disconnect);
            });
            Ok(TrainerLink {
                info: info("trainer"),
                caps,
                control: commands,
                events: receiver,
            })
        })
    }
}

/// Simulated heart-rate strap. One live link at a time.
pub struct SimHeart(SimRig);
impl HeartRateDriver for SimHeart {
    fn connect(&self, _dev: &DeviceRef) -> BoxFuture<'static, Result<HeartRateLink, DeviceError>> {
        let rig = self.0.clone();
        Box::pin(async move {
            let (events, receiver) = mpsc::channel(256);
            for s in [LinkState::Connecting, LinkState::Ready] {
                events
                    .try_send(HrEvent::Link(s))
                    .map_err(|e| DeviceError::Gatt(e.to_string()))?;
            }
            rig.attach(|w| {
                if w.heart.tx.as_ref().is_some_and(|tx| !tx.is_closed()) {
                    return Err(DeviceError::ConnectFailed(
                        "driver already connected".into(),
                    ));
                }
                w.next_generation += 1;
                w.heart = HeartSide {
                    generation: w.next_generation,
                    tx: Some(events),
                    ..HeartSide::default()
                };
                Ok(())
            })?;
            Ok(HeartRateLink {
                info: info("heart"),
                events: receiver,
            })
        })
    }
}
