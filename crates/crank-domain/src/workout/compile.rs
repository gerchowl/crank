//! Frozen TS compiler port, ADR-0001 §3. Resource/finite-arithmetic divergences:
//! <https://github.com/gerchowl/crank/issues/160>.
use super::{
    Athlete, CompileError, CompileOptions, CompiledWorkout, FlatBlock, MAX_NESTING_DEPTH, Offset,
    Phase, PowerTarget, Severity, StepEnd, StepNode, ValidationIssue, ValidationKind, Workout,
    WorkoutNode,
};
use crate::js_round;

/// Reference trainer ceiling, watts.
pub const TRAINER_MAX_W: f64 = 2200.;
/// Expected manual-lap duration, seconds.
pub const LAP_EXPECTED_S: f64 = 300.;
/// Manual-lap duration range, seconds.
pub const LAP_RANGE_S: [f64; 2] = [30., 1800.];
/// Work intensity threshold, percentage points of FTP.
pub const WORK_THRESHOLD_PCT: f64 = 80.;
const MAX_VISITS: u32 = 100_000;
const MAX_SECONDS: u32 = 604_800;

fn finite(x: f64) -> Result<f64, CompileError> {
    if x.is_finite() {
        Ok(x)
    } else {
        Err(CompileError::InvalidArithmetic)
    }
}
fn input_finite(x: f64) -> Result<f64, CompileError> {
    finite(x).map_err(|_| CompileError::NonFinite)
}
// JavaScript Math.max propagates NaN; f64::max ignores it.
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}
// Number-to-string conversion in JS discards the sign of zero.
fn display_number(value: f64) -> String {
    if value == 0. {
        "0".into()
    } else {
        value.to_string()
    }
}
// Compiler-only subset of athlete.ts; the wider athlete API belongs to #136.
fn power_for_hr(bpm: f64, ftp: f64, a: &Athlete) -> f64 {
    js_max(
        ((bpm - (a.rest_hr + 12.)) / (js_round(a.max_hr * 0.9) - (a.rest_hr + 12.))) * ftp,
        0.,
    )
}
fn hr_steady(power: f64, ftp: f64, a: &Athlete) -> f64 {
    let raw =
        a.rest_hr + 12. + (js_round(a.max_hr * 0.9) - (a.rest_hr + 12.)) * (power.max(0.) / ftp);
    let knee = a.max_hr - 8.;
    if raw <= knee {
        raw
    } else {
        knee + 8. * (1. - (-(raw - knee) / 8.).exp())
    }
}
fn athlete_finite(a: &Athlete) -> Result<(), CompileError> {
    for x in [a.ftp, a.weight_kg, a.max_hr, a.rest_hr] {
        input_finite(x)?;
    }
    Ok(())
}
fn target_finite(t: &PowerTarget) -> Result<(), CompileError> {
    let values = match *t {
        PowerTarget::PctFtp { value } | PowerTarget::Watts { value } => [value, 0.],
        PowerTarget::Range { lo, hi } => [lo, hi],
        PowerTarget::Ramp { from, to } => [from, to],
        PowerTarget::Hr { bpm } => [bpm, 0.],
        PowerTarget::Free => [0., 0.],
    };
    for x in values {
        input_finite(x)?;
    }
    Ok(())
}
/// Representative intensity, % FTP; ramps use their maximum, ranges their midpoint.
/// HR inversion uses the supplied athlete. NaN/Infinity results from finite inputs
/// are retained for JavaScript-compatible intensity comparisons (e.g. zero FTP).
///
/// # Errors
/// Nonfinite input parameters return [`CompileError::NonFinite`].
pub fn target_pct(t: &PowerTarget, ftp: f64, athlete: &Athlete) -> Result<f64, CompileError> {
    target_finite(t)?;
    input_finite(ftp)?;
    athlete_finite(athlete)?;
    Ok(match *t {
        PowerTarget::PctFtp { value } => value,
        PowerTarget::Watts { value } => value / ftp * 100.,
        PowerTarget::Range { lo, hi } => {
            // Retain the TS addition order, including overflow used only in comparisons.
            let sum = lo + hi;
            sum / 2.
        }
        PowerTarget::Ramp { from, to } => from.max(to),
        PowerTarget::Hr { bpm } => power_for_hr(bpm, ftp, athlete) / ftp * 100.,
        PowerTarget::Free => 50.,
    })
}
/// Start/end target watts with progression; HR and free targets ignore offsets.
///
/// # Errors
/// Nonfinite inputs/results return a typed error (see issue #160).
pub fn target_watts(
    t: &PowerTarget,
    ftp: f64,
    off: Offset,
    athlete: &Athlete,
) -> Result<[f64; 2], CompileError> {
    target_finite(t)?;
    for x in [ftp, off.watts, off.pct] {
        input_finite(x)?;
    }
    athlete_finite(athlete)?;
    let w = |p: f64| -> Result<f64, CompileError> {
        Ok(finite(((p + off.pct) / 100.) * ftp + off.watts)?.max(0.))
    };
    let pair = match *t {
        PowerTarget::PctFtp { value } => [w(value)?, w(value)?],
        PowerTarget::Watts { value } => {
            let v = finite(value + off.watts + (off.pct / 100.) * ftp)?.max(0.);
            [v, v]
        }
        PowerTarget::Range { lo, hi } => {
            let v = w(finite(lo + hi)? / 2.)?;
            [v, v]
        }
        PowerTarget::Ramp { from, to } => [w(from)?, w(to)?],
        PowerTarget::Hr { bpm } => {
            let v = finite(power_for_hr(bpm, ftp, athlete))?;
            [v, v]
        }
        PowerTarget::Free => [0.5 * ftp, 0.5 * ftp],
    };
    for x in pair {
        finite(x)?;
    }
    Ok(pair)
}
/// Work classification with the reference default athlete and 80% FTP threshold.
/// HR, free and HR-recovery steps never receive progression.
///
/// # Errors
/// Nonfinite input parameters return a typed error; NaN intensity compares false.
pub fn is_work_step(s: &StepNode, ftp: f64) -> Result<bool, CompileError> {
    Ok(!matches!(s.end, StepEnd::HrRecovery { .. })
        && !matches!(s.target, PowerTarget::Free | PowerTarget::Hr { .. })
        && target_pct(&s.target, ftp, &Athlete::default())? >= WORK_THRESHOLD_PCT)
}
fn issue(
    issues: &mut Vec<ValidationIssue>,
    kind: ValidationKind,
    severity: Severity,
    id: Option<&str>,
    message: String,
) {
    issues.push(ValidationIssue {
        kind,
        severity,
        node_id: id.map(str::to_owned),
        message,
    });
}
struct Raw<'a> {
    step: &'a StepNode,
    path: Vec<usize>,
    iteration: Vec<usize>,
    off: Offset,
}
struct Flatten<'a> {
    raws: Vec<Raw<'a>>,
    issues: Vec<ValidationIssue>,
    visits: u32,
    ftp: f64,
}
impl<'a> Flatten<'a> {
    fn visit(
        &mut self,
        n: &'a WorkoutNode,
        path: Vec<usize>,
        iteration: Vec<usize>,
        off: Offset,
    ) -> Result<(), CompileError> {
        self.visits += 1;
        if self.visits > MAX_VISITS {
            return Err(CompileError::ExpansionLimit);
        }
        match n {
            WorkoutNode::Step(s) => self.raws.push(Raw {
                step: s,
                path,
                iteration,
                off: if is_work_step(s, self.ftp)? {
                    off
                } else {
                    Offset::default()
                },
            }),
            WorkoutNode::Repeat(r) => {
                if iteration.len() >= MAX_NESTING_DEPTH {
                    return Err(CompileError::NestingTooDeep);
                }
                input_finite(r.count)?;
                if r.count.fract() != 0. || r.count < 1. {
                    issue(
                        &mut self.issues,
                        ValidationKind::InvalidRepeatCount,
                        Severity::Error,
                        Some(&r.id),
                        format!(
                            "Repeat count must be a whole number ≥ 1 (got {})",
                            display_number(r.count)
                        ),
                    );
                    return Ok(());
                }
                if r.children.is_empty() {
                    issue(
                        &mut self.issues,
                        ValidationKind::EmptyRepeat,
                        Severity::Error,
                        Some(&r.id),
                        "Repeat has no steps".into(),
                    );
                    return Ok(());
                }
                if r.count > f64::from(MAX_VISITS) {
                    return Err(CompileError::ExpansionLimit);
                }
                let progression = r.progression.clone().unwrap_or_default();
                let watts = input_finite(progression.watts_per_rep.unwrap_or(0.))?;
                let pct = input_finite(progression.pct_per_rep.unwrap_or(0.))?;
                // Bounded conversion via iteration avoids unchecked float-to-integer casts.
                let mut rep = 0u32;
                while f64::from(rep) < r.count {
                    let next = Offset {
                        watts: finite(off.watts + f64::from(rep) * watts)?,
                        pct: finite(off.pct + f64::from(rep) * pct)?,
                    };
                    for (ci, c) in r.children.iter().enumerate() {
                        if f64::from(rep) >= r.count - 1.
                            && r.skip_last_rest == Some(true)
                            && r.children.len() > 1
                            && ci == r.children.len() - 1
                        {
                            continue;
                        }
                        let mut p = path.clone();
                        p.push(ci);
                        let mut it = iteration.clone();
                        it.push(rep as usize);
                        self.visit(c, p, it, next)?;
                    }
                    rep += 1;
                }
            }
        }
        Ok(())
    }
}
fn phases(raws: &[Raw<'_>], ftp: f64) -> Result<Vec<Phase>, CompileError> {
    let mut work = raws
        .iter()
        .map(|r| is_work_step(r.step, ftp))
        .collect::<Result<Vec<_>, _>>()?;
    if !work.contains(&true) && !raws.is_empty() {
        let pcts = raws
            .iter()
            .map(|r| target_pct(&r.step.target, ftp, &Athlete::default()))
            .collect::<Result<Vec<_>, _>>()?;
        let max = pcts.iter().copied().fold(f64::NEG_INFINITY, js_max);
        for (i, p) in pcts.iter().enumerate() {
            work[i] = *p >= max * 0.95 && !matches!(raws[i].step.target, PowerTarget::Free);
        }
    }
    let first = work.iter().position(|w| *w);
    let last = work.iter().rposition(|w| *w);
    Ok(raws
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let label = r.step.label.as_deref().unwrap_or("").to_lowercase();
            if matches!(r.step.target, PowerTarget::Free) {
                Phase::Free
            } else if label.contains("warm") {
                Phase::Warmup
            } else if label.contains("cool") {
                Phase::Cooldown
            } else if matches!(r.step.end, StepEnd::HrRecovery { .. }) {
                Phase::Recovery
            } else if work[i] {
                Phase::Work
            } else if first.is_some_and(|f| i < f) {
                Phase::Warmup
            } else if last.is_some_and(|l| i > l) {
                Phase::Cooldown
            } else {
                Phase::Recovery
            }
        })
        .collect())
}
struct Timeline<'a> {
    ftp: f64,
    athlete: &'a Athlete,
    tau: f64,
    hr: f64,
    total: f64,
    range: [f64; 2],
    issues: Vec<ValidationIssue>,
}
impl Timeline<'_> {
    fn diagnostic(
        &mut self,
        kind: ValidationKind,
        severity: Severity,
        s: &StepNode,
        message: String,
    ) {
        issue(&mut self.issues, kind, severity, Some(&s.id), message);
    }
    fn duration(
        &mut self,
        s: &StepNode,
        pf: f64,
        pt: f64,
    ) -> Result<(f64, Option<[f64; 2]>), CompileError> {
        let mut range = None;
        let dur = match s.end {
            StepEnd::Time { s: seconds } => {
                input_finite(seconds)?;
                if seconds <= 0. {
                    self.diagnostic(
                        ValidationKind::InvalidDuration,
                        Severity::Error,
                        s,
                        "Duration must be > 0".into(),
                    );
                }
                seconds
            }
            StepEnd::Kj { kj } => {
                input_finite(kj)?;
                if kj <= 0. {
                    self.diagnostic(
                        ValidationKind::InvalidKilojoules,
                        Severity::Error,
                        s,
                        "kJ target must be > 0".into(),
                    );
                }
                finite(kj * 1000.)? / (finite(pf + pt)? / 2.).max(1.)
            }
            StepEnd::Lap => {
                range = Some(LAP_RANGE_S);
                LAP_EXPECTED_S
            }
            StepEnd::HrRecovery {
                target_bpm,
                min_s,
                max_s,
                ..
            } => {
                for x in [target_bpm, min_s, max_s] {
                    input_finite(x)?;
                }
                if min_s > max_s {
                    self.diagnostic(
                        ValidationKind::ReversedRecoveryBounds,
                        Severity::Error,
                        s,
                        "HR recovery: min duration exceeds max".into(),
                    );
                }
                if target_bpm <= self.athlete.rest_hr || target_bpm >= self.athlete.max_hr {
                    self.diagnostic(
                        ValidationKind::RecoveryTargetOutsideRange,
                        Severity::Error,
                        s,
                        format!(
                            "HR recovery target {} bpm is outside {}–{}",
                            display_number(target_bpm),
                            display_number(self.athlete.rest_hr),
                            display_number(self.athlete.max_hr)
                        ),
                    );
                }
                let hr_inf = hr_steady(pf, self.ftp, self.athlete);
                let est = if hr_inf >= target_bpm - 1. {
                    self.diagnostic(ValidationKind::UnreachableRecovery,Severity::Warning,s,format!("HR < {} is likely unreachable at {} W (steady HR ≈ {}); block will run to its max",display_number(target_bpm),display_number(js_round(pf)),display_number(js_round(hr_inf))));
                    max_s
                } else if self.hr <= target_bpm {
                    min_s
                } else {
                    self.tau * ((self.hr - hr_inf) / (target_bpm - hr_inf)).ln()
                };
                // An infinite estimate is capped by the finite bounds in TS.
                // NaN propagates through Math.min/Math.max rather than choosing a bound.
                if est.is_nan() {
                    return Err(CompileError::InvalidArithmetic);
                }
                range = Some([min_s.min(max_s), min_s.max(max_s)]);
                max_s.min(min_s.max(est))
            }
        };
        let dur = finite(dur)?.max(0.);
        if dur > f64::from(MAX_SECONDS) {
            return Err(CompileError::TraceLimit);
        }
        Ok((dur, range))
    }
    fn block(&mut self, r: Raw<'_>, phase: Phase) -> Result<FlatBlock, CompileError> {
        let s = r.step;
        let [pf, pt] = target_watts(&s.target, self.ftp, r.off, self.athlete)?;
        let avg = finite(pf + pt)? / 2.;
        let (dur, range) = self.duration(s, pf, pt)?;
        let max = TRAINER_MAX_W.min(4. * self.ftp);
        if pf.max(pt) > max {
            self.diagnostic(
                ValidationKind::ExcessivePower,
                Severity::Error,
                s,
                format!(
                    "Target {} W exceeds max {} W",
                    display_number(js_round(pf.max(pt))),
                    display_number(js_round(max))
                ),
            );
        } else if pf.max(pt) > 1.5 * self.ftp && dur > 60. {
            self.diagnostic(
                ValidationKind::LongSprint,
                Severity::Warning,
                s,
                format!(
                    "{} % FTP for {} s is beyond sprint range",
                    display_number(js_round((pf.max(pt) / self.ftp) * 100.)),
                    display_number(js_round(dur))
                ),
            );
        }
        // TS can carry a NaN HR state at zero FTP even when no HR end uses it.
        // Do not reject unused state: zero-FTP golden vectors are valid diagnostics.
        let hr_end = hr_steady(avg, self.ftp, self.athlete);
        let tau = if hr_end > self.hr { 35. } else { self.tau };
        self.hr = hr_end + (self.hr - hr_end) * (-dur / tau).exp();
        let start = self.total;
        self.total = finite(start + dur)?;
        if dur > 0. && self.total <= start {
            return Err(CompileError::TimelinePrecision);
        }
        for (i, total) in self.range.iter_mut().enumerate() {
            *total = finite(*total + range.map_or(dur, |r| r[i]))?;
        }
        Ok(FlatBlock {
            path: r.path,
            iteration: r.iteration,
            node_id: s.id.clone(),
            start_s: start,
            dur_s: dur,
            power_from: finite(pf * 10.).map(|p| js_round(p) / 10.)?,
            power_to: finite(pt * 10.).map(|p| js_round(p) / 10.)?,
            phase,
            dur_range_s: range,
        })
    }
}
/// Normalized power, watts, from a 1 Hz trace (30 s rolling fourth-power mean).
/// Short traces use their arithmetic mean.
///
/// # Errors
/// Nonfinite inputs/arithmetic or more than `u32::MAX` windows fail explicitly.
pub fn normalized_power(power: &[f64]) -> Result<f64, CompileError> {
    let mut roll = 0.;
    let mut s4 = 0.;
    let mut count = 0u32;
    for (i, p) in power.iter().enumerate() {
        input_finite(*p)?;
        roll = finite(roll + p)?;
        if i >= 30 {
            roll = finite(roll - power[i - 30])?;
        }
        if i >= 29 {
            s4 = finite(s4 + (roll / 30.).powi(4))?;
            count = count.checked_add(1).ok_or(CompileError::TraceLimit)?;
        }
    }
    if power.is_empty() {
        Ok(0.)
    } else if power.len() < 30 {
        let n = u32::try_from(power.len()).map_err(|_| CompileError::TraceLimit)?;
        finite(roll / f64::from(n))
    } else {
        finite((s4 / f64::from(count)).powf(0.25))
    }
}
fn power_trace(blocks: &[FlatBlock]) -> Result<(Vec<f64>, Vec<usize>), CompileError> {
    let mut trace = Vec::new();
    let mut owner = Vec::new();
    for (bi, b) in blocks.iter().enumerate() {
        let n = js_round(b.dur_s);
        let existing = u32::try_from(trace.len()).map_err(|_| CompileError::TraceLimit)?;
        if n + f64::from(existing) > f64::from(MAX_SECONDS) {
            return Err(CompileError::TraceLimit);
        }
        let mut k = 0u32;
        while f64::from(k) < n {
            trace.push(finite(
                b.power_from + (b.power_to - b.power_from) * ((f64::from(k) + 0.5) / n.max(1.)),
            )?);
            owner.push(bi);
            k += 1;
        }
    }
    Ok((trace, owner))
}
fn reserve_diagnostic(
    trace: &[f64],
    owner: &[usize],
    blocks: &[FlatBlock],
    ftp: f64,
    issues: &mut Vec<ValidationIssue>,
) -> Result<(), CompileError> {
    let mut reserve = 20_000.;
    let mut minimum = f64::INFINITY;
    let mut min_i = 0;
    for (i, p) in trace.iter().enumerate() {
        reserve = finite(if *p > ftp {
            reserve - (p - ftp)
        } else {
            20_000. - (20_000. - reserve) * (-(ftp - p) / 20_000.).exp()
        })?;
        if reserve < minimum {
            minimum = reserve;
            min_i = i;
        }
    }
    if minimum < 3000. {
        let pct = display_number(js_round(minimum / 20_000. * 100.));
        let (kind, severity, message) = if minimum < 0. {
            (
                ValidationKind::DepletedReserve,
                Severity::Error,
                format!("W'bal drops to {pct} % — not sustainable as prescribed"),
            )
        } else {
            (
                ValidationKind::LowReserve,
                Severity::Warning,
                format!("W'bal drops to {pct} % — very hard, little reserve"),
            )
        };
        issue(
            issues,
            kind,
            severity,
            owner
                .get(min_i)
                .and_then(|i| blocks.get(*i))
                .map(|b| b.node_id.as_str()),
            message,
        );
    }
    Ok(())
}
/// Flatten and estimate a workout using the frozen reference algorithms (ADR-0001 §3).
/// Invalid prescriptions remain typed diagnostics; unsafe arithmetic/size returns an error.
/// Limits: 32 enclosing repeats, 100,000 expanded node visits (each visit to a repeat
/// or step counts once, including revisits of the same source node; skipped children
/// are not visited), 604,800 duration seconds per block and
/// rounded trace samples overall. Finite-input divergences, including precision loss:
/// <https://github.com/gerchowl/crank/issues/160>. No clock, I/O or global state.
///
/// # Extreme values
/// Distinct classes, all documented here (#160). The frozen TS compiler has no bounds and may
/// hang or allocate without limit; none of the following changes a frozen vector.
/// - *Invalid prescriptions* (zero or negative FTP, out-of-range targets) are typed diagnostics
///   in `issues`, never errors. Comparison-only NaN/Infinity produced by finite inputs
///   (including watts/HR percentages at zero FTP) keeps JS semantics, and `Math.max`
///   propagates NaN during phase classification.
/// - [`CompileError::NonFinite`] is reserved for supplied numbers that are NaN or infinite.
/// - [`CompileError::InvalidArithmetic`] is nonfinite arithmetic *needed for output* from finite
///   inputs: overflow or undefined calculations, where TS may emit NaN or infinity. Example:
///   FTP 0, a hard step, then an HR-recovery step: TS reports NaN `durS`/`totalS`; this
///   returns the error. Guards also bound intermediates before their consumers, such as
///   expanded progression offsets that rest/HR targets would discard, so these remain safety
///   divergences even when TS eventually emits finite data. Public helpers validate supplied
///   athlete and offset parameters even when a target does not consume them.
/// - An infinite HR-recovery estimate is clamped to the finite duration bounds as in TS; an
///   unused NaN HR state does not reject an otherwise finite compile.
/// - Positive durations that cannot advance the `f64` timeline are
///   [`CompileError::TimelinePrecision`].
/// - Diagnostic text normalises signed zero to `0` as JS does; other extreme finite values use
///   Rust shortest-decimal formatting, which can differ from JS exponent notation.
///
/// # Errors
/// Returns [`CompileError`] for nonfinite inputs, nonfinite arithmetic needed in output,
/// or exceeded safety bounds. Comparison-only NaN/Infinity retains TS semantics.
pub fn compile_workout(
    w: &Workout,
    opts: &CompileOptions,
) -> Result<CompiledWorkout, CompileError> {
    input_finite(w.ftp)?;
    let a = opts.athlete.clone().unwrap_or_default();
    athlete_finite(&a)?;
    let tau = input_finite(opts.usual_tau_s.unwrap_or(55.))?;
    let mut flat = Flatten {
        raws: Vec::new(),
        issues: Vec::new(),
        visits: 0,
        ftp: w.ftp,
    };
    if w.ftp <= 0. {
        issue(
            &mut flat.issues,
            ValidationKind::InvalidFtp,
            Severity::Error,
            None,
            "FTP must be positive".into(),
        );
    }
    for (i, n) in w.nodes.iter().enumerate() {
        flat.visit(n, vec![i], vec![], Offset::default())?;
    }
    let phases = phases(&flat.raws, w.ftp)?;
    let mut timeline = Timeline {
        ftp: w.ftp,
        athlete: &a,
        tau,
        hr: a.rest_hr + 10.,
        total: 0.,
        range: [0., 0.],
        issues: flat.issues,
    };
    let mut blocks = Vec::new();
    for (r, p) in flat.raws.into_iter().zip(phases) {
        blocks.push(timeline.block(r, p)?);
    }
    let (trace, owner) = power_trace(&blocks)?;
    let np = normalized_power(&trace)?;
    let intensity = if w.ftp > 0. { finite(np / w.ftp)? } else { 0. };
    let seconds = u32::try_from(trace.len()).map_err(|_| CompileError::TraceLimit)?;
    let tss = if w.ftp > 0. {
        finite(((f64::from(seconds) * np * intensity) / (w.ftp * 3600.)) * 100.)?
    } else {
        0.
    };
    let mut tiz = [0u32; 7];
    let bounds = [0.55, 0.75, 0.9, 1.05, 1.2, 1.5, 8.].map(|p| js_round(p * w.ftp));
    for p in &trace {
        let zi = bounds.iter().position(|hi| p < hi).unwrap_or(6);
        tiz[zi] += 1;
    }
    reserve_diagnostic(&trace, &owner, &blocks, w.ftp, &mut timeline.issues)?;
    if w.nodes.is_empty() {
        issue(
            &mut timeline.issues,
            ValidationKind::EmptyWorkout,
            Severity::Error,
            None,
            "Workout is empty".into(),
        );
    } else if timeline.total > 1200.
        && blocks
            .iter()
            .rev()
            .take_while(|b| b.phase == Phase::Cooldown)
            .map(|b| b.dur_s)
            .sum::<f64>()
            < 180.
    {
        issue(
            &mut timeline.issues,
            ValidationKind::MissingCooldown,
            Severity::Warning,
            None,
            "No cool-down (≥ 3 min easy) at the end".into(),
        );
    }
    Ok(CompiledWorkout {
        blocks,
        total_s: timeline.total,
        total_range_s: timeline.range,
        tss: js_round(finite(tss * 10.)?) / 10.,
        intensity_factor: js_round(finite(intensity * 1000.)?) / 1000.,
        time_in_zone_s: tiz,
        issues: timeline.issues,
    })
}
