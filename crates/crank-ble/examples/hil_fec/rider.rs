//! The Part B control checks with a rider on the trainer (#44 checks 6, 7, 8, 9, 12; #412), and
//! check 10, which holds the link while the operator tries a second central.
//!
//! Safety (ADR-0002 §7): every run connects into page 48 at 0 %, the release state; ERG is
//! engaged as the engine does at a ride start (§7.5: 0 W until the first sample with cadence,
//! then a 5 s ramp in 1 Hz steps from the 3 s power); targets never exceed [`MAX_ERG_W`] and
//! grades [`MAX_GRADE_PCT`]; while ERG is applied the §7.4 release rule (cadence below
//! 40 rpm for 2 s) and a 2 s telemetry silence (§6) stop the procedure, also while a write is
//! pending, and a write not acknowledged within [`WRITE_WAIT`] fails it. Whatever ends it
//! (completion, a guard, an error, a panic, SIGINT/SIGTERM/SIGHUP) the trainer is
//! released with page 48 at 0 %, the ride-end mode of §7.9, before the link is closed.
use crate::{
    frames::{Cps, Note, Record, page_of},
    partb::{Descent, STALL_FLOOR_RPM, Sample},
    session::Session,
};
use crank_engine::{
    control::SOFT_START_TICKS,
    erg::policy::{RELEASE_RPM, RELEASE_TICKS},
    ticks::TICKS_PER_SECOND,
};
use crank_fec::{ControlPage, encode};
use futures_util::{FutureExt, StreamExt, stream::BoxStream};
use serde_json::{Value, json};
use std::{future::Future, panic::AssertUnwindSafe, time::Duration};
use tokio::time::{Instant, sleep};

/// Highest ERG target any check may command (#44 check 6).
pub const MAX_ERG_W: u16 = 250;
/// Steepest SIM grade any check may command (#44 check 8).
pub const MAX_GRADE_PCT: f32 = 10.0;
const SEC: u64 = 1_000_000;
/// No page 25 for this long while riding stops the run (ADR-0002 §6: 8 ticks).
const SILENCE_US: u64 = 2 * SEC;
/// The rider must start pedalling within this time.
const START_WAIT: Duration = Duration::from_mins(2);
/// Bound on the release write: a stalled GATT write must not hold the exit (ADR-0002 §7.9's 2 s).
const RELEASE_WAIT: Duration = Duration::from_secs(2);
/// Bound on a procedure write: unacknowledged this long, the link counts as lost (ADR-0002 §6).
pub const WRITE_WAIT: Duration = Duration::from_secs(2);
/// Check 7's target: the page-49 write the silent window starts from.
pub const CHECK7_W: u16 = 150;
const HOLD: Duration = Duration::from_secs(20);
/// Check 5's two ERG targets, W.
pub const CHECK5_W: [u16; 2] = [150, 200];
/// How long check 5 holds each target.
const CHECK5_HOLD: Duration = Duration::from_secs(30);
/// How long into check 5's ride a Cycling Power notification may take to arrive.
const CPS_WAIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);

/// What a procedure drives: the live [`Session`] or the test simulator.
pub trait Wire {
    /// Microseconds since the capture started (the `.fecap` time base).
    fn now_us(&self) -> u64;
    /// Record and write one frame (acknowledged, with response, as the driver does).
    fn send(&self, frame: [u8; 13]) -> impl Future<Output = Option<String>>;
    /// Number of rx frames so far (a `since` mark).
    fn mark(&self) -> usize;
    /// rx frames since `mark`, on the capture time base.
    fn since(&self, mark: usize) -> Vec<Record>;
    /// The link or the capture has failed.
    fn dropped(&self) -> bool;
    /// Every record so far, both directions.
    fn records(&self) -> Vec<Record>;
    /// Every Cycling Power notification so far (check 5), on the capture time base.
    fn cps(&self) -> Vec<Cps>;
    /// Append a tagged comment line to the capture (`Capture::record_aux`); its error, if any.
    fn note(&self, tag: &str, bytes: &[u8]) -> Option<String>;
    /// A scan running beside the connection: one item per advertisement, `true` for this
    /// trainer's; the stream ends when the scan stops by itself, dropping it stops the scan.
    fn adverts(&self) -> BoxStream<'static, bool>;
    /// After a drop: connect the trainer again, discover and subscribe, recording the adapter's
    /// connect event as `# link <us> 03` ([`crate::check11::CONNECTED`]); the error, if any.
    /// Unbounded: the caller bounds it.
    fn reconnect(&self) -> impl Future<Output = Option<String>>;
}

impl Wire for Session {
    fn now_us(&self) -> u64 {
        Session::elapsed_us(self)
    }
    async fn send(&self, frame: [u8; 13]) -> Option<String> {
        Session::send(self, frame, true).await.1
    }
    fn mark(&self) -> usize {
        Session::mark(self)
    }
    fn since(&self, mark: usize) -> Vec<Record> {
        Session::since(self, mark)
            .into_iter()
            .map(|r| Record {
                at_us: self.capture_us(r.at),
                rx: true,
                bytes: r.bytes,
            })
            .collect()
    }
    fn dropped(&self) -> bool {
        self.pump_ended()
    }
    fn records(&self) -> Vec<Record> {
        std::fs::read_to_string(&self.capture_path)
            .map(|t| crate::frames::parse_capture(&t))
            .unwrap_or_default()
    }
    fn cps(&self) -> Vec<Cps> {
        std::fs::read_to_string(&self.capture_path)
            .map(|t| crate::frames::parse_cps(&t))
            .unwrap_or_default()
    }
    fn note(&self, tag: &str, bytes: &[u8]) -> Option<String> {
        self.capture
            .record_aux(crank_devices::Clock::now(&*self.clock), tag, bytes)
            .err()
            .map(|e| e.to_string())
    }
    fn adverts(&self) -> BoxStream<'static, bool> {
        Session::adverts(self)
    }
    async fn reconnect(&self) -> Option<String> {
        Session::reconnect(self).await.err().map(|e| e.to_string())
    }
}

/// An instruction to the rider; the agent relays it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// Start pedalling at about 90 rpm.
    Start,
    /// Hold about this cadence.
    Cadence(u8),
    /// Check 9: lower the cadence step by step until it stalls.
    Descend,
    /// SIM: steady, comfortable effort.
    Steady,
    /// Check 10: stay off the pedals and try the phone as a second central.
    Phone,
    /// Check 11b: stop pedalling until the unplugged trainer's lights go out.
    LightsOut,
    /// Check 11b: the link dropped; pedal to wake the trainer.
    Wake,
    /// The run is over and the trainer released.
    Stop,
}

impl Cue {
    /// Text shown to the operator.
    pub fn text(self) -> String {
        match self {
            Self::Start => "start pedalling at about 90 rpm".into(),
            Self::Cadence(rpm) => format!("hold about {rpm} rpm, steady"),
            Self::Descend => format!(
                "lower cadence smoothly, about 5 rpm every 10 s, and keep turning: the run ends when the trainer lets go or at {STALL_FLOOR_RPM} rpm; stopping early is safe (released) but gives no result"
            ),
            Self::Steady => {
                "easy gear, steady comfortable effort; shift down when it gets steep, never strain"
                    .into()
            }
            Self::Phone => "stay off the pedals; on your phone open the Tacx app, look for the NEO and try to connect to it, then answer the questions here".into(),
            Self::LightsOut => "trainer released: stop pedalling and wait until its lights go out (it must be unplugged)".into(),
            Self::Wake => "the link dropped: pedal again, easy, to wake the trainer; the harness reconnects (60 s at most) and asks before it applies anything".into(),
            Self::Stop => "done: trainer released (0 % resistance), stop pedalling".into(),
        }
    }
}

/// The person at the trainer.
pub trait Operator {
    /// Show a cue (stamped with capture time).
    fn cue(&self, at_us: u64, cue: Cue);
    /// Ask to start; `false` (refused, no answer, closed input) runs nothing.
    fn confirm(&self, text: &str) -> impl Future<Output = bool>;
    /// Ask `question` until the answer is one of `options`; `None` when the input closes.
    fn ask(
        &self,
        question: &str,
        options: &'static [&'static str],
    ) -> impl Future<Output = Option<&'static str>>;
}

/// Prompts on stdout, confirmation on stdin.
pub struct Console(tokio::sync::Mutex<tokio::sync::mpsc::Receiver<String>>);

impl Console {
    /// Read stdin lines on a detached thread, so a pending read never holds runtime shutdown.
    pub fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        std::thread::spawn(move || {
            for line in std::io::stdin().lines() {
                let Ok(line) = line else { break };
                if tx.blocking_send(line).is_err() {
                    break;
                }
            }
        });
        Self::from_lines(rx)
    }

    /// Confirmation answers from `lines` (stdin, or a test).
    pub fn from_lines(lines: tokio::sync::mpsc::Receiver<String>) -> Self {
        Self(tokio::sync::Mutex::new(lines))
    }
}

/// Write a line; a closed terminal (SIGHUP) must not panic the release path.
fn say(text: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
}

impl Operator for Console {
    fn cue(&self, at_us: u64, cue: Cue) {
        say(&format!(
            "[{:6.1} s] CUE: {}",
            crate::partb::secs(at_us),
            cue.text()
        ));
    }
    async fn confirm(&self, text: &str) -> bool {
        say(&format!(
            "{text}\nType y and Enter to start; anything else aborts (Ctrl-C at any time releases the trainer and stops)."
        ));
        self.0
            .lock()
            .await
            .recv()
            .await
            .is_some_and(|l| matches!(l.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
    }
    async fn ask(&self, question: &str, options: &'static [&'static str]) -> Option<&'static str> {
        let choices = options.join(" / ");
        say(&format!(
            "QUESTION: {question}\nType one of {choices} and Enter."
        ));
        let mut lines = self.0.lock().await;
        loop {
            let line = lines.recv().await?;
            let typed = line.trim().to_ascii_lowercase();
            if let Some(o) = options.iter().find(|o| **o == typed) {
                return Some(o);
            }
            say(&format!(
                "{:?} is not an answer; type one of {choices}",
                line.trim()
            ));
        }
    }
}

/// Why a procedure stopped early.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// A safety guard or a precondition: the run ended deliberately.
    Aborted(String),
    /// A signal arrived.
    Interrupted(String),
    /// The link, the capture or the harness failed.
    Failed(String),
}

/// Which rule watches the rider while a hold runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guard {
    /// ERG applied: §7.4 release rule.
    Erg,
    /// Check 9's descent from `from_us` (capture time): [`crate::partb::descent`] ends the
    /// hold at a stall or the floor and aborts the run when the rider leaves the cue.
    Descent {
        /// When the cued descent starts.
        from_us: u64,
    },
    /// SIM: no torque target; silence and link only.
    Sim,
}

/// The run state shared by the procedures.
pub struct Rig<'a, W, O> {
    w: &'a W,
    op: &'a O,
    guard: Guard,
    seen: usize,
    last_sample_us: u64,
    low_since: Option<u64>,
    recent: Vec<Sample>,
}

fn frame(page: &ControlPage) -> Result<[u8; 13], Stop> {
    encode(page).map_err(|e| Stop::Failed(format!("encode {page:?}: {e}")))
}

impl<'a, W: Wire, O: Operator> Rig<'a, W, O> {
    fn new(w: &'a W, op: &'a O) -> Self {
        Self {
            w,
            op,
            guard: Guard::Sim,
            seen: w.mark(),
            last_sample_us: w.now_us(),
            low_since: None,
            recent: Vec::new(),
        }
    }

    fn cue(&self, cue: Cue) {
        self.op.cue(self.w.now_us(), cue);
    }

    /// Write one control page. The guards keep running while it is pending, and a write not
    /// acknowledged within [`WRITE_WAIT`] fails the run.
    async fn write(&mut self, page: &ControlPage) -> Result<(), Stop> {
        let w = self.w;
        let send = w.send(frame(page)?);
        tokio::pin!(send);
        let deadline = Instant::now() + WRITE_WAIT;
        loop {
            tokio::select! {
                e = &mut send => {
                    return e.map_or(Ok(()), |e| Err(Stop::Failed(format!("write {page:?}: {e}"))));
                }
                () = sleep(POLL) => {
                    self.observe()?;
                    if Instant::now() >= deadline {
                        return Err(Stop::Failed(format!(
                            "write {page:?} not completed within 2 s"
                        )));
                    }
                }
            }
        }
    }

    async fn erg(&mut self, watts: u16) -> Result<(), Stop> {
        if watts > MAX_ERG_W {
            return Err(Stop::Failed(format!(
                "{watts} W is above the {MAX_ERG_W} W limit"
            )));
        }
        self.write(&ControlPage::TargetPower {
            watts: f32::from(watts),
        })
        .await
    }

    async fn track(&mut self, grade_pct: f32) -> Result<(), Stop> {
        if !grade_pct.is_finite() || grade_pct.abs() > MAX_GRADE_PCT {
            return Err(Stop::Failed(format!(
                "{grade_pct} % is outside ±{MAX_GRADE_PCT} %"
            )));
        }
        self.write(&ControlPage::Track {
            grade_pct,
            crr: 0.004,
        })
        .await
    }

