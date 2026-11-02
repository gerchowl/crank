//! The Part B control checks with a rider on the trainer (#44 checks 6, 7, 12; #412).
//!
//! Safety (ADR-0002 §7): every run connects into page 48 at 0 %, the release state; ERG is
//! engaged as the engine does at a ride start (§7.5: 0 W until the first sample with cadence,
//! then a 5 s ramp in 1 Hz steps from the 3 s power); targets never exceed [`MAX_ERG_W`] and
//! grades [`MAX_GRADE_PCT`]; while ERG is applied the §7.4 release rule (cadence below
//! 40 rpm for 2 s) and a 2 s telemetry silence (§6) stop the procedure. Whatever ends it
//! (completion, a guard, an error, a panic, SIGINT/SIGTERM/SIGHUP) the trainer is
//! released with page 48 at 0 %, the ride-end mode of §7.9, before the link is closed.
use crate::{
    frames::{Record, page_of},
    partb::Sample,
    session::Session,
};
use crank_engine::{
    control::SOFT_START_TICKS,
    erg::policy::{RELEASE_RPM, RELEASE_TICKS},
    ticks::TICKS_PER_SECOND,
};
use crank_fec::{ControlPage, encode};
use futures_util::FutureExt;
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
const HOLD: Duration = Duration::from_secs(20);
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
}

/// An instruction to the rider; the agent relays it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// Start pedalling at about 90 rpm.
    Start,
    /// Hold about this cadence.
    Cadence(u8),
    /// The run is over and the trainer released.
    Stop,
}