    /// Take new page-25 samples and apply the guard; `Ok(true)` when check 9's descent ended.
    fn observe(&mut self) -> Result<bool, Stop> {
        if self.w.dropped() {
            return Err(Stop::Failed("link dropped".into()));
        }
        let fresh = self.w.since(self.seen);
        self.seen += fresh.len();
        for s in crate::partb::samples(&fresh) {
            self.last_sample_us = s.at_us;
            self.recent.push(s);
            let low = s.cadence.is_none_or(|c| c < RELEASE_RPM);
            self.low_since = if low {
                self.low_since.or(Some(s.at_us))
            } else {
                None
            };
            match self.guard {
                Guard::Erg => {
                    let since = self.low_since.map_or(0, |t| s.at_us - t);
                    if since >= RELEASE_TICKS * SEC / TICKS_PER_SECOND {
                        return Err(Stop::Aborted(format!(
                            "cadence below {RELEASE_RPM} rpm for 2 s under ERG (ADR-0002 §7.4)"
                        )));
                    }
                }
                Guard::Descent { .. } | Guard::Sim => {}
            }
        }
        if self.w.now_us().saturating_sub(self.last_sample_us) > SILENCE_US {
            return Err(Stop::Aborted(SILENT.into()));
        }
        let Guard::Descent { from_us } = self.guard else {
            return Ok(false);
        };
        match crate::partb::descent(&self.recent, from_us, f64::from(MAX_ERG_W)) {
            Descent::Going => Ok(false),
            Descent::Stalled { .. } | Descent::Floor { .. } => Ok(true),
            Descent::Departed { why, .. } => Err(Stop::Aborted(why)),
        }
    }

    /// Ride for `dur` under the guard; returns early (`true`) only when check 9's descent ended.
    async fn hold(&mut self, dur: Duration) -> Result<bool, Stop> {
        let end = Instant::now() + dur;
        while Instant::now() < end {
            if self.observe()? {
                return Ok(true);
            }
            sleep(POLL.min(end - Instant::now())).await;
        }
        self.observe()
    }

    /// Wait for the first sample with cadence > 0 (no guard: nothing is applied yet).
    async fn wait_pedalling(&mut self) -> Result<(), Stop> {
        self.cue(Cue::Start);
        let guard = std::mem::replace(&mut self.guard, Guard::Sim);
        let end = Instant::now() + START_WAIT;
        let mut ok = false;
        while !ok && Instant::now() < end {
            self.observe()?;
            ok = self
                .recent
                .last()
                .is_some_and(|s| s.cadence.is_some_and(|c| c > 0));
            sleep(POLL).await;
        }
        self.guard = guard;
        if ok {
            Ok(())
        } else {
            Err(Stop::Aborted(
                "the rider did not start pedalling within 120 s".into(),
            ))
        }
    }

    /// Engage ERG at `target` as the engine does at a ride start (ADR-0002 §7.5).
    async fn engage(&mut self, target: u16) -> Result<(), Stop> {
        self.guard = Guard::Sim;
        self.erg(0).await?;
        self.wait_pedalling().await?;
        let now = self.w.now_us();
        let window: Vec<f64> = self
            .recent
            .iter()
            .filter(|s| s.at_us + 3 * SEC >= now)
            .filter_map(|s| s.power.map(f64::from))
            .collect();
        #[allow(clippy::cast_precision_loss)]
        let origin = if window.is_empty() {
            0.0
        } else {
            window.iter().sum::<f64>() / window.len() as f64
        };
        self.guard = Guard::Erg;
        self.low_since = None;
        let steps = SOFT_START_TICKS / TICKS_PER_SECOND;
        for k in 0..=steps {
            #[allow(clippy::cast_precision_loss)]
            let w = origin + (f64::from(target) - origin) * (k as f64 / steps as f64);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            self.erg(w.round().clamp(0.0, f64::from(MAX_ERG_W)) as u16)
                .await?;
            if k < steps {
                self.hold(Duration::from_secs(1)).await?;
            }
        }
        Ok(())
    }

    /// Page-70 request for page 71, to leave the command status in the capture.
    async fn request71(&mut self) -> Result<(), Stop> {
        sleep(Duration::from_millis(300)).await;
        self.write(&ControlPage::Request { page: 71, times: 1 })
            .await?;
        sleep(Duration::from_millis(300)).await;
        Ok(())
    }

    fn note(&self, tag: &str, bytes: &[u8]) -> Result<(), Stop> {
        self.w
            .note(tag, bytes)
            .map_or(Ok(()), |e| Err(Stop::Failed(format!("capture: {e}"))))
    }

    /// Await `fut` until `until`, recording `scan`'s advertisements and running the guard every
    /// [`POLL`] meanwhile (on a fixed tick, so a busy scan cannot starve it); `None` at `until`.
    async fn during<T>(
        &mut self,
        scan: &mut Option<BoxStream<'static, bool>>,
        until: Instant,
        fut: impl Future<Output = T>,
    ) -> Result<Option<T>, Stop> {
        use crate::check10::{ADV, ENDED, SCAN};
        tokio::pin!(fut);
        let mut tick = tokio::time::interval(POLL);
        loop {
            let next = async {
                match scan.as_mut() {
                    Some(s) => s.next().await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                v = &mut fut => return Ok(Some(v)),
                ad = next => {
                    if let Some(mine) = ad {
                        self.note(ADV, &[u8::from(mine)])?;
                    } else {
                        *scan = None;
                        self.note(SCAN, &[ENDED])?;
                    }
                }
                _ = tick.tick() => {
                    self.observe()?;
                    if Instant::now() >= until {
                        return Ok(None);
                    }
                }
            }
        }
    }

    /// Ask `q`, running the guards and the scan (if any) meanwhile, and record the answer; it must
    /// come by `until`.
    async fn answer(
        &mut self,
        scan: &mut Option<BoxStream<'static, bool>>,
        until: Instant,
        q: &crate::check10::Question,
    ) -> Result<&'static str, Stop> {
        let op = self.op;
        let text = q.text;
        let answer = self
            .during(scan, until, op.ask(text, q.options))
            .await?
            .ok_or_else(|| {
                Stop::Aborted(format!(
                    "no answer to {text:?} within {} s",
                    crate::check10::ANSWER_WAIT.as_secs()
                ))
            })?
            .ok_or_else(|| {
                Stop::Aborted(format!("the input closed before an answer to {text:?}"))
            })?;
        self.note(&crate::check10::answer_tag(q.key), answer.as_bytes())?;
        Ok(answer)
    }
}

/// #44 check 5: a ride at two ERG targets, 150 W then 200 W at 90 rpm, 30 s each, while the
/// capture records page 25 and the trainer's Cycling Power notifications side by side. Settled
/// windows (the target held ≥ 10 s, after its first 10 s) are compared by [`crate::cps`].
///
/// The ride is useless without CPS, so it ends (released) when no notification has arrived
/// 5 s after the rider is up to power.
async fn check5<W: Wire, O: Operator>(r: &mut Rig<'_, W, O>) -> Result<(), Stop> {
    r.engage(CHECK5_W[0]).await?;
    r.cue(Cue::Cadence(90));
    r.hold(CPS_WAIT).await?;
    if r.w.cps().is_empty() {
        return Err(Stop::Aborted(
            "no Cycling Power (2A63) notification 5 s after the rider is up to power".into(),
        ));
    }
    r.hold(CHECK5_HOLD.saturating_sub(CPS_WAIT)).await?;
    r.erg(CHECK5_W[1]).await?;
    r.hold(CHECK5_HOLD).await?;
    Ok(())
}

/// #44 check 6: ERG steps 100→150→250→100 W at 90 rpm, then cadence 90→60→90 at 200 W.
async fn check6<W: Wire, O: Operator>(r: &mut Rig<'_, W, O>) -> Result<(), Stop> {
    r.engage(100).await?;
    r.cue(Cue::Cadence(90));
    r.hold(HOLD).await?;
    for w in [150, 250, 100, 200] {
        r.erg(w).await?;
        r.hold(HOLD).await?;
    }
    for rpm in [60, 90] {
        r.cue(Cue::Cadence(rpm));
        r.hold(HOLD).await?;
    }
    Ok(())
}

/// #44 check 7: 150 W, then 30 s with no write at all (no keep-alive, no page-70 poll).
async fn check7<W: Wire, O: Operator>(r: &mut Rig<'_, W, O>) -> Result<(), Stop> {
    r.engage(CHECK7_W).await?;
    r.cue(Cue::Cadence(90));
    r.hold(Duration::from_secs(10)).await?;
    r.hold(Duration::from_secs(30)).await?;
    Ok(())
}

/// #44 check 8: SIM grades 0, 5 and 10 %, and page 55 mass 75 → 100 kg at 5 %.
async fn check8<W: Wire, O: Operator>(r: &mut Rig<'_, W, O>) -> Result<(), Stop> {
    let user = |rider_kg| ControlPage::UserConfig {
        rider_kg,
        bike_kg: 10.0,
        wheel_m: 0.708,
    };
    r.guard = Guard::Sim;
    r.wait_pedalling().await?;
    r.write(&user(75.0)).await?;
    r.write(&ControlPage::Wind {
        cw_kg_per_m: 0.51,
        speed_kmh: 0.0,
        drafting: 1.0,
    })
    .await?;
    r.track(0.0).await?;
    r.cue(Cue::Steady);
    r.hold(Duration::from_secs(30)).await?;
    r.track(5.0).await?;
    r.hold(Duration::from_secs(30)).await?;
    r.write(&user(100.0)).await?;
    r.hold(Duration::from_secs(30)).await?;
    r.write(&user(75.0)).await?;
    r.track(10.0).await?;
    r.hold(Duration::from_secs(30)).await?;
    Ok(())
}

/// #44 check 9: 250 W ERG while the rider lowers cadence until the trainer lets go.
///
/// The descent starts 10 s after the first 250 W write ([`crate::partb::descent_target_at`],
/// as `analyse` reads it), so 5 s after the soft start when it begins at 250 W and writes it
/// six times. It has no timer of its own: [`crate::partb::descent`] ends it at a stall or at
/// the floor, and aborts it once the cadence is 15 rpm above the cue, which falls from 90 rpm
/// at 0.5 rpm/s; so it ends within 160 s (the cue at 10 rpm, 15 under the floor).
async fn check9<W: Wire, O: Operator>(r: &mut Rig<'_, W, O>) -> Result<(), Stop> {
    r.engage(MAX_ERG_W).await?;
    let from_us = crate::partb::descent_target_at(&r.w.records()).unwrap_or_else(|| r.w.now_us())
        + crate::partb::DESCENT_AFTER_US;
    r.cue(Cue::Cadence(90));
    r.hold(Duration::from_micros(from_us.saturating_sub(r.w.now_us())))
        .await?;
    r.guard = Guard::Descent { from_us };
    r.cue(Cue::Descend);
    while !r.hold(Duration::from_secs(1)).await? {}
    Ok(())
}

/// #44 check 12: ERG 200 W, then `Free` (page 50 defaults, page 51 grade 0), then 70 rpm.
async fn check12<W: Wire, O: Operator>(r: &mut Rig<'_, W, O>) -> Result<(), Stop> {
    r.engage(200).await?;
    r.cue(Cue::Cadence(90));
    r.hold(Duration::from_secs(15)).await?;
    r.write(&ControlPage::Wind {
        cw_kg_per_m: 0.51,
        speed_kmh: 0.0,
        drafting: 1.0,
    })
    .await?;
    r.request71().await?;
    r.track(0.0).await?;
    r.request71().await?;
    r.hold(Duration::from_secs(10)).await?;
    r.cue(Cue::Cadence(70));
    r.hold(Duration::from_secs(15)).await?;
    Ok(())
}

/// The guard's abort when page 25 stops (ADR-0002 §6).
const SILENT: &str = "no page 25 for 2 s (ADR-0002 §6)";

/// A guard stop during check 10's phone window, naming the likely cause: the second central.
fn phone_window(e: Stop) -> Stop {
    match e {
        Stop::Failed(why) if why == "link dropped" => Stop::Failed(
            "link dropped while the phone was trying to connect (second central)".into(),
        ),
        Stop::Aborted(why) if why == SILENT => Stop::Aborted(
            "no page 25 for 2 s while the phone was trying to connect: the second central may have taken the trainer (ADR-0002 §5, §6)".into(),
        ),
        e => e,
    }
}

/// #44 check 10: hold the link at the release state, with no rider, while the operator tries to
/// connect the phone (a second central) and answers [`crate::check10::LISTED`], then its
/// [`crate::check10::follow_up`]; a scan beside the connection records whether the trainer still
/// advertises. The window lasts at least [`crate::check10::MIN_WINDOW`]; the answers must be in
/// within [`crate::check10::ANSWER_WAIT`].
async fn check10<W: Wire, O: Operator>(r: &mut Rig<'_, W, O>) -> Result<(), Stop> {
    use crate::check10::{ANSWER_WAIT, CLOSE, LISTED, MIN_WINDOW, OPEN, SCAN, WINDOW, follow_up};
    r.guard = Guard::Sim;
    // The scan opens before the window and stops after it, so its stamps cover the window.
    let mut scan = Some(r.w.adverts());
    r.note(SCAN, &[OPEN])?;
    let start = Instant::now();
    r.note(WINDOW, &[OPEN])?;
    r.cue(Cue::Phone);
    let listed = r
        .answer(&mut scan, start + ANSWER_WAIT, &LISTED)
        .await
        .map_err(phone_window)?;
    if let Some(q) = follow_up(listed) {
        r.answer(&mut scan, start + ANSWER_WAIT, q)
            .await
            .map_err(phone_window)?;
    }
    r.during(&mut scan, start + MIN_WINDOW, std::future::pending::<()>())
        .await
        .map_err(phone_window)?;
    r.note(WINDOW, &[CLOSE])?;
    if scan.take().is_some() {
        r.note(SCAN, &[CLOSE])?;
    }
    Ok(())
}

/// Check 11b after the drop: the wake cue, then the trainer back within
/// [`crate::check11::RECONNECT_WAIT`] (seen in the scan, reconnected, first page 25), each phase
/// stamped. `Some((drop, mark))` once back (capture time of the drop, the rx mark before the
/// reconnect); `None` when the bound ran out (stamped 06, the lights asked unguarded: the link is
/// down); a failed reconnect is stamped 07 and fails the run.
async fn back_after_drop<W: Wire, O: Operator>(
    r: &mut Rig<'_, W, O>,
) -> Result<Option<(u64, usize)>, Stop> {
    use crate::check10::ANSWER_WAIT;
    use crate::check11::{
        DETECTED, FAILED, FIRST_PAGE25, GAVE_UP, LIGHTS, LINK, READY, RECONNECT_WAIT, SEEN,
    };
    r.note(LINK, &[DETECTED])?;
    let detected = r.w.now_us();
    r.cue(Cue::Wake);
    let deadline = Instant::now() + RECONNECT_WAIT;
    let mark = r.w.mark();
    let w = r.w;
    let back = async {
        let mut scan = w.adverts();
        loop {
            match scan.next().await {
                Some(true) => break,
                Some(false) => {}
                None => {
                    return Err(Stop::Failed(
                        "the scan ended before the trainer advertised again".into(),
                    ));
                }
            }
        }
        drop(scan);
        r.note(LINK, &[SEEN])?;
        if let Some(e) = w.reconnect().await {
            return Err(Stop::Failed(format!("reconnect: {e}")));
        }
        r.note(LINK, &[READY])?;
        while !w.since(mark).iter().any(|f| page_of(&f.bytes) == Some(25)) {
            sleep(Duration::from_millis(10)).await;
        }
        r.note(LINK, &[FIRST_PAGE25])
    };
    let reconnected = tokio::time::timeout_at(deadline, back).await;
    if let Ok(Err(e)) = reconnected {
        r.note(LINK, &[FAILED])?;
        return Err(e);
    }
    if reconnected.is_err() {
        r.note(LINK, &[GAVE_UP])?;
        let answer = tokio::time::timeout(ANSWER_WAIT, r.op.ask(LIGHTS.text, LIGHTS.options)).await;
        if let Ok(Some(a)) = answer {
            r.note(&crate::check10::answer_tag(LIGHTS.key), a.as_bytes())?;
        }
        return Ok(None);
    }
    Ok(Some((detected, mark)))
}

/// #44 check 11b, the unplugged stop (#412 part c2): ERG at [`crate::check11::TARGET_W`] by soft
/// start, then the release and the cue to stop until the lights go out. Silence is expected now,
/// so only the link counts, bounded by [`crate::check11::DROP_WAIT`]. Once it drops, the rider
/// pedals to wake the trainer and the harness reconnects within
/// [`crate::check11::RECONNECT_WAIT`], stamping each phase. Control never resumes silently: the
/// first write after the reconnect is the release, then the operator answers
/// [`crate::check11::LIGHTS`] and must type `y` before ERG is engaged again by soft start.
///
/// After the release, the trainer's page-81 serial must match the one before the drop
/// ([`crate::check11::same_trainer`]); a different unit aborts the run (released).
///
/// Completed means the drop was observed and either the trainer came back and held the target
/// 20 s, or the reconnect bound ran out (a result: no reconnect). A failed reconnect (the scan
/// ended, the connect failed) fails the run, stamped `# link 07`.
async fn check11b<W: Wire, O: Operator>(r: &mut Rig<'_, W, O>) -> Result<(), Stop> {
    use crate::check10::ANSWER_WAIT;
    use crate::check11::{CONFIRMED, DROP_WAIT, LIGHTS, STOPPING, TARGET_W, WINDOW, same_trainer};
    r.engage(TARGET_W).await?;
    r.cue(Cue::Cadence(90));
    r.hold(HOLD).await?;
    // A stopping rider under ERG is what §7.4 releases: release first, then cue the stop.
    r.guard = Guard::Sim;
    r.write(&ControlPage::BasicResistance { pct: 0.0 }).await?;
    r.note(WINDOW, &[STOPPING])?;
    r.cue(Cue::LightsOut);
    let deadline = Instant::now() + DROP_WAIT;
    while !r.w.dropped() {
        if Instant::now() >= deadline {
            return Err(Stop::Aborted(format!(
                "the link did not drop within {} s of the stop cue (is the trainer unplugged?)",
                DROP_WAIT.as_secs()
            )));
        }
        sleep(POLL).await;
    }
    let Some((detected, mark)) = back_after_drop(r).await? else {
        return Ok(());
    };
    let w = r.w;
    // From here the guards watch the new link, from its first page 25 (which resets the silence).
    (r.seen, r.low_since) = (mark, None);
    r.recent.clear();
    let back = r.w.now_us();
    r.write(&ControlPage::BasicResistance { pct: 0.0 }).await?;
    // The same trainer? Its page-81 serial (a read-only request), against the one before the drop.
    let asked = r.w.mark();
    r.write(&ControlPage::Request { page: 81, times: 1 })
        .await?;
    let serial = async {
        while !w
            .since(asked)
            .iter()
            .any(|f| page_of(&f.bytes) == Some(0x51))
        {
            sleep(Duration::from_millis(10)).await;
        }
    };
    if r.during(&mut None, Instant::now() + WRITE_WAIT, serial)
        .await?
        .is_none()
    {
        return Err(Stop::Aborted(
            "no page-81 serial within 2 s of the reconnect: the trainer is not confirmed".into(),
        ));
    }
    same_trainer(&r.w.records(), detected, back).map_err(Stop::Aborted)?;
    let until = Instant::now() + ANSWER_WAIT;
    r.answer(&mut None, until, &LIGHTS).await?;
    let op = r.op;
    let confirmed = r
        .during(
            &mut None,
            until,
            op.confirm(&format!(
                "reconnected and released (0 %): re-engage ERG at {TARGET_W} W by soft start?"
            )),
        )
        .await?;
    match confirmed {
        Some(true) => r.note(WINDOW, &[CONFIRMED])?,
        Some(false) => return Err(Stop::Aborted("not confirmed after the reconnect".into())),
        None => {
            return Err(Stop::Aborted(format!(
                "no confirmation within {} s of the reconnect",
                ANSWER_WAIT.as_secs()
            )));
        }
    }
    r.engage(TARGET_W).await?;
    r.cue(Cue::Cadence(90));
    r.hold(HOLD).await?;
    Ok(())
}

/// The Part B control checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Procedure {
    /// #44 check 5.
    Check5,
    /// #44 check 6.
    Check6,
    /// #44 check 7.
    Check7,
    /// #44 check 8.
    Check8,
    /// #44 check 9.
    Check9,
    /// #44 check 10.
    Check10,
    /// #44 check 11b.
    Check11b,
    /// #44 check 12.
    Check12,
}

impl Procedure {
    /// What the procedure writes, for the summary.
    pub fn what(self) -> &'static str {
        match self {
            Self::Check5 => {
                "page 48 0 %; soft start to 150 W; 30 s at 90 rpm; page 49 200 W; 30 s; release; also subscribes (read-only) to Cycling Power 2A63 and records it beside page 25"
            }
            Self::Check6 => {
                "page 48 0 %; page 49 0 W until cadence, 5 s soft start to 100 W; 100/150/250/100/200 W held 20 s; at 200 W cues 60 then 90 rpm, 20 s each; release"
            }
            Self::Check7 => {
                "page 48 0 %; soft start to 150 W; 10 s; 30 s with no write at all; release"
            }
            Self::Check8 => {
                "page 48 0 %; after cadence: page 55 75+10 kg 708 mm, page 50 defaults, page 51 0 %; 30 s each at 0 %, 5 %, 5 % with page 55 100 kg, 10 % with 75 kg; release"
            }
            Self::Check9 => {
                "page 48 0 %; soft start to 250 W; 10 s at 90 rpm; cue to lower cadence 5 rpm per 10 s; end when power stays below 90 % for 2 s while the cadence follows the cue, or at the first sample below 25 rpm; abort when the cadence leaves the cue (falls > 9 rpm in 3 s, or > 15 rpm above it); release"
            }
            Self::Check10 => {
                "page 48 0 %; no rider; holds the link at least 60 s while the operator tries to connect the phone (a second central) and types two answers (within 180 s); a scan beside the connection records the trainer's advertisements; release"
            }
            Self::Check11b => {
                "page 48 0 %; soft start to 100 W; 20 s; page 48 0 %; cue to stop until the lights go out (unplugged); waits up to 90 s for the drop; scans and reconnects (60 s at most); page 48 0 % first; asks about the lights and for y; soft start to 100 W; 20 s; release"
            }
            Self::Check12 => {
                "page 48 0 %; soft start to 200 W; 15 s; page 50 defaults, request 71, page 51 0 %, request 71; 10 s at 90 rpm, 15 s at 70 rpm; release"
            }
        }
    }
}

/// How a driven run ended.
#[derive(Debug)]
pub struct Report {
    /// `Ok` when the procedure ran to completion.
    pub procedure: Result<(), Stop>,
    /// The release write's error, if any.
    pub release_error: Option<String>,
}

/// The release write (page 48 at 0 %, ADR-0002 §7.9), bounded at [`RELEASE_WAIT`]; its error, if any.
pub async fn release<W: Wire>(w: &W) -> Option<String> {
    match frame(&ControlPage::BasicResistance { pct: 0.0 }) {
        Ok(f) => tokio::time::timeout(RELEASE_WAIT, w.send(f))
            .await
            .unwrap_or_else(|_| Some("release write not completed within 2 s".into())),
        Err(e) => Some(format!("{e:?}")),
    }
}

/// Connect state → confirmation → procedure, then always the release.
///
/// `interrupt` resolves on a signal; the procedure is dropped at its next await and the
/// release follows. A panic in the procedure is caught and released the same way.
pub async fn drive<W: Wire, O: Operator>(
    w: &W,
    op: &O,
    p: Procedure,
    interrupt: impl Future<Output = String>,
) -> Report {
    let release = || release(w);
    if let Some(e) = release().await {
        return Report {
            procedure: Err(Stop::Failed(format!("initial page 48: {e}"))),
            release_error: Some(e),
        };
    }
    // Ask for the trainer's page-81 serial (a read-only request, bounded like the release): a
    // capture is tied to its trainer by it, which check 9 needs of check 6. An unanswered
    // request leaves the capture without a serial, so that tie fails closed (`PowerLag`).
    if let Ok(f) = frame(&ControlPage::Request { page: 81, times: 1 }) {
        let _ = tokio::time::timeout(RELEASE_WAIT, w.send(f)).await;
    }
    let run = async {
        if !op.confirm(&format!("hil_fec {p:?}: {}", p.what())).await {
            return Err(Stop::Aborted("not confirmed by the operator".into()));
        }
        let mut r = Rig::new(w, op);
        match p {
            Procedure::Check5 => check5(&mut r).await,
            Procedure::Check6 => check6(&mut r).await,
            Procedure::Check7 => check7(&mut r).await,
            Procedure::Check8 => check8(&mut r).await,
            Procedure::Check9 => check9(&mut r).await,
            Procedure::Check10 => check10(&mut r).await,
            Procedure::Check11b => check11b(&mut r).await,
            Procedure::Check12 => check12(&mut r).await,
        }
    };
    let procedure = tokio::select! {
        r = AssertUnwindSafe(run).catch_unwind() => r.unwrap_or_else(|_| Err(Stop::Failed("procedure panicked".into()))),
        sig = interrupt => Err(Stop::Interrupted(sig)),
    };
    let release_error = release().await;
    // Leave the command status of the release in the capture (evidence only, bounded).
    if let Ok(f) = frame(&ControlPage::Request { page: 71, times: 1 }) {
        sleep(Duration::from_millis(300)).await;
        let _ = tokio::time::timeout(RELEASE_WAIT, w.send(f)).await;
        sleep(Duration::from_millis(700)).await;
    }
    op.cue(w.now_us(), Cue::Stop);
    Report {
        procedure,
        release_error,
    }
}