impl Cue {
    /// Text shown to the operator.
    pub fn text(self) -> String {
        match self {
            Self::Start => "start pedalling at about 90 rpm".into(),
            Self::Cadence(rpm) => format!("hold about {rpm} rpm, steady"),
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
        Self(tokio::sync::Mutex::new(rx))
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
            "{text}\nPress Enter to start (Ctrl-C at any time releases the trainer and stops)."
        ));
        self.0.lock().await.recv().await.is_some()
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
    /// No ERG target applied: silence and link only.
    Idle,
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
            guard: Guard::Idle,
            seen: w.mark(),
            last_sample_us: w.now_us(),
            low_since: None,
            recent: Vec::new(),
        }
    }

    fn cue(&self, cue: Cue) {
        self.op.cue(self.w.now_us(), cue);
    }

    async fn write(&self, page: &ControlPage) -> Result<(), Stop> {
        match self.w.send(frame(page)?).await {
            None => Ok(()),
            Some(e) => Err(Stop::Failed(format!("write {page:?}: {e}"))),
        }
    }

    async fn erg(&self, watts: u16) -> Result<(), Stop> {
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

    async fn track(&self, grade_pct: f32) -> Result<(), Stop> {
        if grade_pct.abs() > MAX_GRADE_PCT {
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

    /// Take new page-25 samples and apply the guard.
    fn observe(&mut self) -> Result<(), Stop> {
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
            let since = self.low_since.map_or(0, |t| s.at_us - t);
            if self.guard == Guard::Erg && since >= RELEASE_TICKS * SEC / TICKS_PER_SECOND {
                return Err(Stop::Aborted(format!(
                    "cadence below {RELEASE_RPM} rpm for 2 s under ERG (ADR-0002 §7.4)"
                )));
            }
        }
        if self.w.now_us().saturating_sub(self.last_sample_us) > SILENCE_US {
            return Err(Stop::Aborted("no page 25 for 2 s (ADR-0002 §6)".into()));
        }
        Ok(())
    }

    /// Ride for `dur` under the guard.
    async fn hold(&mut self, dur: Duration) -> Result<(), Stop> {
        let end = Instant::now() + dur;
        while Instant::now() < end {
            self.observe()?;
            sleep(POLL.min(end - Instant::now())).await;
        }
        self.observe()
    }

    /// Wait for the first sample with cadence > 0 (no guard: nothing is applied yet).
    async fn wait_pedalling(&mut self) -> Result<(), Stop> {
        self.cue(Cue::Start);
        let guard = std::mem::replace(&mut self.guard, Guard::Idle);
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
        self.guard = Guard::Idle;
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
    r.engage(150).await?;
    r.cue(Cue::Cadence(90));
    r.hold(Duration::from_secs(10)).await?;
    r.hold(Duration::from_secs(30)).await?;
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

/// The Part B control checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Procedure {
    /// #44 check 6.
    Check6,
    /// #44 check 7.
    Check7,
    /// #44 check 12.
    Check12,
}

impl Procedure {
    /// What the procedure writes, for the summary.
    pub fn what(self) -> &'static str {
        match self {
            Self::Check6 => {
                "page 48 0 %; page 49 0 W until cadence, 5 s soft start to 100 W; 100/150/250/100/200 W held 20 s; at 200 W cues 60 then 90 rpm, 20 s each; release"
            }
            Self::Check7 => {
                "page 48 0 %; soft start to 150 W; 10 s; 30 s with no write at all; release"
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
    let release = || async {
        match frame(&ControlPage::BasicResistance { pct: 0.0 }) {
            Ok(f) => tokio::time::timeout(RELEASE_WAIT, w.send(f))
                .await
                .unwrap_or_else(|_| Some("release write not completed within 2 s".into())),
            Err(e) => Some(format!("{e:?}")),
        }
    };
    if let Some(e) = release().await {
        return Report {
            procedure: Err(Stop::Failed(format!("initial page 48: {e}"))),
            release_error: Some(e),
        };
    }
    let run = async {
        if !op.confirm(&format!("hil_fec {p:?}: {}", p.what())).await {
            return Err(Stop::Aborted("not confirmed by the operator".into()));
        }
        let mut r = Rig::new(w, op);
        match p {
            Procedure::Check6 => check6(&mut r).await,
            Procedure::Check7 => check7(&mut r).await,
            Procedure::Check12 => check12(&mut r).await,
        }
    };
    let procedure = tokio::select! {
        r = AssertUnwindSafe(run).catch_unwind() => r.unwrap_or_else(|_| Err(Stop::Failed("procedure panicked".into()))),
        sig = interrupt => Err(Stop::Interrupted(sig)),
    };
    let release_error = release().await;
    // Leave the command status of the release in the capture (evidence only).
    if let Ok(f) = frame(&ControlPage::Request { page: 71, times: 1 }) {
        sleep(Duration::from_millis(300)).await;
        let _ = w.send(f).await;
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

/// The summary of one driven run (live or simulated).
pub fn summary(p: Procedure, stem: &str, report: &Report, records: &[Record]) -> Value {
    let mut v = json!({
        "check": stem,
        "what_was_sent": p.what(),
        "procedure": match &report.procedure {
            Ok(()) => json!("completed"),
            Err(Stop::Aborted(why)) => json!({"aborted": why}),
            Err(Stop::Interrupted(sig)) => json!({"interrupted": sig}),
            Err(Stop::Failed(why)) => json!({"failed": why}),
        },
        "release_write_error": report.release_error,
        "page71_frames": records.iter().filter(|r| r.rx && page_of(&r.bytes) == Some(71)).count(),
    });
    v["result"] = crate::partb::analyse(stem, records).unwrap_or(Value::Null);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        frames::{hex, parse_capture},
        partb::{Control, analyse, controls},
        sim::{Plan, Sim},
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

    /// A summary re-derived offline from the written `.fecap` text equals the live one.
    fn reanalysed(sim: &Sim, stem: &str) -> Value {
        let text = sim.records().iter().fold(String::new(), |mut t, r| {
            let dir = if r.rx { "rx" } else { "tx" };
            let _ = writeln!(t, "{} {dir} {}", r.at_us, hex(&r.bytes));
            t
        });
        let live = analyse(stem, &sim.records()).expect("part B stem");
        let offline = analyse(stem, &parse_capture(&text)).expect("part B stem");
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
        let v = reanalysed(&sim, "check6");
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
        let v = reanalysed(&sim, "check7");
        assert!(v["silent_s"].as_f64().expect("silent") >= 40.0, "{v}");
        assert_eq!(
            (v["target_w"].as_f64(), &v["held"]),
            (Some(150.0), &json!(true))
        );
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
        let v = reanalysed(&sim, "check12");
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
        let (sim, report) = run(Procedure::Check6, Plan::default(), Some(20)).await;
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
        let r = Rig::new(&sim, &sim);
        assert!(matches!(r.erg(MAX_ERG_W + 1).await, Err(Stop::Failed(_))));
        assert!(matches!(
            r.track(MAX_GRADE_PCT + 0.5).await,
            Err(Stop::Failed(_))
        ));
        assert!(sim.records().iter().all(|x| x.rx), "nothing written");
    }
}