/// SIGINT, SIGTERM or SIGHUP (a closed Alacritty window), whichever comes first.
pub async fn signals() -> String {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let wait = |kind: SignalKind, name: &'static str| async move {
            match signal(kind) {
                Ok(mut s) => {
                    s.recv().await;
                    name.to_string()
                }
                Err(_) => std::future::pending().await,
            }
        };
        tokio::select! {
            n = wait(SignalKind::interrupt(), "SIGINT") => n,
            n = wait(SignalKind::terminate(), "SIGTERM") => n,
            n = wait(SignalKind::hangup(), "SIGHUP") => n,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "Ctrl-C".to_string()
    }
}

/// How a procedure ended, as one line: `completed`, `aborted: …`, `interrupted: …`, `failed: …`.
/// The capture carries it as its outcome line ([`crate::partb::OUTCOME`]).
pub fn outcome(procedure: &Result<(), Stop>) -> String {
    let text = match procedure {
        Ok(()) => crate::partb::COMPLETED.to_string(),
        Err(Stop::Aborted(why)) => format!("aborted: {why}"),
        Err(Stop::Interrupted(sig)) => format!("interrupted: {sig}"),
        Err(Stop::Failed(why)) => format!("failed: {why}"),
    };
    text.replace(['\n', '\r'], " ")
}

/// The summary of one driven run (live or simulated); `lag` is check 6 of the capture set.
pub fn summary(
    p: Procedure,
    stem: &str,
    report: &Report,
    records: &[Record],
    cps: &[Cps],
    notes: &[Note],
    lag: &crate::partb::PowerLag,
) -> Value {
    let outcome = outcome(&report.procedure);
    let mut v = json!({
        "check": stem,
        "what_was_sent": p.what(),
        "procedure": outcome,
        "release_write_error": report.release_error,
        "page71_frames": records.iter().filter(|r| r.rx && page_of(&r.bytes) == Some(71)).count(),
    });
    v["result"] = crate::partb::analyse(stem, records, cps, notes, Some(&outcome), lag)
        .unwrap_or(Value::Null);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        frames::{CPS_TAG, hex, parse_capture, parse_cps, parse_notes},
        partb::{Control, FINISHED, MAX_T90_US, OUTCOME, PowerLag, analyse, controls, outcome_in},
        sim::{Pattern, Plan, Sim},
    };
    use std::fmt::Write as _;

    async fn run(p: Procedure, plan: Plan, interrupt_at_s: Option<u64>) -> (Sim, Report) {
        let sim = Sim::new(plan);
        let interrupt = async {
            match interrupt_at_s {
                Some(s) => {
                    sleep(Duration::from_secs(s)).await;
                    "SIGINT".to_string()
                }
                None => std::future::pending().await,
            }
        };
        let report = drive(&sim, &sim, p, interrupt).await;
        (sim, report)
    }

    /// A run bounded at 10 min of (paused) time: `None` when `drive` does not return.
    async fn bounded(p: Procedure, plan: Plan) -> (Sim, Option<Report>) {
        let sim = Sim::new(plan);
        let report = tokio::time::timeout(
            Duration::from_mins(10),
            drive(&sim, &sim, p, std::future::pending()),
        )
        .await
        .ok();
        (sim, report)
    }

    /// Writes `(at_us, page)` in order.
    fn writes(sim: &Sim) -> Vec<(u64, u8)> {
        sim.records()
            .iter()
            .filter(|r| !r.rx)
            .filter_map(|r| Some((r.at_us, page_of(&r.bytes)?)))
            .collect()
    }

    /// A control's kind and values, to compare within the codec's resolution.
    fn fields(c: Control) -> (u8, Vec<f64>) {
        match c {
            Control::Resistance(x) => (48, vec![x]),
            Control::Erg(x) => (49, vec![x]),
            Control::Wind(cw, speed, drafting) => (50, vec![cw, speed, drafting]),
            Control::Track(grade, crr) => (51, vec![grade, crr]),
            Control::User(rider, bike) => (55, vec![rider, bike]),
        }
    }

    fn ctl(sim: &Sim) -> Vec<Control> {
        controls(&sim.records())
            .into_iter()
            .map(|(_, c)| c)
            .collect()
    }

    fn erg_targets(c: &[Control]) -> Vec<f64> {
        c.iter()
            .filter_map(|c| match c {
                Control::Erg(w) => Some(*w),
                _ => None,
            })
            .collect()
    }

    /// Engagement as the engine does it (ADR-0002 §7.5): 0 W, then six 1 s ramp writes ending at
    /// the target, from an origin near the rider's 3 s power. Returns the writes after the ramp.
    fn assert_soft_start(c: &[Control], target: f64) -> Vec<Control> {
        assert_eq!(
            c[0],
            Control::Resistance(0.0),
            "connects into the release state"
        );
        assert_eq!(c[1], Control::Erg(0.0), "0 W until the rider pedals");
        let ramp = erg_targets(&c[2..8]);
        assert_eq!(ramp.len(), 6, "{c:?}");
        assert!(
            ramp.windows(2).all(|w| w[1] >= w[0]),
            "monotonic ramp {ramp:?}"
        );
        assert!(
            ramp[0] < 50.0,
            "origin is the 3 s power at the first pedal stroke, {ramp:?}"
        );
        assert!(
            (ramp[5] - target).abs() < 1e-9,
            "ends at the target, {ramp:?}"
        );
        c[8..].to_vec()
    }

    /// The last control write is page 48 at 0 %, followed by a page-70 request for page 71.
    fn assert_released(sim: &Sim, report: &Report) {
        assert_eq!(report.release_error, None);
        let c = ctl(sim);
        assert_eq!(c.last(), Some(&Control::Resistance(0.0)), "{c:?}");
        let tx: Vec<u8> = sim
            .records()
            .iter()
            .filter(|r| !r.rx)
            .filter_map(|r| page_of(&r.bytes))
            .collect();
        assert_eq!(
            tx[tx.len() - 2..],
            [48, 70],
            "release then its page-71 request"
        );
        assert!(erg_targets(&c).iter().all(|w| *w <= f64::from(MAX_ERG_W)));
    }

    /// The `.fecap` text of a run, with its outcome line as `hil_fec` appends it.
    fn fecap(sim: &Sim, report: &Report) -> String {
        let mut text = sim.records().iter().fold(String::new(), |mut t, r| {
            let dir = if r.rx { "rx" } else { "tx" };
            let _ = writeln!(t, "{} {dir} {}", r.at_us, hex(&r.bytes));
            t
        });
        for c in sim.cps() {
            let _ = writeln!(text, "{CPS_TAG}{} {}", c.at_us, hex(&c.bytes));
        }
        for n in sim.notes() {
            let _ = writeln!(text, "# {} {} {}", n.tag, n.at_us, hex(&n.bytes));
        }
        let _ = writeln!(text, "{OUTCOME}{}", outcome(&report.procedure));
        let _ = writeln!(text, "{FINISHED}{SESSION_START_S}");
        text
    }

    /// When the simulated runs ended, Unix seconds (every run of a test is of one session).
    const SESSION_START_S: u64 = 1_700_000_000;

    /// A check 6 that met the power-lag bound.
    const MEASURED: PowerLag = PowerLag::Within(MAX_T90_US);

    /// The live result equals the one re-derived offline from the written `.fecap` text.
    fn reanalysed(sim: &Sim, report: &Report, p: Procedure, stem: &str) -> Value {
        reanalysed_with(sim, report, p, stem, &MEASURED)
    }

    /// [`reanalysed`] with check 6 of the capture set measuring `lag`.
    fn reanalysed_with(
        sim: &Sim,
        report: &Report,
        p: Procedure,
        stem: &str,
        lag: &PowerLag,
    ) -> Value {
        let live = summary(
            p,
            stem,
            report,
            &sim.records(),
            &sim.cps(),
            &sim.notes(),
            lag,
        )["result"]
            .clone();
        let text = fecap(sim, report);
        let offline = analyse(
            stem,
            &parse_capture(&text),
            &parse_cps(&text),
            &parse_notes(&text),
            outcome_in(&text).as_deref(),
            lag,
        )
        .expect("part B stem");
        assert_eq!(live, offline);
        live
    }

    #[tokio::test(start_paused = true)]
    async fn check6_steps_cadence_and_release() {
        let (sim, report) = run(Procedure::Check6, Plan::default(), None).await;
        assert_eq!(report.procedure, Ok(()));
        assert_released(&sim, &report);
        let rest = assert_soft_start(&ctl(&sim), 100.0);
        assert_eq!(
            rest,
            [150.0, 250.0, 100.0, 200.0]
                .map(Control::Erg)
                .into_iter()
                .chain([Control::Resistance(0.0)])
                .collect::<Vec<_>>()
        );
        assert_eq!(
            sim.cues(),
            [
                Cue::Start,
                Cue::Cadence(90),
                Cue::Cadence(60),
                Cue::Cadence(90),
                Cue::Stop
            ]
        );
        let v = reanalysed(&sim, &report, Procedure::Check6, "check6");
        let steps: Vec<(f64, f64)> = v["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .map(|s| {
                (
                    s["from_w"].as_f64().expect("from"),
                    s["to_w"].as_f64().expect("to"),
                )
            })
            .collect();
        assert_eq!(
            steps,
            [
                (100.0, 150.0),
                (150.0, 250.0),
                (250.0, 100.0),
                (100.0, 200.0)
            ]
        );
        // The sim's 0.8 s lag reaches 90 % in 1.84 s; samples arrive every 0.35 s.
        for s in v["steps"].as_array().expect("steps") {
            let t = s["t90_s"].as_f64().expect("t90 measured");
            assert!((1.8..=2.3).contains(&t), "{s}");
        }
        let dirs: Vec<&str> = v["cadence_steps"]
            .as_array()
            .expect("cadence steps")
            .iter()
            .map(|s| s["direction"].as_str().expect("dir"))
            .collect();
        assert_eq!(dirs, ["down", "up"]);
        assert_eq!(v["release"]["page71_echoed"], json!(true));
        assert!(
            v["line"]
                .as_str()
                .expect("line")
                .starts_with("| 6 | ERG steps")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check7_writes_nothing_for_the_window_and_holds() {
        let (sim, report) = run(Procedure::Check7, Plan::default(), None).await;
        assert_eq!(report.procedure, Ok(()));
        assert_released(&sim, &report);
        assert_eq!(
            assert_soft_start(&ctl(&sim), 150.0),
            [Control::Resistance(0.0)]
        );
        let v = reanalysed(&sim, &report, Procedure::Check7, "check7");
        assert!(v["silent_s"].as_f64().expect("silent") >= 40.0, "{v}");
        assert_eq!(
            (v["target_w"].as_f64(), &v["held"]),
            (Some(150.0), &json!(true))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check8_sim_grades_and_mass_without_erg() {
        let (sim, report) = run(Procedure::Check8, Plan::default(), None).await;
        assert_eq!(report.procedure, Ok(()));
        assert_released(&sim, &report);
        let want = [
            Control::Resistance(0.0),
            Control::User(75.0, 10.0),
            Control::Wind(0.51, 0.0, 1.0),
            Control::Track(0.0, 0.004),
            Control::Track(5.0, 0.004),
            Control::User(100.0, 10.0),
            Control::User(75.0, 10.0),
            Control::Track(10.0, 0.004),
            Control::Resistance(0.0),
        ];
        // The codec quantises to its resolution (0.01 kg/m, 0.00005 Crr): compare within it.
        let got = ctl(&sim);
        assert_eq!(got.len(), want.len(), "{got:?}");
        for (g, w) in got.iter().zip(want) {
            let ((kind_g, got_v), (kind_w, want_v)) = (fields(*g), fields(w));
            let close = got_v.iter().zip(&want_v).all(|(x, y)| (x - y).abs() < 1e-3);
            assert!(kind_g == kind_w && close, "{got:?}");
        }
        let v = reanalysed(&sim, &report, Procedure::Check8, "check8");
        let seg: Vec<(f64, f64, f64)> = v["segments"]
            .as_array()
            .expect("segments")
            .iter()
            .map(|s| {
                (
                    s["grade_pct"].as_f64().expect("g"),
                    s["mass_kg"].as_f64().expect("m"),
                    s["measured_over_model"].as_f64().expect("ratio"),
                )
            })
            .collect();
        assert_eq!(
            seg.iter().map(|s| (s.0, s.1)).collect::<Vec<_>>(),
            [(0.0, 85.0), (5.0, 85.0), (5.0, 110.0), (10.0, 85.0)]
        );
        // The sim applies the same road model, so measured ≈ model.
        assert!(seg.iter().all(|s| (0.9..=1.1).contains(&s.2)), "{seg:?}");
    }

    /// Release (page 48) and the page-70 request after it: `(at_us of the release)`.
    fn release_at(sim: &Sim) -> u64 {
        let w = writes(sim);
        assert_eq!(w[w.len() - 2].1, 48, "{w:?}");
        w[w.len() - 2].0
    }

    #[tokio::test(start_paused = true)]
    async fn check9_descends_to_the_stall_and_releases() {
        // ERG collapses below 35 rpm while the rider follows the cue: a trainer stall.
        let (sim, report) = run(Procedure::Check9, Plan::default(), None).await;
        assert_eq!(report.procedure, Ok(()));
        assert_released(&sim, &report);
        assert_eq!(
            assert_soft_start(&ctl(&sim), 250.0),
            [Control::Resistance(0.0)]
        );
        assert_eq!(
            sim.cues(),
            [Cue::Start, Cue::Cadence(90), Cue::Descend, Cue::Stop]
        );
        let v = reanalysed(&sim, &report, Procedure::Check9, "check9");
        assert_eq!(v["descent"], json!("stalled"), "{v}");
        // Held 250 W settled at 35 rpm; the cue steps to 30 rpm and power falls below 225 W.
        assert_eq!(v["descent_detail"]["dropped_at_rpm"], json!(30), "{v}");
        assert_eq!(v["held_settled_to_rpm"], json!(35), "{v}");
        assert_eq!(v["lowest_followed_rpm"], json!(35));
        assert_eq!(
            v["verdict"],
            json!("stall below the release cadence: the release comes first")
        );
        // The hold ends when the stall is confirmed (2 s), and the release follows at once.
        let at = v["descent_detail"]["at_s"].as_f64().expect("at");
        let onset = v["descent_detail"]["onset_s"].as_f64().expect("onset");
        assert!((2.0..2.5).contains(&(at - onset)), "{v}");
        let released = crate::partb::secs(release_at(&sim));
        assert!((at..=at + 0.25).contains(&released), "{released} {v}");
        // Without the outcome line (a killed run) the same capture gives no verdict.
        let v = analyse("check9", &sim.records(), &[], &[], None, &MEASURED).expect("part B stem");
        assert_eq!(v["verdict"], Value::Null, "{v}");
    }

    #[tokio::test(start_paused = true)]
    async fn check9_ends_at_25_rpm_when_erg_holds() {
        // ERG never collapses: the descent ends at the first sample below 25 rpm.
        let plan = Plan {
            erg_floor_rpm: 0.0,
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check9, plan, None).await;
        assert_eq!(report.procedure, Ok(()));
        assert_released(&sim, &report);
        let below = crate::partb::samples(&sim.records())
            .into_iter()
            .find(|s| s.at_us > 30 * SEC && s.cadence.is_some_and(|c| c < 25))
            .expect("a sample below 25 rpm")
            .at_us;
        let released = release_at(&sim);
        assert!(
            (below..=below + 200_000).contains(&released),
            "released at {released} us, first sample below 25 rpm at {below} us"
        );
        let v = reanalysed(&sim, &report, Procedure::Check9, "check9");
        assert_eq!(v["descent"], json!("floor"), "{v}");
        assert_eq!(v["verdict"], json!(FIRST), "{v}");
    }

    #[tokio::test(start_paused = true)]
    async fn check9_aborts_a_rider_who_does_not_descend() {
        let plan = Plan {
            descent: Pattern::Hold,
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check9, plan, None).await;
        assert!(
            matches!(&report.procedure, Err(Stop::Aborted(why)) if why.contains("did not follow the cued descent")),
            "{report:?}"
        );
        assert_released(&sim, &report);
        // 90 rpm is 15 rpm above the cue 30 s into the descent.
        let descend = sim.cue_at(Cue::Descend).expect("descend cue");
        let after = release_at(&sim) - descend;
        assert!((29_500_000..=30_600_000).contains(&after), "{after} us");
        let v = reanalysed(&sim, &report, Procedure::Check9, "check9");
        assert_eq!(
            (&v["descent"], &v["verdict"]),
            (&json!("departed"), &Value::Null)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check9_aborts_when_cadence_is_unavailable() {
        let descend = descend_cue_us().await;
        let plan = Plan {
            cadence_unavailable_from_us: Some(descend + 5 * SEC),
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check9, plan, None).await;
        assert!(
            matches!(&report.procedure, Err(Stop::Aborted(why)) if why.contains("cadence or power unavailable")),
            "{report:?}"
        );
        assert_released(&sim, &report);
        assert!(release_at(&sim) < descend + 6 * SEC);
    }

    #[tokio::test(start_paused = true)]
    async fn check9_a_rider_who_stops_while_power_falls_is_no_stall() {
        // ERG lets go below 50 rpm; the rider stops 0.5 s into the power drop, before it has
        // lasted 2 s: cadence and power fall together, so no stall cadence can be named.
        let plan = Plan {
            erg_floor_rpm: 50.0,
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check9, plan, None).await;
        let v = reanalysed(&sim, &report, Procedure::Check9, "check9");
        let onset = v["descent_detail"]["onset_s"].as_f64().expect("onset");
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let stop = (onset * 1e6) as u64 + 500_000;
        let plan = Plan {
            erg_floor_rpm: 50.0,
            stop_at_us: Some(stop),
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check9, plan, None).await;
        assert!(
            matches!(&report.procedure, Err(Stop::Aborted(why)) if why.contains("fell together")),
            "{report:?}"
        );
        assert_released(&sim, &report);
        let v = reanalysed(&sim, &report, Procedure::Check9, "check9");
        assert_eq!(v["verdict"], Value::Null, "{v}");
    }

    /// Check 6 on the trainer of `plan`, as its capture set holds it.
    async fn check6_of(plan: Plan) -> PowerLag {
        let (sim, report) = run(Procedure::Check6, plan, None).await;
        assert_eq!(report.procedure, Ok(()));
        PowerLag::of_check6(Some(&fecap(&sim, &report)))
    }

    #[tokio::test(start_paused = true)]
    async fn the_simulated_trainer_meets_the_power_lag_bound() {
        // A 0.8 s lag: t90 is 1.84 s plus up to 0.35 s to the next page-25 sample.
        let lag = check6_of(Plan::default()).await;
        assert!(
            matches!(lag, PowerLag::Within(t) if (1_840_000..=2_200_000).contains(&t)),
            "{lag:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check9_first_needs_check_6_within_the_power_lag_bound() {
        // ERG at 224 W (below 90 %) from 40.5 rpm down, with a 2 s power lag: the rider follows
        // to 43 rpm, then 37 rpm and holds. Power still reads >= 225 W for 6 s at 37 rpm, so
        // the settled hold says the trainer holds at 39 rpm; it does not at 40 rpm.
        let plan = Plan {
            power_tau_s: 2.0,
            erg_curve: Some(&[(40.5, 0.896), (43.0, 1.0)]),
            descent: Pattern::Path(&[(0, 90.0), (100_000_000, 43.0), (101_500_000, 37.0)]),
            ..Plan::default()
        };
        let lag = check6_of(plan).await;
        let PowerLag::TooSlow(Some(t90)) = lag else {
            panic!("{lag:?}")
        };
        assert!(t90 > 4 * SEC, "{lag:?}");
        let (sim, report) = run(Procedure::Check9, plan, None).await;
        assert_eq!(report.procedure, Ok(()));
        let v = reanalysed_with(&sim, &report, Procedure::Check9, "check9", &lag);
        assert_eq!(v["held_settled_to_rpm"], json!(37), "{v}");
        assert_eq!(
            v["verdict"],
            json!(format!(
                "not valid: power response too slow (t90 = {:.2} s)",
                crate::partb::secs(t90)
            )),
            "{v}"
        );
        // A trainer within the bound (0.8 s lag, ERG letting go below 35 rpm): "first" with a
        // check 6 that shows it, and only conditionally without one.
        let lag = check6_of(Plan::default()).await;
        let (sim, report) = run(Procedure::Check9, Plan::default(), None).await;
        let v = reanalysed_with(&sim, &report, Procedure::Check9, "check9", &lag);
        assert_eq!(v["verdict"], json!(FIRST), "{v}");
        let none = PowerLag::of_check6(None);
        let v = reanalysed_with(&sim, &report, Procedure::Check9, "check9", &none);
        assert_eq!(
            v["verdict"],
            json!(
                "stall below the release cadence: the release comes first only if check 6 t90 ≤ 2.3 s (not measured)"
            ),
            "{v}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check9_reads_check_6_from_its_capture_set() {
        let dir = tempfile::tempdir().expect("tmp");
        let check9 = dir.path().join("check9.fecap");
        let (sim9, report9) = run(Procedure::Check9, Plan::default(), None).await;
        let text9 = fecap(&sim9, &report9);
        assert!(matches!(
            PowerLag::beside(&check9, &text9),
            PowerLag::NotMeasured(_)
        ));
        let (sim, report) = run(Procedure::Check6, Plan::default(), None).await;
        let text = fecap(&sim, &report);
        let without_outcome: String =
            text.lines()
                .filter(|l| !l.starts_with(OUTCOME))
                .fold(String::new(), |mut t, l| {
                    let _ = writeln!(t, "{l}");
                    t
                });
        std::fs::write(dir.path().join("check6.fecap"), &without_outcome).expect("write");
        assert!(matches!(
            PowerLag::beside(&check9, &text9),
            PowerLag::NotMeasured(_)
        ));
        // The same steps, but the procedure did not complete.
        let aborted = format!("{without_outcome}{OUTCOME}aborted: the rider stopped\n");
        std::fs::write(dir.path().join("check6.fecap"), aborted).expect("write");
        assert!(matches!(
            PowerLag::beside(&check9, &text9),
            PowerLag::NotMeasured(_)
        ));
        std::fs::write(dir.path().join("check6.fecap"), &text).expect("write");
        assert!(matches!(
            PowerLag::beside(&check9, &text9),
            PowerLag::Within(_)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn check9_descent_starts_where_analyse_starts_it_when_the_soft_start_is_at_250_w() {
        // The rider already makes 260 W: the soft start writes 250 W six times, 1 s apart.
        let plan = Plan {
            warm_w: 260.0,
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check9, plan, None).await;
        assert_eq!(report.procedure, Ok(()));
        let ramp = &erg_targets(&ctl(&sim))[1..7];
        assert!(ramp.iter().all(|w| (w - 250.0).abs() < 1e-9), "{ramp:?}");
        let v = reanalysed(&sim, &report, Procedure::Check9, "check9");
        let from = v["descent_from_s"].as_f64().expect("from");
        let first = writes(&sim)
            .iter()
            .find(|w| w.1 == 49 && w.0 > 0)
            .expect("ramp")
            .0;
        assert!(
            (from - crate::partb::secs(first + 10 * SEC)).abs() < 0.05,
            "{v}"
        );
        let cue = crate::partb::secs(sim.cue_at(Cue::Descend).expect("cue"));
        assert!((from..from + 0.2).contains(&cue), "cue at {cue} s: {v}");
        assert_eq!(v["verdict"], json!(FIRST), "{v}");
    }

    const FIRST: &str = "stall below the release cadence: the release comes first";
    const TOO_LATE: &str = "stall at or above the release cadence: the release comes too late";

    /// Ramps, step patterns and step-then-hold riders that follow the cue.
    fn riders() -> Vec<(String, Pattern)> {
        // Faster from 90 rpm, a rider falls more than 15 rpm below the cue before power drops
        // and departs without a verdict; these follow the cue to 50 rpm (at 80 s) first.
        const AT_50: u64 = 80_000_000;
        let leak = |path: Vec<(u64, f64)>| -> Pattern { Pattern::Path(Box::leak(path.into())) };
        let mut out = vec![
            ("steps 5 rpm / 10 s".to_string(), Plan::default().descent),
            (
                "ramp 0.5 rpm/s".to_string(),
                leak(vec![(0, 90.0), (180_000_000, 0.0)]),
            ),
        ];
        for (rpm, every_us) in [(9.0, 3_050_000), (9.0, 6_000_000), (3.0, 3_000_000)] {
            let mut path = vec![(0, 90.0), (AT_50, 50.0)];
            let mut k = 0_u32;
            while 50.0 - rpm * f64::from(k) > 20.0 {
                k += 1;
                let at = AT_50 + u64::from(k) * every_us;
                path.extend([
                    (at - 1, 50.0 - rpm * f64::from(k - 1)),
                    (at, 50.0 - rpm * f64::from(k)),
                ]);
            }
            out.push((
                format!("cue to 50, steps {rpm} rpm / {every_us} us"),
                leak(path),
            ));
        }
        for rate in [1.0, 2.0, 2.9, 3.1] {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let end = AT_50 + (50.0 / rate * 1e6) as u64;
            out.push((
                format!("cue to 50, ramp {rate} rpm/s"),
                leak(vec![(0, 90.0), (AT_50, 50.0), (end, 0.0)]),
            ));
        }
        for hold in 36_u32..=44 {
            // With the cue down to 6 rpm above, then a 6 rpm step, then hold.
            let above = f64::from(hold + 6);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let at = ((90.0 - above) / 0.5 * 1e6) as u64;
            let path: &'static [(u64, f64)] = Box::leak(Box::new([
                (0, 90.0),
                (at, above),
                (at + 500_000, f64::from(hold)),
            ]));
            out.push((
                format!("cue to {above}, step to {hold} and hold"),
                Pattern::Path(path),
            ));
        }
        out
    }

    #[tokio::test(start_paused = true)]
    async fn check9_an_unsafe_trainer_is_never_reported_safe() {
        // ERG lets go below `floor` (power 250 W x cadence / floor): below 225 W at 40 rpm and
        // above is an unsafe trainer (the release comes too late), above 225 W a safe one.
        // Floors 40-50 rpm in 0.5 rpm steps, and 30 and 35 rpm, where a safe trainer resolves.
        let mut wrong = Vec::new();
        let mut judged = std::collections::BTreeSet::new();
        let mut verdicts = std::collections::BTreeMap::<String, usize>::new();
        for half in [60_u32, 70].into_iter().chain(80..=100) {
            let floor = f64::from(half) / 2.0;
            let unsafe_trainer = 250.0 * 40.0 / floor < 225.0;
            for (name, descent) in riders() {
                let plan = Plan {
                    erg_floor_rpm: floor,
                    descent,
                    ..Plan::default()
                };
                let (sim, report) = run(Procedure::Check9, plan, None).await;
                let v = summary(
                    Procedure::Check9,
                    "check9",
                    &report,
                    &sim.records(),
                    &[],
                    &[],
                    &MEASURED,
                )["result"]
                    .clone();
                let verdict = v["verdict"].as_str().unwrap_or("no verdict").to_string();
                if unsafe_trainer && !v["verdict"].is_null() {
                    judged.insert(name.clone());
                }
                *verdicts
                    .entry(verdict.split(':').next().unwrap_or("").to_string())
                    .or_default() += 1;
                if (unsafe_trainer && verdict == FIRST) || (!unsafe_trainer && verdict == TOO_LATE)
                {
                    wrong.push(format!(
                        "floor {floor}, {name}: {verdict} ({})",
                        v["descent_detail"]
                    ));
                }
            }
        }
        // Not vacuous: every rider is judged on an unsafe trainer, and the sweep resolves both
        // ways somewhere.
        assert_eq!(judged.len(), riders().len(), "{judged:?}");
        assert!(
            verdicts.contains_key("stall below the release cadence")
                && verdicts.contains_key("stall at or above the release cadence"),
            "{verdicts:?}"
        );
        assert!(
            wrong.is_empty(),
            "{} wrong of the sweep ({verdicts:?}):\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check9_a_rider_who_stops_at_any_time_is_no_stall() {
        // Every 0.5 s from the cue to the power drop of an undisturbed run, ERG letting go
        // below 35 and below 50 rpm: a stop is never a stall.
        let descend = descend_cue_us().await;
        for floor in [35.0, 50.0] {
            let plan = Plan {
                erg_floor_rpm: floor,
                ..Plan::default()
            };
            let (sim, report) = run(Procedure::Check9, plan, None).await;
            let v = summary(
                Procedure::Check9,
                "check9",
                &report,
                &sim.records(),
                &[],
                &[],
                &MEASURED,
            )["result"]
                .clone();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let end = (v["descent_detail"]["onset_s"].as_f64().expect("onset") * 1e6) as u64;
            let mut offset = 0;
            while descend + offset < end {
                let plan = Plan {
                    erg_floor_rpm: floor,
                    stop_at_us: Some(descend + offset),
                    ..Plan::default()
                };
                let (sim, report) = run(Procedure::Check9, plan, None).await;
                let v = summary(
                    Procedure::Check9,
                    "check9",
                    &report,
                    &sim.records(),
                    &[],
                    &[],
                    &MEASURED,
                )["result"]
                    .clone();
                assert!(
                    matches!(&report.procedure, Err(Stop::Aborted(_))) && v["verdict"].is_null(),
                    "floor {floor}, stop at cue + {offset} us: {report:?} {}",
                    v["verdict"]
                );
                offset += 500_000;
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn unavailable_cadence_under_erg_counts_as_low() {
        // §7.4: a cadence the trainer cannot report is no proof of pedalling.
        let plan = Plan {
            cadence_unavailable_from_us: Some(30 * SEC),
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check6, plan, None).await;
        assert!(
            matches!(&report.procedure, Err(Stop::Aborted(why)) if why.contains("40 rpm")),
            "{report:?}"
        );
        assert_released(&sim, &report);
        assert!(release_at(&sim) <= 32_500_000, "{}", release_at(&sim));
    }

    /// The Descend cue time of an undisturbed check 9 run (the same in every run until the
    /// plan diverges).
    async fn descend_cue_us() -> u64 {
        let (sim, _) = run(Procedure::Check9, Plan::default(), None).await;
        sim.cue_at(Cue::Descend).expect("descend cue")
    }

    #[tokio::test(start_paused = true)]
    async fn check9_a_rider_who_stops_is_no_stall() {
        // Falsifier repro (#421): the rider stops abruptly at the cue + 0/1/5/20/60 s, with
        // ERG collapsing below 35 rpm and below 50 rpm. That is not a trainer stall: the run
        // is aborted, no verdict, and the trainer is released.
        let descend = descend_cue_us().await;
        for floor in [35.0, 50.0] {
            for offset in [0, 1, 5, 20, 60] {
                let plan = Plan {
                    stop_at_us: Some(descend + offset * SEC),
                    erg_floor_rpm: floor,
                    ..Plan::default()
                };
                let (sim, report) = run(Procedure::Check9, plan, None).await;
                let case = format!("collapse below {floor} rpm, stop at cue + {offset} s");
                assert!(
                    matches!(&report.procedure, Err(Stop::Aborted(why)) if why.contains("rider")),
                    "{case}: {report:?}"
                );
                assert_released(&sim, &report);
                let v = reanalysed(&sim, &report, Procedure::Check9, "check9");
                assert_eq!(v["verdict"], Value::Null, "{case}: {v}");
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn check9_slow_descent_verdicts() {
        // Collapse below 35 rpm, 5 rpm steps: held at 35, fell at 30. Collapse below 50 rpm on
        // a smooth 0.5 rpm/s ramp: power below 225 W at 44 rpm. With 5 rpm steps instead the
        // trainer is seen holding at 45 and failing at 40 only: ambiguous, not guessed.
        static RAMP: [(u64, f64); 2] = [(0, 90.0), (180_000_000, 0.0)];
        for (floor, descent, verdict) in [
            (35.0, Plan::default().descent, FIRST),
            (50.0, Pattern::Path(&RAMP), TOO_LATE),
            (50.0, Plan::default().descent, "ambiguous"),
        ] {
            let plan = Plan {
                erg_floor_rpm: floor,
                descent,
                ..Plan::default()
            };
            let (sim, report) = run(Procedure::Check9, plan, None).await;
            assert_eq!(report.procedure, Ok(()), "{floor}");
            assert_released(&sim, &report);
            let v = reanalysed(&sim, &report, Procedure::Check9, "check9");
            assert!(
                v["verdict"]
                    .as_str()
                    .is_some_and(|x| x.starts_with(verdict)),
                "{floor}: {v}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn check12_free_releases_erg() {
        let (sim, report) = run(Procedure::Check12, Plan::default(), None).await;
        assert_eq!(report.procedure, Ok(()));
        assert_released(&sim, &report);
        let rest = assert_soft_start(&ctl(&sim), 200.0);
        assert!(
            matches!(rest[..], [Control::Wind(..), Control::Track(g, _), Control::Resistance(r)] if g == 0.0 && r == 0.0),
            "{rest:?}"
        );
        let v = reanalysed(&sim, &report, Procedure::Check12, "check12");
        assert_eq!(v["released"], json!(true), "{v}");
        assert_eq!(
            (&v["page71_echoed_50"], &v["page71_echoed_51"]),
            (&json!(true), &json!(true))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn low_cadence_under_erg_aborts_and_releases() {
        let plan = Plan {
            stop_at_us: Some(40 * SEC),
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check6, plan, None).await;
        assert!(
            matches!(&report.procedure, Err(Stop::Aborted(why)) if why.contains("40 rpm")),
            "{report:?}"
        );
        assert_released(&sim, &report);
        // Nothing but the release after the last ERG write that preceded the stop.
        let c = ctl(&sim);
        assert_eq!(c[c.len() - 2], Control::Erg(150.0));
    }

    #[tokio::test(start_paused = true)]
    async fn a_signal_mid_run_releases() {
        let (sim, report) = run(Procedure::Check9, Plan::default(), Some(20)).await;
        assert_eq!(report.procedure, Err(Stop::Interrupted("SIGINT".into())));
        assert_released(&sim, &report);
        let last_rx = sim
            .records()
            .iter()
            .filter(|r| !r.rx)
            .map(|r| r.at_us)
            .max()
            .expect("tx");
        assert!(
            last_rx < 22 * SEC,
            "released at once, not after the procedure"
        );
        // Interrupted mid-descent: the samples so far would give a verdict; it is withheld.
        assert!(sim.cue_at(Cue::Descend).is_some());
        let v = reanalysed(&sim, &report, Procedure::Check9, "check9");
        assert!(v["bins"].as_array().is_some_and(|b| !b.is_empty()), "{v}");
        assert_eq!(v["verdict"], Value::Null, "{v}");
        assert!(
            v["line"]
                .as_str()
                .expect("line")
                .contains("no result: procedure interrupted: SIGINT"),
            "{v}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_applied_without_confirmation_or_a_rider() {
        let (sim, report) = run(
            Procedure::Check6,
            Plan {
                confirms: false,
                ..Plan::default()
            },
            None,
        )
        .await;
        assert!(matches!(report.procedure, Err(Stop::Aborted(_))));
        assert_eq!(
            ctl(&sim),
            [Control::Resistance(0.0), Control::Resistance(0.0)]
        );
        let (sim, report) = run(
            Procedure::Check12,
            Plan {
                pedals: false,
                ..Plan::default()
            },
            None,
        )
        .await;
        assert!(
            matches!(&report.procedure, Err(Stop::Aborted(why)) if why.contains("did not start")),
            "{report:?}"
        );
        assert_eq!(
            ctl(&sim),
            [
                Control::Resistance(0.0),
                Control::Erg(0.0),
                Control::Resistance(0.0)
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_dropped_link_fails_the_run_and_still_attempts_the_release() {
        let plan = Plan {
            drop_at_us: Some(30 * SEC),
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check7, plan, None).await;
        assert!(
            matches!(&report.procedure, Err(Stop::Failed(why)) if why.contains("link dropped")),
            "{report:?}"
        );
        assert_eq!(
            report.release_error.as_deref(),
            Some("not connected"),
            "the release write was attempted"
        );
        assert_eq!(sim.cues().last(), Some(&Cue::Stop));
    }

    #[tokio::test(start_paused = true)]
    async fn limits_refuse_unsafe_targets() {
        let sim = Sim::new(Plan::default());
        let mut r = Rig::new(&sim, &sim);
        assert!(matches!(r.erg(MAX_ERG_W + 1).await, Err(Stop::Failed(_))));
        for grade in [
            MAX_GRADE_PCT + 0.5,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ] {
            assert!(
                matches!(r.track(grade).await, Err(Stop::Failed(why)) if why.contains("outside")),
                "{grade}: refused by the limit, not left to the encoder"
            );
        }
        assert!(sim.records().iter().all(|x| x.rx), "nothing written");
    }

    #[tokio::test(start_paused = true)]
    async fn a_ride_that_never_started_is_no_result() {
        let plan = Plan {
            pedals: false,
            ..Plan::default()
        };
        for p in [
            Procedure::Check6,
            Procedure::Check7,
            Procedure::Check8,
            Procedure::Check9,
            Procedure::Check12,
        ] {
            let (sim, report) = run(p, plan, None).await;
            assert!(matches!(&report.procedure, Err(Stop::Aborted(_))));
            let stem = format!("{p:?}").to_lowercase();
            // Live and offline (`hil_fec analyse` reads the outcome line of the capture).
            let v = reanalysed(&sim, &report, p, &stem);
            let line = v["line"].as_str().expect("line");
            assert!(
                line.contains("no result: procedure aborted") && !line.contains("true"),
                "{line}"
            );
            assert_eq!(
                (&v["held"], &v["released"], &v["verdict"]),
                (&Value::Null, &Value::Null, &Value::Null),
                "{v}"
            );
            assert_eq!(v["completed"], json!(false));
        }
        // A capture without an outcome line (killed run, older capture) is no result either.
        let (sim, report) = run(Procedure::Check7, Plan::default(), None).await;
        assert_eq!(report.procedure, Ok(()));
        let v = analyse("check7", &sim.records(), &[], &[], None, &MEASURED).expect("part B stem");
        assert_eq!(v["held"], Value::Null, "{v}");
        assert!(v["line"].as_str().expect("line").contains("no result"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_hung_write_fails_and_the_exit_stays_bounded() {
        // From then on no write is acknowledged: under ERG the 250 W step, under SIM the
        // 5 % grade; then the release and its request.
        for (p, from_s) in [(Procedure::Check6, 40), (Procedure::Check8, 20)] {
            hung_write(p, from_s).await;
        }
    }

    async fn hung_write(p: Procedure, from_s: u64) {
        let plan = Plan {
            hang_from_us: Some(from_s * SEC),
            ..Plan::default()
        };
        let (sim, report) = bounded(p, plan).await;
        let report = report.expect("drive returns although no write completes");
        assert!(
            matches!(&report.procedure, Err(Stop::Failed(why)) if why.contains("not completed within 2 s")),
            "{report:?}"
        );
        assert_eq!(
            report.release_error.as_deref(),
            Some("release write not completed within 2 s")
        );
        let w = writes(&sim);
        let hung = w
            .iter()
            .position(|(at, _)| *at >= from_s * SEC)
            .expect("hung write");
        let (at, _) = w[hung];
        assert_eq!(
            w[hung + 1..].iter().map(|x| x.1).collect::<Vec<_>>(),
            [48, 70]
        );
        assert!(
            w[hung + 1].0 <= at + 2_100_000,
            "released 2 s after the stuck write"
        );
        assert!(sim.now_us() <= at + 8 * SEC, "the exit is bounded too");
        assert_eq!(sim.cues().last(), Some(&Cue::Stop));
    }

    #[tokio::test(start_paused = true)]
    async fn the_guards_keep_running_while_a_write_is_pending() {
        // The 250 W write (at about 46 s) never completes; the rider has stopped, or the
        // trainer has gone silent, just before it: the guard stops the run before the write's
        // own 2 s limit.
        for (plan, why) in [
            (
                Plan {
                    stop_at_us: Some(43 * SEC),
                    hang_from_us: Some(45 * SEC),
                    ..Plan::default()
                },
                "40 rpm",
            ),
            (
                Plan {
                    silent_from_us: Some(45 * SEC),
                    hang_from_us: Some(45 * SEC),
                    ..Plan::default()
                },
                "no page 25",
            ),
        ] {
            let (sim, report) = bounded(Procedure::Check6, plan).await;
            let report = report.expect("drive returns");
            assert!(
                matches!(&report.procedure, Err(Stop::Aborted(w)) if w.contains(why)),
                "{why}: {report:?}"
            );
            let w = writes(&sim);
            let hung = w.iter().position(|(at, _)| *at >= 45 * SEC).expect("hung");
            assert!(
                w[hung + 1].0 < w[hung].0 + 2 * SEC,
                "{why}: stopped while the write was pending, {w:?}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_panic_in_the_procedure_releases() {
        let plan = Plan {
            panic_at_us: Some(30 * SEC),
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check6, plan, None).await;
        assert_eq!(
            report.procedure,
            Err(Stop::Failed("procedure panicked".into()))
        );
        assert_released(&sim, &report);
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_trainer_aborts_and_releases() {
        let plan = Plan {
            silent_from_us: Some(30 * SEC),
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check7, plan, None).await;
        assert!(
            matches!(&report.procedure, Err(Stop::Aborted(why)) if why.contains("no page 25")),
            "{report:?}"
        );
        assert_released(&sim, &report);
        let w = writes(&sim);
        assert!(
            w[w.len() - 2].0 <= 32_200_000,
            "released 2 s into the silence, {w:?}"
        );
    }

    /// Check 10's events in the capture, adverts left out: `(tag, payload)`.
    fn check10_events(sim: &Sim) -> Vec<(String, Vec<u8>)> {
        sim.notes()
            .into_iter()
            .filter(|n| n.tag != crate::check10::ADV)
            .map(|n| (n.tag, n.bytes))
            .collect()
    }

    fn event_at(sim: &Sim, tag: &str, code: u8) -> u64 {
        sim.notes()
            .iter()
            .find(|n| n.tag == tag && n.bytes == [code])
            .expect("event")
            .at_us
    }

    #[tokio::test(start_paused = true)]
    async fn check10_holds_the_link_records_the_answers_and_releases() {
        use crate::check10::{CLOSE, OPEN, WINDOW};
        let (sim, report) = run(Procedure::Check10, Plan::default(), None).await;
        assert_eq!(report.procedure, Ok(()));
        assert_released(&sim, &report);
        assert_eq!(
            ctl(&sim),
            [Control::Resistance(0.0), Control::Resistance(0.0)],
            "nothing but the release state"
        );
        assert_eq!(sim.cues(), [Cue::Phone, Cue::Stop]);
        let e = |t: &str, b: &[u8]| (t.to_string(), b.to_vec());
        assert_eq!(
            check10_events(&sim),
            [
                e("scan", &[1]),
                e("check10", &[1]),
                e("answer-listed", b"seen"),
                e("answer-connect", b"stuck"),
                e("check10", &[2]),
                e("scan", &[2]),
            ]
        );
        // Answered at 20 s; the window still lasts its 60 s.
        let open = event_at(&sim, WINDOW, OPEN);
        let close = event_at(&sim, WINDOW, CLOSE);
        assert!(
            (60 * SEC..60 * SEC + 200_000).contains(&(close - open)),
            "{open} {close}"
        );
        let v = reanalysed(&sim, &report, Procedure::Check10, "check10");
        assert_eq!(
            v["second_central"],
            json!("refused: the phone's connect never completed")
        );
        let adv = v["advertising"].as_str().expect("advertising");
        assert!(
            adv.starts_with("continues: the scan saw 6")
                && adv.ends_with("longest gap 1 s; the phone: seen"),
            "{v}"
        );
        assert_eq!(v["link"], json!("held"));
        assert!(
            v["line"].as_str().expect("line").starts_with(
                "| 10 | Second central (phone) while connected | second central refused"
            ),
            "{v}"
        );
        // A killed run (no outcome line) gives neither verdict.
        let v = analyse(
            "check10",
            &sim.records(),
            &[],
            &sim.notes(),
            None,
            &MEASURED,
        )
        .expect("part B stem");
        assert_eq!(
            (&v["verdict"], &v["advertising"], &v["second_central"]),
            (&Value::Null, &Value::Null, &Value::Null)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check10_answers_after_180_s_or_closed_input_abort_with_no_verdict() {
        for (plan, why) in [
            (
                Plan {
                    answer_at_us: 181 * SEC,
                    ..Plan::default()
                },
                "within 180 s",
            ),
            (
                Plan {
                    answers: &["seen"],
                    ..Plan::default()
                },
                "the input closed before an answer to \"Try to connect",
            ),
        ] {
            let (sim, report) = bounded(Procedure::Check10, plan).await;
            let report = report.expect("drive returns");
            assert!(
                matches!(&report.procedure, Err(Stop::Aborted(w)) if w.contains(why)),
                "{why}: {report:?}"
            );
            assert_released(&sim, &report);
            let v = reanalysed(&sim, &report, Procedure::Check10, "check10");
            assert_eq!(
                (&v["verdict"], &v["advertising"], &v["second_central"]),
                (&Value::Null, &Value::Null, &Value::Null),
                "{why}: {v}"
            );
            assert!(
                v["line"]
                    .as_str()
                    .expect("line")
                    .contains("no result: procedure aborted")
            );
        }
        // The 180 s bound counts from the window's start: an answer at 179 s still counts.
        let plan = Plan {
            answer_at_us: 179 * SEC,
            ..Plan::default()
        };
        let (_, report) = run(Procedure::Check10, plan, None).await;
        assert_eq!(report.procedure, Ok(()));
        // Both answers fall within 180 s of the window's start: the second question does not get
        // a fresh 180 s from the first answer.
        let plan = Plan {
            answer_at_us: 100 * SEC,
            connect_answer_after_us: 81_000_000, // at 181 s
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check10, plan, None).await;
        assert!(
            matches!(&report.procedure, Err(Stop::Aborted(w)) if w.contains("Try to connect") && w.contains("within 180 s")),
            "{report:?}"
        );
        let w = writes(&sim);
        let open = event_at(&sim, crate::check10::WINDOW, crate::check10::OPEN);
        assert!(
            (open + 180 * SEC..=open + 180 * SEC + 200_000).contains(&w[w.len() - 2].0),
            "released at {} us, window opened at {open} us",
            w[w.len() - 2].0
        );
        let plan = Plan {
            answer_at_us: 181 * SEC,
            ..Plan::default()
        };
        let (sim, _) = run(Procedure::Check10, plan, None).await;
        let w = writes(&sim);
        let released = w[w.len() - 2].0;
        let open = event_at(&sim, crate::check10::WINDOW, crate::check10::OPEN);
        assert!(
            (open + 180 * SEC..=open + 180 * SEC + 200_000).contains(&released),
            "released at {released} us, window opened at {open} us"
        );
    }

    /// Every answer combination, live and offline: the connect question follows only `seen`, and
    /// only `seen` with a connect outcome gives `accepted` or `refused`.
    #[tokio::test(start_paused = true)]
    async fn check10_asks_the_connect_only_after_seen_and_follows_the_answer_table() {
        let connects = ["connected", "failed", "stuck", "not-tried"];
        for listed in ["seen", "not-seen", "unsure"] {
            for connect in connects {
                let answers: &'static [&'static str] =
                    Box::leak(vec![listed, connect].into_boxed_slice());
                let plan = Plan {
                    answers,
                    ..Plan::default()
                };
                let (sim, report) = run(Procedure::Check10, plan, None).await;
                assert_eq!(report.procedure, Ok(()), "{listed} {connect}");
                let asked: Vec<String> = check10_events(&sim)
                    .into_iter()
                    .filter_map(|(t, b)| {
                        t.starts_with("answer-")
                            .then(|| format!("{t}={}", String::from_utf8_lossy(&b)))
                    })
                    .collect();
                let v = reanalysed(&sim, &report, Procedure::Check10, "check10");
                let second = v["second_central"].as_str().expect("second central");
                if listed == "seen" {
                    assert_eq!(
                        asked,
                        [
                            "answer-listed=seen".to_string(),
                            format!("answer-connect={connect}")
                        ]
                    );
                    let want = match connect {
                        "connected" => "accepted: the phone connected while hil_fec held the link",
                        "failed" => "refused: the phone's connect failed",
                        "stuck" => "refused: the phone's connect never completed",
                        _ => "unknown: the phone listed the NEO but did not try to connect",
                    };
                    assert_eq!(second, want);
                } else {
                    assert_eq!(
                        asked,
                        [format!("answer-listed={listed}")],
                        "no connect question"
                    );
                    assert!(
                        second.starts_with("unknown: the phone did not find the NEO")
                            || second.starts_with("unknown: the operator was unsure"),
                        "{listed}: {second}"
                    );
                }
            }
        }
    }

    /// Advertising, live and offline: "not seen" needs the phone's answer and a working scan over
    /// the whole window; otherwise it is "unknown" and says what is missing.
    #[tokio::test(start_paused = true)]
    async fn check10_says_what_was_observed_and_unknown_when_nothing_was() {
        let quiet = Plan {
            advertisers: crate::sim::Advertisers::Others,
            ..Plan::default()
        };
        for (plan, advertising) in [
            (
                Plan {
                    answers: &["unsure"],
                    ..quiet
                },
                "unknown: the phone's answer is \"unsure\", and the scan saw no advertisement of the trainer (the scan ran over the whole window, 59 of other peripherals;",
            ),
            (
                Plan {
                    answers: &["unsure"],
                    scan_ends_at_us: Some(30 * SEC),
                    ..quiet
                },
                "unknown: the phone's answer is \"unsure\", and the scan saw no advertisement of the trainer (the scan ended by itself at 30 s",
            ),
            (
                Plan {
                    answers: &["seen", "connected"],
                    ..quiet
                },
                "continues: the phone listed it; the scan saw none (the scan ran over the whole window)",
            ),
            (
                Plan {
                    answers: &["not-seen"],
                    ..quiet
                },
                "not seen: the phone did not list it, and the scan, which ran over the whole window and saw 59 advertisements of other peripherals, saw none of the trainer",
            ),
            (
                Plan {
                    answers: &["not-seen"],
                    scan_ends_at_us: Some(30 * SEC),
                    ..quiet
                },
                "unknown: the phone did not list it, but the scan ended by itself at 30 s",
            ),
            (
                Plan {
                    answers: &["not-seen"],
                    advertisers: crate::sim::Advertisers::Nobody,
                    ..quiet
                },
                "unknown: the phone did not list it, but the scan ran over the whole window and saw 0 advertisements of other peripherals",
            ),
            (
                Plan {
                    answers: &["not-seen"],
                    ..Plan::default()
                },
                "continues: the scan saw 6",
            ),
        ] {
            let (sim, report) = run(Procedure::Check10, plan, None).await;
            assert_eq!(report.procedure, Ok(()));
            let v = reanalysed(&sim, &report, Procedure::Check10, "check10");
            assert!(
                v["advertising"]
                    .as_str()
                    .is_some_and(|a| a.starts_with(advertising)),
                "{advertising}: {v}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn check10_a_dropped_link_fails_with_its_cause_and_no_verdict() {
        let plan = Plan {
            drop_at_us: Some(30 * SEC),
            ..Plan::default()
        };
        let (sim, report) = run(Procedure::Check10, plan, None).await;
        assert_eq!(
            report.procedure,
            Err(Stop::Failed(
                "link dropped while the phone was trying to connect (second central)".into()
            ))
        );
        assert_eq!(report.release_error.as_deref(), Some("not connected"));
        let v = reanalysed(&sim, &report, Procedure::Check10, "check10");
        assert_eq!(v["verdict"], Value::Null, "{v}");
    }

    #[tokio::test(start_paused = true)]
    async fn check10_the_guards_run_while_waiting_for_an_answer_even_with_a_busy_scan() {
        // 100 advertisements a second, no answer yet, and the trainer goes silent at 30 s.
        let plan = Plan {
            advert_every_us: 10_000,
            answer_at_us: 150 * SEC,
            silent_from_us: Some(30 * SEC),
            ..Plan::default()
        };
        let (sim, report) = bounded(Procedure::Check10, plan).await;
        let report = report.expect("drive returns");
        assert_eq!(
            report.procedure,
            Err(Stop::Aborted(
                "no page 25 for 2 s while the phone was trying to connect: the second central may have taken the trainer (ADR-0002 §5, §6)".into()
            ))
        );
        let w = writes(&sim);
        assert!(
            w[w.len() - 2].0 <= 32_500_000,
            "released 2 s into the silence, {w:?}"
        );
    }

    /// Check 11b in the sim with a scripted dropout, driven like `run`.
    async fn run11b(plan: Plan, d: crate::sim::Dropout) -> (Sim, Report) {
        let sim = Sim::new(plan).with_dropout(d);
        let report = drive(&sim, &sim, Procedure::Check11b, std::future::pending()).await;
        (sim, report)
    }

    /// The link stamps `(code, at_us)` of a run.
    fn link_stamps(sim: &Sim) -> Vec<(u8, u64)> {
        sim.notes()
            .into_iter()
            .filter(|n| n.tag == crate::check11::LINK)
            .map(|n| (n.bytes[0], n.at_us))
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn check11b_drops_reconnects_releases_first_and_asks_before_control() {
        use crate::check11::{DETECTED, FIRST_PAGE25};
        let plan = Plan {
            answers: &["out"],
            ..Plan::default()
        };
        let (sim, report) = run11b(plan, crate::sim::Dropout::default()).await;
        assert_eq!(report.procedure, Ok(()));
        assert_released(&sim, &report);
        let link = link_stamps(&sim);
        assert_eq!(
            link.iter().map(|l| l.0).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
        let at = |code| crate::frames::stamp(&link, code).expect("stamped");
        // Woken at the drop, it boots 4 s later; connect 0.7 s, discovery 1.2 s.
        let rel = |code| at(code) - at(DETECTED);
        assert!((4 * SEC..=4 * SEC + 500_000).contains(&rel(2)), "{link:?}");
        assert_eq!(rel(3) - rel(2), 700_000, "{link:?}");
        assert_eq!(rel(4) - rel(3), 1_200_000, "{link:?}");
        assert!(rel(5) - rel(4) <= 400_000, "{link:?}");
        // Nothing was written from the stop until the reconnect; then the release first, and
        // ERG only after the lights answer and the second `y`.
        let stop = event_at(&sim, crate::check11::WINDOW, crate::check11::STOPPING);
        // Released before the stop cue (as §7.4 would), after the last ERG target.
        let before: Vec<(u64, Control)> = controls(&sim.records())
            .into_iter()
            .filter(|(t, _)| *t <= stop)
            .collect();
        assert_eq!(
            before.last().map(|c| c.1),
            Some(Control::Resistance(0.0)),
            "{before:?}"
        );
        assert!(matches!(before[before.len() - 2].1, Control::Erg(w) if (w - 100.0).abs() < 0.5));
        let after: Vec<(u64, u8)> = writes(&sim)
            .into_iter()
            .filter(|(t, _)| *t > stop)
            .collect();
        assert!(after[0].0 >= at(FIRST_PAGE25), "{after:?}");
        assert_eq!(
            controls(&sim.records())
                .into_iter()
                .find(|(t, _)| *t >= at(FIRST_PAGE25))
                .map(|c| c.1),
            Some(Control::Resistance(0.0))
        );
        let answered = sim
            .notes()
            .iter()
            .find(|n| n.tag == "answer-lights")
            .expect("answered")
            .at_us;
        let first_erg = controls(&sim.records())
            .into_iter()
            .find(|(t, c)| *t > stop && matches!(c, Control::Erg(_)))
            .map(|c| c.0)
            .expect("re-engaged");
        assert!(first_erg >= answered, "{first_erg} {answered}");
        let y = event_at(&sim, crate::check11::WINDOW, crate::check11::CONFIRMED);
        assert!(
            (answered..=first_erg).contains(&y),
            "the y is stamped before any ERG"
        );
        let v = reanalysed(&sim, &report, Procedure::Check11b, "check11b");
        assert_eq!(v["cause"], json!("power-off (the lights went out)"));
        let reconnect = v["reconnect"].as_str().expect("reconnect");
        assert!(
            reconnect.contains("connected +4.") && reconnect.contains("discovered +"),
            "{v}"
        );
        let held = v["after_reconnect"]["erg_mean_power_w"]
            .as_f64()
            .expect("held");
        assert!((95.0..=105.0).contains(&held), "{v}");
        assert!(
            v["line"].as_str().expect("line").starts_with(
                "| 11b | Dropout: unplugged stop, reconnect | dropout power-off (the lights went out); detected "
            ),
            "{v}"
        );
    }

    /// The lights answer decides the cause, live and offline.
    #[tokio::test(start_paused = true)]
    async fn check11b_the_cause_follows_the_lights_answer() {
        for (answers, cause) in [
            (&["out"][..], "power-off (the lights went out)"),
            (
                &["on"][..],
                "unknown: the link dropped with the lights still on, not a power-off",
            ),
            (
                &["unsure"][..],
                "unknown: the operator was unsure about the lights",
            ),
        ] {
            let plan = Plan {
                answers,
                ..Plan::default()
            };
            let (sim, report) = run11b(plan, crate::sim::Dropout::default()).await;
            assert_eq!(report.procedure, Ok(()));
            let v = reanalysed(&sim, &report, Procedure::Check11b, "check11b");
            assert_eq!(v["cause"], json!(cause));
        }
        // No answer: the run stops released, with no verdict.
        let plan = Plan {
            answers: &[],
            ..Plan::default()
        };
        let (sim, report) = run11b(plan, crate::sim::Dropout::default()).await;
        assert!(
            matches!(&report.procedure, Err(Stop::Aborted(w)) if w.starts_with("the input closed before an answer to \"When the link dropped")),
            "{report:?}"
        );
        assert_released(&sim, &report);
        let v = reanalysed(&sim, &report, Procedure::Check11b, "check11b");
        assert_eq!((&v["verdict"], &v["cause"]), (&Value::Null, &Value::Null));
    }

    #[tokio::test(start_paused = true)]
    async fn check11b_with_no_reconnect_in_60_s_completes_with_none() {
        let plan = Plan {
            answers: &["out"],
            ..Plan::default()
        };
        let d = crate::sim::Dropout {
            boots_after_us: None,
            ..crate::sim::Dropout::default()
        };
        let (sim, report) = run11b(plan, d).await;
        assert_eq!(report.procedure, Ok(()));
        assert_eq!(report.release_error.as_deref(), Some("not connected"));
        let link = link_stamps(&sim);
        assert_eq!(link.iter().map(|l| l.0).collect::<Vec<_>>(), [1, 6]);
        assert!(
            (60 * SEC..=60 * SEC + 100_000).contains(&(link[1].1 - link[0].1)),
            "{link:?}"
        );
        let v = reanalysed(&sim, &report, Procedure::Check11b, "check11b");
        assert_eq!(
            v["reconnect"],
            json!(
                "none within 60 s of the drop (seen unknown, connected unknown, discovered unknown)"
            )
        );
        assert_eq!(v["after_reconnect"], Value::Null);
        assert_eq!(v["cause"], json!("power-off (the lights went out)"));
    }

    /// A reconnect that fails (here the scan ends) fails the run with its cause, stamped 07.
    #[tokio::test(start_paused = true)]
    async fn check11b_a_failed_reconnect_fails_with_its_cause() {
        let plan = Plan {
            answers: &["out"],
            scan_ends_at_us: Some(1),
            ..Plan::default()
        };
        let (sim, report) = run11b(plan, crate::sim::Dropout::default()).await;
        assert_eq!(
            report.procedure,
            Err(Stop::Failed(
                "the scan ended before the trainer advertised again".into()
            ))
        );
        assert_eq!(
            link_stamps(&sim).iter().map(|l| l.0).collect::<Vec<_>>(),
            [1, 7]
        );
        let v = reanalysed(&sim, &report, Procedure::Check11b, "check11b");
        assert_eq!(v["verdict"], Value::Null);
    }

    /// Another unit answering after the reconnect aborts the run, released, before any control.
    #[tokio::test(start_paused = true)]
    async fn check11b_another_trainer_after_the_reconnect_aborts_released() {
        let plan = Plan {
            answers: &["out"],
            ..Plan::default()
        };
        let d = crate::sim::Dropout {
            other_trainer: true,
            ..crate::sim::Dropout::default()
        };
        let (sim, report) = run11b(plan, d).await;
        assert_eq!(
            report.procedure,
            Err(Stop::Aborted(
                "a different trainer answered after the reconnect (page-81 serials differ)".into()
            ))
        );
        assert_released(&sim, &report);
        let back = crate::frames::stamp(&link_stamps(&sim), crate::check11::FIRST_PAGE25)
            .expect("reconnected");
        assert!(
            controls(&sim.records())
                .iter()
                .filter(|(t, _)| *t >= back)
                .all(|(_, c)| *c == Control::Resistance(0.0))
        );
        let v = reanalysed(&sim, &report, Procedure::Check11b, "check11b");
        assert_eq!(v["verdict"], Value::Null);
    }

    #[tokio::test(start_paused = true)]
    async fn check11b_without_a_drop_in_90_s_aborts_released_with_no_verdict() {
        // Plugged in: the trainer never powers off.
        let (sim, report) = run(Procedure::Check11b, Plan::default(), None).await;
        assert!(
            matches!(&report.procedure, Err(Stop::Aborted(w)) if w == "the link did not drop within 90 s of the stop cue (is the trainer unplugged?)"),
            "{report:?}"
        );
        assert_released(&sim, &report);
        let stop = event_at(&sim, crate::check11::WINDOW, crate::check11::STOPPING);
        let w = writes(&sim);
        assert!(
            (stop + 90 * SEC..=stop + 90 * SEC + 200_000).contains(&w[w.len() - 2].0),
            "released at {} us, stop cue at {stop} us",
            w[w.len() - 2].0
        );
        let v = reanalysed(&sim, &report, Procedure::Check11b, "check11b");
        assert_eq!(v["verdict"], Value::Null, "{v}");
    }

    #[tokio::test(start_paused = true)]
    async fn check11b_needs_a_second_y_before_any_control_after_the_reconnect() {
        let plan = Plan {
            answers: &["out"],
            ..Plan::default()
        };
        let d = crate::sim::Dropout {
            confirms_again: false,
            ..crate::sim::Dropout::default()
        };
        let (sim, report) = run11b(plan, d).await;
        assert_eq!(
            report.procedure,
            Err(Stop::Aborted("not confirmed after the reconnect".into()))
        );
        assert_released(&sim, &report);
        let back = crate::frames::stamp(&link_stamps(&sim), crate::check11::FIRST_PAGE25)
            .expect("reconnected");
        assert!(
            controls(&sim.records())
                .iter()
                .filter(|(t, _)| *t >= back)
                .all(|(_, c)| *c == Control::Resistance(0.0)),
            "only the release after the reconnect"
        );
        let v = reanalysed(&sim, &report, Procedure::Check11b, "check11b");
        assert_eq!(v["verdict"], Value::Null);
    }

    #[tokio::test(start_paused = true)]
    async fn check11b_the_guards_watch_the_new_link() {
        let plan = Plan {
            answers: &["out"],
            ..Plan::default()
        };
        let (sim, _) = run11b(plan, crate::sim::Dropout::default()).await;
        let back = crate::frames::stamp(&link_stamps(&sim), crate::check11::FIRST_PAGE25)
            .expect("reconnected");
        // The same run, but the reconnected trainer goes silent 3 s after its first page 25.
        let silent = back + 3 * SEC;
        let plan = Plan {
            silent_from_us: Some(silent),
            ..plan
        };
        let (sim, report) = run11b(plan, crate::sim::Dropout::default()).await;
        assert_eq!(report.procedure, Err(Stop::Aborted(SILENT.into())));
        let w = writes(&sim);
        let released = w[w.len() - 2].0;
        assert!(
            (silent..=silent + 2_500_000).contains(&released),
            "released 2 s into the silence: {released} us, silent from {silent} us"
        );
    }

    #[tokio::test]
    async fn a_question_takes_only_an_accepted_answer() {
        const OPTIONS: &[&str] = &["seen", "not-seen"];
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        for line in ["y", "SEEN maybe", " Not-Seen "] {
            tx.send(line.to_string()).await.expect("send");
        }
        drop(tx); // a console that skipped the accepted answer would see closed input, not hang
        assert_eq!(
            Console::from_lines(rx).ask("q?", OPTIONS).await,
            Some("not-seen")
        );
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send("yes".to_string()).await.expect("send");
        drop(tx);
        assert_eq!(
            Console::from_lines(rx).ask("q?", OPTIONS).await,
            None,
            "closed input"
        );
    }

    #[tokio::test]
    async fn confirmation_needs_an_explicit_yes() {
        for (line, ok) in [
            ("", false),
            ("n", false),
            ("yes please", false),
            ("y", true),
            (" YES ", true),
        ] {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            tx.send(line.to_string()).await.expect("send");
            assert_eq!(Console::from_lines(rx).confirm("go?").await, ok, "{line:?}");
        }
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(1);
        drop(tx);
        assert!(
            !Console::from_lines(rx).confirm("go?").await,
            "closed input"
        );
    }
}
