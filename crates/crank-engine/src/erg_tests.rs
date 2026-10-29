//! Tests of the ERG policy (#218): the following machine against the executable model's
//! fixture and against hand-written rules, then the engine around it.
use crate::engine::{CommandError, EngineCmd, RideSetup};
use crate::erg::TrainerFollowing as F;
use crate::erg::following::{
    Class, Comp, Component as C, ComponentState, Machine, REPLY_ROWS, SEC, TABLE_ORDER,
    reply_component,
};
use crate::tests::{Recorder, Rig, block, erg, rig};
use crate::ticks::TICK_US;
use crank_devices::{
    CommandStatus, ControlMode, LinkState, MonoTime, ObservationSource, ObservedControl,
    TrainerEvent, TrainerObservation, Wind,
};
use crank_domain::workout::Phase;
use std::sync::Arc;

fn obs(
    at: u64,
    control: Option<ObservedControl>,
    status: Option<CommandStatus>,
) -> TrainerObservation {
    TrainerObservation {
        at: MonoTime(at),
        source: ObservationSource::Page71Echo,
        control,
        status,
        target_limit: None,
    }
}
fn echo_erg(w: u16) -> ObservedControl {
    ObservedControl::Erg { quarter_w: w * 4 }
}
#[expect(clippy::unnecessary_wraps, reason = "the argument of set_desired")]
fn erg_key(w: u16) -> Option<Vec<ObservedControl>> {
    Some(vec![echo_erg(w)])
}
fn sim_mode(grade: f32) -> ControlMode {
    ControlMode::Sim {
        grade_pct: grade,
        crr: 0.004,
        wind: Wind::default(),
    }
}
#[expect(clippy::unnecessary_wraps, reason = "the argument of set_desired")]
fn key(m: ControlMode) -> Option<Vec<ObservedControl>> {
    Some(m.grid_key().expect("in range"))
}
fn pass(c: ObservedControl) -> TrainerObservation {
    obs(0, Some(c), Some(CommandStatus::Pass))
}
/// An active ERG machine at `t = 0` desiring `w`.
fn erg_machine(w: u16) -> Machine {
    let mut m = Machine::new();
    m.set_link(true, 0);
    m.set_desired(erg_key(w), 0);
    m
}
/// Agreeing ERG echoes at `times_ms`, ticking at each.
fn replies(m: &mut Machine, w: u16, times_ms: &[u64]) {
    for &t in times_ms {
        let at = t * 1000;
        m.observe(&pass(echo_erg(w)), at);
        m.tick(at);
    }
}

// ---- the model's table, constants and traces -------------------------------------------

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../tests/fixtures/following.json")).expect("fixture")
}
/// The model's numbers are this crate's.
fn assert_constants(fx: &serde_json::Value) {
    use crate::erg::following as f;
    let c = &fx["constants"];
    let secs = |v: &serde_json::Value| v.as_u64().expect("int") * SEC;
    assert_eq!(secs(&c["L_s"]), f::L);
    assert_eq!(c["N_follow"].as_u64(), Some(u64::from(f::N_FOLLOW)));
    assert_eq!(secs(&c["T_verify_s"]), f::T_VERIFY);
    assert_eq!(secs(&c["T_mismatch_s"]), f::T_MISMATCH);
    assert_eq!(c["unknown_mult"].as_u64(), Some(f::UNKNOWN_MULT));
    assert_eq!(secs(&c["unknown_pad_s"]), f::UNKNOWN_PAD);
    for (name, comp) in [("erg", C::Erg), ("track", C::Track), ("wind", C::Wind)] {
        assert_eq!(secs(&c["P_s"][name]), comp.period(), "P_{name}");
        assert_eq!(
            secs(&c["T_unknown_s"][name]),
            comp.t_unknown(),
            "T_unknown({name})"
        );
    }
    assert_eq!(
        C::Resistance.period(),
        C::Erg.period(),
        "ADR: resistance polls like ERG"
    );
    assert_eq!(C::Resistance.t_unknown(), C::Erg.t_unknown());
}
/// The ordered table: ids, order, input classes and the pre-step marker.
fn assert_table(fx: &serde_json::Value) {
    let order: Vec<&str> = fx["table_order"]
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().expect("str"))
        .collect();
    assert_eq!(order, TABLE_ORDER);
    let rows = fx["reply_rows"].as_array().expect("array");
    assert_eq!(rows.len(), REPLY_ROWS.len());
    for (spec, row) in rows.iter().zip(REPLY_ROWS.iter()) {
        let on = spec["on"].as_str().expect("str");
        assert_eq!(spec["id"].as_str(), Some(row.id));
        assert_eq!(on.contains('A'), row.on.0, "{} agree", row.id);
        assert_eq!(on.contains('D'), row.on.1, "{} disagree", row.id);
        assert_eq!(spec["pre"].as_bool(), Some(row.pre), "{} pre-step", row.id);
    }
}
fn verdict_name(v: F) -> &'static str {
    match v {
        F::Unknown => "unknown",
        F::Checking => "checking",
        F::Suspect => "suspect",
        F::Following => "following",
        F::NotFollowing => "not_following",
    }
}
/// Replay one trace (link, desired, reply, tick per instant) and compare every step's
/// aggregate and required-component states with the model's. Returns the verdicts seen.
fn replay(n: usize, tr: &serde_json::Value, seen: &mut std::collections::BTreeSet<String>) {
    let unit = tr["unit"].as_u64().expect("int");
    let events = tr["events"].as_array().expect("array");
    let mut m = Machine::new();
    let mut desired: Option<Vec<ObservedControl>> = None;
    for t in 0..=tr["until"].as_u64().expect("int") {
        let now = t * SEC / unit;
        if let Some(e) = events.iter().find(|e| e["t"].as_u64() == Some(t)) {
            match e.get("link").and_then(|v| v.as_str()) {
                Some("lost") => m.set_link(false, now),
                Some("ready") => m.set_link(true, now),
                _ => {}
            }
            if let Some(d) = e.get("desired") {
                desired = mode_key(d);
                m.set_desired(desired.clone(), now);
            }
            if let Some(r) = e.get("rep").filter(|r| !r.is_null()) {
                m.observe(&reply(r, desired.as_deref()), now);
            }
        }
        m.tick(now);
        let i = usize::try_from(t).expect("small");
        let want = tr["agg"][i].as_str().expect("str");
        seen.insert(want.to_string());
        assert_eq!(verdict_name(m.verdict()), want, "trace {n} t={t}");
        let comps: Vec<String> = if m.active() {
            m.components()
                .into_iter()
                .map(|(c, s)| format!("{}:{}", comp_name(c), state_name(s)))
                .collect()
        } else {
            vec![]
        };
        let want: Vec<&str> = tr["comps"][i]
            .as_array()
            .expect("array")
            .iter()
            .map(|v| v.as_str().expect("str"))
            .collect();
        assert_eq!(comps, want, "trace {n} t={t} components");
    }
}
#[test]
fn refines_the_model() {
    let fx = fixture();
    assert_constants(&fx);
    assert_table(&fx);
    let traces = fx["traces"].as_array().expect("array");
    assert!(traces.len() > 40);
    let mut seen = std::collections::BTreeSet::new();
    for (n, tr) in traces.iter().enumerate() {
        replay(n, tr, &mut seen);
    }
    for v in [
        "unknown",
        "checking",
        "suspect",
        "following",
        "not_following",
    ] {
        assert!(seen.contains(v), "the fixture never reaches {v}");
    }
}
fn comp_name(c: C) -> &'static str {
    match c {
        C::Erg => "erg",
        C::Resistance => "resistance",
        C::Track => "track",
        C::Wind => "wind",
    }
}
fn state_name(s: ComponentState) -> &'static str {
    match s {
        ComponentState::Unverified => "unverified",
        ComponentState::Following => "following",
        ComponentState::NotFollowing => "not_following",
        ComponentState::Silent => "silent",
    }
}
/// The model's abstract mode → the engine's `ControlMode`'s grid key.
fn mode_key(v: &serde_json::Value) -> Option<Vec<ObservedControl>> {
    let a = v.as_array()?;
    let n = |i: usize| a[i].as_i64().expect("int");
    match a[0].as_str()? {
        "erg" => erg_key(u16::try_from(n(1)).expect("small")),
        "sim" => key(ControlMode::Sim {
            #[expect(clippy::cast_precision_loss, reason = "tiny")]
            grade_pct: n(1) as f32,
            crr: 0.004,
            wind: Wind {
                #[expect(clippy::cast_precision_loss, reason = "tiny")]
                speed_kmh: n(2) as f32,
                ..Wind::default()
            },
        }),
        other => panic!("mode {other}"),
    }
}
/// The model's abstract reply → an observation (`fail` echoes the desired value with a
/// failing status, the case R3 singles out).
fn reply(r: &serde_json::Value, desired: Option<&[ObservedControl]>) -> TrainerObservation {
    let a = r.as_array().expect("array");
    let comp = a[1].as_str().expect("str");
    let value = |v: i64| match comp {
        "erg" => echo_erg(u16::try_from(v).expect("small")),
        "track" => ObservedControl::Track {
            grade_centi_pct: i32::try_from(v * 100).expect("small"),
            crr_units: 80,
        },
        "wind" => ObservedControl::Wind {
            cw_centi: 51,
            speed_kmh: i16::try_from(v).expect("small"),
            drafting_centi: 100,
        },
        other => panic!("component {other}"),
    };
    match a[0].as_str().expect("str") {
        "echo" => obs(
            0,
            Some(value(a[2].as_i64().expect("int"))),
            Some(CommandStatus::Pass),
        ),
        "fail" => {
            let own = desired
                .into_iter()
                .flatten()
                .copied()
                .find(|c| C::of(*c).is_some_and(|k| comp_name(k) == comp));
            obs(
                0,
                Some(own.unwrap_or_else(|| value(0))),
                Some(CommandStatus::Fail),
            )
        }
        other => panic!("reply {other}"),
    }
}

// ---- the machine's rules, hand-written (each fails without its row) ----------------------

#[test]
fn a_single_stale_agreeing_reply_never_verifies() {
    let mut m = erg_machine(200);
    replies(&mut m, 200, &[3000]);
    assert_eq!(m.verdict(), F::Checking);
}
#[test]
fn a_burst_of_agreeing_replies_inside_t_verify_never_verifies() {
    let mut m = erg_machine(200);
    // N_follow replies within 1.5 s: count met, span not (C3's second condition)
    replies(&mut m, 200, &[1000, 1500, 2000, 2400]);
    assert_eq!(m.verdict(), F::Checking);
    replies(&mut m, 200, &[3000]);
    assert_eq!(m.verdict(), F::Following, "3.0 s − 1.0 s ≥ T_verify");
}
#[test]
fn recovery_is_exact_for_erg_and_sim() {
    // ERG: replies every second from 10 s after silence → following at first + 2 s
    let mut m = erg_machine(150);
    replies(&mut m, 150, &[10000, 11000]);
    assert_ne!(m.verdict(), F::Following);
    replies(&mut m, 150, &[12000]);
    assert_eq!(m.verdict(), F::Following);
    // SIM: wind 10/12/14 s, track 11/13/15 s → following at exactly 15 s (§4.4.1 guarantee 4)
    let sim = sim_mode(3.0);
    let ks = sim.grid_key().expect("in range");
    let mut m = Machine::new();
    m.set_link(true, 0);
    m.set_desired(key(sim), 0);
    let at = |t: u64| t * SEC;
    for (t, c) in [
        (10, ks[0]),
        (11, ks[1]),
        (12, ks[0]),
        (13, ks[1]),
        (14, ks[0]),
    ] {
        m.observe(&pass(c), at(t));
        m.tick(at(t));
        assert_ne!(m.verdict(), F::Following, "t={t}");
    }
    m.observe(&pass(ks[1]), at(15));
    m.tick(at(15));
    assert_eq!(m.verdict(), F::Following);
}
#[test]
fn a_disagreement_needs_two_replies_t_mismatch_apart() {
    let mut m = erg_machine(200);
    m.observe(
        &obs(SEC, Some(echo_erg(150)), Some(CommandStatus::Pass)),
        SEC,
    );
    assert_eq!(m.verdict(), F::Suspect);
    m.observe(
        &obs(2 * SEC, Some(echo_erg(150)), Some(CommandStatus::Pass)),
        2 * SEC,
    );
    assert_eq!(m.verdict(), F::Suspect, "1 s apart: C6, not C5");
    m.observe(
        &obs(3 * SEC, Some(echo_erg(150)), Some(CommandStatus::Pass)),
        3 * SEC,
    );
    assert_eq!(m.verdict(), F::NotFollowing);
    assert_eq!(m.blocking(), Some(C::Erg));
}
#[test]
fn a_lagging_echo_inside_l_is_neutral_for_the_episode_and_agrees() {
    let mut m = erg_machine(200);
    m.set_desired(erg_key(250), SEC);
    // an echo of 200 at 2.5 s: 1.5 s after it stopped being desired ≤ L → Agree (R2)
    m.observe(&pass(echo_erg(200)), 2_500_000);
    assert_eq!(m.verdict(), F::Checking);
    // at 3.5 s it is 2.5 s old: Disagree (R3)
    m.observe(&pass(echo_erg(200)), 3_500_000);
    assert_eq!(m.verdict(), F::Suspect);
}
#[test]
fn routing_r1_neutral_observations_change_nothing() {
    let mut m = erg_machine(200);
    // page-25 flags, no control, no status, NotControl whatever the status, Pending, Other
    let neutral = [
        obs(0, None, None),
        obs(0, None, Some(CommandStatus::Fail)),
        obs(0, Some(echo_erg(150)), None),
        obs(
            0,
            Some(ObservedControl::NotControl(0x37)),
            Some(CommandStatus::Pass),
        ),
        obs(
            0,
            Some(ObservedControl::NotControl(0x10)),
            Some(CommandStatus::NotSupported),
        ),
        obs(
            0,
            Some(ObservedControl::NotControl(0x10)),
            Some(CommandStatus::Rejected),
        ),
        obs(0, Some(echo_erg(150)), Some(CommandStatus::Pending)),
        obs(0, Some(echo_erg(150)), Some(CommandStatus::Other(9))),
    ];
    for o in neutral {
        for t in [1, 3, 6] {
            let before = m.clone();
            m.observe(&o, t * SEC);
            assert_eq!(m.components(), before.components(), "{o:?}");
            assert_eq!(m.verdict(), before.verdict());
        }
    }
    assert_eq!(m.verdict(), F::Checking);
}
#[test]
fn routing_r3_failing_status_disagrees_even_with_an_equal_echo() {
    for status in [
        CommandStatus::Fail,
        CommandStatus::NotSupported,
        CommandStatus::Rejected,
    ] {
        let mut m = erg_machine(200);
        m.observe(&obs(SEC, Some(echo_erg(200)), Some(status)), SEC);
        assert_eq!(m.verdict(), F::Suspect, "{status:?}");
    }
}
#[test]
fn routing_r4_r5_a_page_that_was_required_is_neutral_a_foreign_one_disagrees() {
    let sim = sim_mode(2.0);
    let ks = sim.grid_key().expect("in range");
    let mut m = erg_machine(200);
    m.set_desired(key(sim), SEC); // ERG stopped being required at 1 s
    // ERG echo 1.5 s later: within L → R4, neutral
    m.observe(&pass(echo_erg(200)), 2_500_000);
    assert_eq!(m.verdict(), F::Checking);
    // ERG echo 3 s after: not required within L → R5: Disagree for every required component
    m.observe(&pass(echo_erg(200)), 4 * SEC);
    assert_eq!(m.verdict(), F::Suspect);
    assert!(ks.len() == 2);
}
#[test]
fn row_c2_runs_before_the_matching_row() {
    // Disagree at 0 opens an episode; the next Disagree arrives exactly T_unknown later, with
    // no tick in between (C7 has not run). C2 resets first, so it is a fresh episode (C6),
    // not a confirmed mismatch (C5).
    let mut m = erg_machine(200);
    let wrong = obs(0, Some(echo_erg(100)), Some(CommandStatus::Pass));
    m.observe(&wrong, 0);
    m.observe(&wrong, C::Erg.t_unknown());
    assert_eq!(m.verdict(), F::Suspect);
    // …and the reply table agrees when driven directly
    let c = reply_component(Comp::fresh(0), C::Erg, Class::Disagree, 0);
    let c = reply_component(c, C::Erg, Class::Disagree, C::Erg.t_unknown());
    assert_eq!(c.state, ComponentState::Unverified);
}
#[test]
fn silence_is_t_unknown_of_required_time_and_all_silent_is_unknown() {
    let mut m = erg_machine(200);
    m.tick(5 * SEC - 1);
    assert_eq!(m.verdict(), F::Checking);
    m.tick(5 * SEC);
    assert_eq!(
        m.verdict(),
        F::Unknown,
        "A2: every required component silent"
    );
    // SIM: 9 s per component; one talking sibling makes the silent one not_following (A3)
    let sim = sim_mode(1.0);
    let ks = sim.grid_key().expect("in range");
    let mut m = Machine::new();
    m.set_link(true, 0);
    m.set_desired(key(sim), 0);
    for t in [1, 3, 5, 7, 9, 11] {
        m.observe(&pass(ks[1]), t * SEC);
        m.tick(t * SEC);
    }
    assert_eq!(m.verdict(), F::NotFollowing);
    assert_eq!(m.blocking(), Some(C::Wind), "wind never answered");
}
#[test]
fn activity_follows_link_and_desired_and_inactive_machines_ignore_replies() {
    let mut m = Machine::new();
    m.observe(&pass(echo_erg(200)), SEC);
    assert_eq!(m.verdict(), F::Unknown);
    m.set_link(true, 0);
    assert!(!m.active(), "no desired mode yet");
    m.set_desired(erg_key(200), 0);
    assert!(m.active());
    replies(&mut m, 200, &[1000, 2000, 3000]);
    assert_eq!(m.verdict(), F::Following);
    m.set_desired(None, 4 * SEC); // ride 1 ends, link stays Ready
    assert_eq!(m.verdict(), F::Unknown);
    m.set_desired(erg_key(120), 6 * SEC); // ride 2
    assert!(m.active());
    replies(&mut m, 120, &[7000, 8000, 9000]);
    assert_eq!(m.verdict(), F::Following);
    m.set_link(false, 10 * SEC);
    assert_eq!(m.verdict(), F::Unknown);
}
#[test]
fn a_mode_toggle_cannot_mask_a_missing_track_component() {
    // ERG ↔ SIM every 2 s; wind (SIM) and ERG are answered, track never is (the model's
    // `r54_3_kind_toggle_cannot_mask_track`): the silence clock counts required time only,
    // and toggling can neither restart it nor let the aggregate verify.
    let sim = sim_mode(1.0);
    let ks = sim.grid_key().expect("in range");
    let mut m = Machine::new();
    m.set_link(true, 0);
    m.set_desired(key(sim), 0);
    let mut not_following = false;
    for t in 1..60_u64 {
        let in_sim = (t / 2) % 2 == 0;
        m.set_desired(if in_sim { key(sim) } else { erg_key(200) }, t * SEC);
        m.observe(&pass(if in_sim { ks[0] } else { echo_erg(200) }), t * SEC);
        m.tick(t * SEC);
        if in_sim {
            assert_ne!(m.verdict(), F::Following, "t={t}");
        }
        not_following |= m.verdict() == F::NotFollowing;
    }
    assert!(not_following);
}

// ---- #226: the captured frame is decoded by `crank-ble` (crates/crank-ble/src/routing_tests.rs)

#[test]
fn an_unsupported_page_request_reply_is_neutral_in_every_mode() {
    // `47 10 DD 02 …`: page 71, last page 0x10 (not a control page), status 2 (R1)
    let o = obs(
        0,
        Some(ObservedControl::NotControl(0x10)),
        Some(CommandStatus::NotSupported),
    );
    for desired in [erg_key(200), key(sim_mode(2.0)), key(ControlMode::Free)] {
        let mut m = Machine::new();
        m.set_link(true, 0);
        m.set_desired(desired, 0);
        let before = m.components();
        m.observe(&o, 2 * SEC);
        assert_eq!(m.components(), before);
        assert_eq!(
            m.verdict(),
            F::Checking,
            "R5 would have made this a Disagree"
        );
    }
}

// ---- engine level ---------------------------------------------------------------------------

fn pedal(r: &Rig, from: u64, to: u64, w: u16, rpm: u8) {
    for k in from..to {
        r.sample(k, w, rpm);
    }
}
fn echo_at(r: &Rig, k: u64, control: ObservedControl) {
    r.events
        .try_send(TrainerEvent::Observed(obs(
            k * TICK_US + 2,
            Some(control),
            Some(CommandStatus::Pass),
        )))
        .expect("channel");
}
fn work(w: f64) -> crank_domain::workout::FlatBlock {
    block(600.0, w, w, Phase::Work)
}

#[test]
fn the_status_aggregates_the_machine_and_names_the_component() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    assert_eq!(r.status().trainer_following, F::Unknown);
    pedal(&r, 0, 40, 200, 90);
    for k in [4, 8, 12] {
        echo_at(&r, k, echo_erg(200));
    }
    r.through(11);
    assert_eq!(r.status().trainer_following, F::Checking);
    r.through(12);
    assert_eq!(r.status().trainer_following, F::Following);
    // a stuck trainer: wrong echoes 1 s apart → suspect, then not_following naming `erg`
    echo_at(&r, 16, echo_erg(150));
    r.through(16);
    assert_eq!(r.status().trainer_following, F::Suspect);
    for k in [20, 24] {
        echo_at(&r, k, echo_erg(150));
    }
    r.through(24);
    let s = r.status();
    assert_eq!(s.trainer_following, F::NotFollowing);
    assert_eq!(s.not_following_component, Some(C::Erg));
    // silence on a talking trainer: every component silent → unknown again
    r.through(39);
    assert_eq!(
        r.status().trainer_following,
        F::NotFollowing,
        "wrong echoes still count"
    );
    r.link(LinkState::Lost);
    r.through(40);
    assert_eq!(r.status().trainer_following, F::Unknown, "Deactivate");
}
#[test]
fn a_requirement_toggle_clears_a_verification_it_does_not_carry_over() {
    // C8: verified ERG, SIM for 1 s (ERG not required), ERG again within T_unknown(erg):
    // the old verification is gone, the verdict is `checking`, and needs fresh evidence.
    let mut m = erg_machine(200);
    replies(&mut m, 200, &[1000, 2000, 3000]);
    assert_eq!(m.verdict(), F::Following);
    m.set_desired(key(sim_mode(1.0)), 3_500_000);
    m.set_desired(erg_key(200), 4_500_000);
    assert_eq!(m.verdict(), F::Checking);
    replies(&mut m, 200, &[5000, 6000]);
    assert_eq!(m.verdict(), F::Checking, "two replies are not N_follow");
    replies(&mut m, 200, &[7000]);
    assert_eq!(m.verdict(), F::Following);
}
#[test]
fn replies_that_predate_a_link_transition_do_not_revive_the_machine() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 6, 200, 90);
    r.through(5);
    // three wrong echoes stamped before the transition, delivered after it
    for k in [1, 3, 4] {
        echo_at(&r, k, echo_erg(100));
    }
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    pedal(&r, 6, 12, 200, 90);
    r.through(11);
    assert_ne!(r.status().trainer_following, F::NotFollowing);
    assert_ne!(r.status().trainer_following, F::Suspect);
}

// -- soft start after leaving free (ADR-0002 §7.5 item 5) --

fn blocks_free_then_work() -> Vec<crank_domain::workout::FlatBlock> {
    vec![
        block(600.0, 0.0, 0.0, Phase::Free),
        work(200.0),
        block(600.0, 0.0, 0.0, Phase::Free),
        work(300.0),
    ]
}
#[test]
fn a_loss_during_a_free_block_is_repaid_when_work_starts() {
    let mut r = rig();
    r.load(blocks_free_then_work());
    r.start();
    pedal(&r, 0, 8, 120, 90);
    r.through(7);
    assert_eq!(r.modes(), vec![ControlMode::Free]);
    // the link drops in the Free block and comes back; 30 s of riding finish the "soft start"
    // in Free, as the old engine's flag did
    r.link(LinkState::Lost);
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    pedal(&r, 8, 8 + 30 * 4, 120, 90);
    r.through(8 + 30 * 4 - 1);
    assert_eq!(r.modes().last(), Some(&ControlMode::Free));
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    let k0 = 8 + 30 * 4;
    pedal(&r, k0, k0 + 24, 120, 90);
    r.through(k0 + 23);
    let sent: Vec<u16> = r
        .modes()
        .into_iter()
        .map(|m| match m {
            ControlMode::Erg { target_w } => target_w,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        sent,
        vec![120, 136, 152, 168, 184, 200],
        "five 1 Hz steps, origin = 120 W"
    );
}
#[test]
fn leaving_free_without_a_loss_also_ramps_from_the_current_power() {
    let mut r = rig();
    r.load(blocks_free_then_work());
    r.start();
    pedal(&r, 0, 16, 90, 85);
    r.through(15);
    r.modes();
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    pedal(&r, 16, 40, 90, 85);
    r.through(39);
    let sent: Vec<u16> = r
        .modes()
        .into_iter()
        .map(|m| match m {
            ControlMode::Erg { target_w } => target_w,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(sent, vec![90, 112, 134, 156, 178, 200]);
}
#[test]
fn the_ramp_follows_a_target_that_changes_mid_ramp() {
    let mut r = rig();
    r.load(vec![
        block(600.0, 0.0, 0.0, Phase::Free),
        work(200.0),
        work(100.0),
    ]);
    r.start();
    pedal(&r, 0, 8, 100, 90);
    r.through(7);
    r.modes();
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    pedal(&r, 8, 20, 100, 90);
    r.through(13);
    r.engine.command(EngineCmd::SkipBlock).expect("skip"); // mid-ramp: new target 100 W
    r.through(19);
    let sent: Vec<u16> = r
        .modes()
        .into_iter()
        .filter_map(|m| match m {
            ControlMode::Erg { target_w } => Some(target_w),
            _ => None,
        })
        .collect();
    // origin 100 W: ramp toward 200 W for the first steps, then toward the new 100 W
    assert_eq!(sent.first(), Some(&100));
    assert!(sent.iter().all(|w| (100..=140).contains(w)), "{sent:?}");
}

// -- release (ADR-0002 §7.4) --

#[test]
fn low_cadence_for_two_seconds_releases_and_sixty_for_two_seconds_re_engages() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 8, 190, 90);
    r.through(7);
    assert_eq!(r.modes(), vec![erg(190), erg(192)]);
    // 39 rpm for 8 ticks = 2 s: released on the eighth
    pedal(&r, 8, 15, 120, 39);
    r.through(14);
    assert!(!r.status().erg_released, "7 ticks are not 2 s");
    pedal(&r, 15, 16, 120, 39);
    r.through(15);
    assert!(r.status().erg_released);
    // the start's ramp (190 W origin) ran until the release
    assert_eq!(
        r.modes(),
        vec![erg(194), erg(196), ControlMode::Resistance { pct: 0.0 }]
    );
    // 59 rpm never re-engages; 60 for 8 ticks does, with a ramp from the current power
    pedal(&r, 16, 40, 100, 59);
    r.through(39);
    assert!(r.status().erg_released);
    pedal(&r, 40, 47, 100, 60);
    r.through(46);
    assert!(r.status().erg_released);
    pedal(&r, 47, 48, 100, 60);
    r.through(47);
    assert!(!r.status().erg_released);
    pedal(&r, 48, 72, 100, 90);
    r.through(71);
    let sent: Vec<u16> = r
        .modes()
        .into_iter()
        .filter_map(|m| match m {
            ControlMode::Erg { target_w } => Some(target_w),
            _ => None,
        })
        .collect();
    assert_eq!(sent, vec![100, 120, 140, 160, 180, 200]);
    // the block timer kept running
    assert_eq!(r.status().block_elapsed.0, 72);
}
#[test]
fn a_zero_target_is_never_released() {
    let mut r = rig();
    r.load(vec![work(0.0)]);
    r.start();
    pedal(&r, 0, 40, 0, 10);
    r.through(39);
    assert!(!r.status().erg_released);
}

// -- health cap --

fn capped(
    cap: Option<u16>,
    blocks: Vec<crank_domain::workout::FlatBlock>,
) -> (Rig, Result<(), CommandError>) {
    let mut r = rig();
    r.engine
        .command(EngineCmd::Load(RideSetup {
            blocks,
            recorder: Box::new(Recorder(Arc::default())),
            cap_w: cap,
        }))
        .expect("load");
    let started = r.engine.command(EngineCmd::Start);
    (r, started)
}
#[test]
fn a_ramp_above_the_cap_is_refused_at_start() {
    let (mut r, started) = capped(Some(180), vec![block(600.0, 100.0, 200.0, Phase::Work)]);
    // 200 W > 180 W cap: the ride does not start (the clamp itself: the next test)
    assert_eq!(started, Err(CommandError::CapExceededTarget));
    r.engine.command(EngineCmd::Stop).expect("unload");
}
#[test]
fn the_soft_start_from_above_the_cap_never_commands_above_it() {
    let (mut r, started) = capped(Some(180), vec![work(150.0)]);
    assert_eq!(started, Ok(()));
    assert_eq!(
        r.status().cap.map(|c| (c.cap_w, c.erg_only)),
        Some((180, true))
    );
    pedal(&r, 0, 8, 400, 90);
    r.through(7);
    r.modes();
    // loss and return while the rider produces 400 W: the origin is 400 W, the command ≤ cap
    r.link(LinkState::Lost);
    r.link(LinkState::Ready);
    pedal(&r, 8, 32, 400, 90);
    r.through(31);
    let sent: Vec<u16> = r
        .modes()
        .into_iter()
        .filter_map(|m| match m {
            ControlMode::Erg { target_w } => Some(target_w),
            _ => None,
        })
        .collect();
    // the ramp 400 → 150 W is above the cap until its last step: clamped, then the target
    assert_eq!(sent, vec![180, 150]);
}
#[test]
fn a_capped_ride_refuses_free_and_over_cap_blocks_and_starts_otherwise() {
    let (_, started) = capped(
        Some(200),
        vec![work(150.0), block(60.0, 0.0, 0.0, Phase::Free)],
    );
    assert_eq!(started, Err(CommandError::CapRequiresErg));
    let (_, started) = capped(Some(200), vec![]);
    assert_eq!(
        started,
        Err(CommandError::CapRequiresErg),
        "a free ride is not ERG"
    );
    let (_, started) = capped(Some(200), vec![work(150.0), work(201.0)]);
    assert_eq!(started, Err(CommandError::CapExceededTarget));
    let (_, started) = capped(Some(200), vec![work(150.0), work(200.0)]);
    assert_eq!(started, Ok(()));
    let (r, started) = capped(None, vec![block(60.0, 0.0, 0.0, Phase::Free), work(900.0)]);
    assert_eq!(started, Ok(()));
    assert_eq!(r.status().cap, None);
}
#[test]
fn the_policy_clamps_a_target_above_the_cap_even_if_a_ride_got_through() {
    // defence in depth (DESIGN §15.2): the clamp holds without the start guard
    use crate::control::{PolicyContext, TargetPolicy};
    let b = block(100.0, 250.0, 250.0, Phase::Work);
    let ctx = PolicyContext {
        block: Some(&b),
        block_index: 0,
        block_elapsed_ticks: 0,
        tick: 0,
        link_epoch: 0,
        flow_start: Some(0),
        power_3s_w: Some(250.0),
        cadence_rpm: Some(90),
        last_sent: None,
        riding: true,
        user_paused: false,
        following: F::Unknown,
        max_target_w: 2000,
        cap_w: Some(180),
    };
    assert_eq!(crate::erg::ErgPolicy::new().desired(&ctx), Some(erg(180)));
}

// -- compliance --

#[test]
fn compliance_counts_settled_followed_erg_seconds_only() {
    let mut r = rig();
    r.load(vec![work(200.0), work(300.0)]);
    r.start();
    // 40 s on target, answered every second so the trainer is `following` after 3 s; the
    // ride start is an engagement (#250), so its first 10 s are settle
    pedal(&r, 0, 160, 200, 90);
    for k in (4..160).step_by(4) {
        echo_at(&r, k, echo_erg(200));
    }
    r.through(159);
    let c = r.status().compliance;
    assert_eq!(c.counted_s + c.excluded_s, 40);
    assert_eq!(c.counted_s, c.within_s, "all on target");
    assert!(c.counted_s >= 25, "{c:?}");
}
#[test]
fn compliance_excludes_not_following_seconds_and_counts_the_rest_off_target() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    // power 150 W (off target) and a stuck trainer: wrong echoes → not_following from 3 s
    pedal(&r, 0, 60, 150, 90);
    for k in (4..60).step_by(4) {
        echo_at(&r, k, echo_erg(100));
    }
    r.through(59);
    let c = r.status().compliance;
    assert_eq!(c.counted_s + c.excluded_s, 15);
    assert!(c.excluded_s >= 11, "{c:?}");
    assert_eq!(c.within_s, 0);
}
#[test]
fn a_ramp_block_is_compared_second_by_second_without_settle() {
    let mut r = rig();
    r.load(vec![block(20.0, 100.0, 200.0, Phase::Work)]);
    r.start();
    for k in 0..80_u16 {
        r.sample(u64::from(k), 100 + k * 100 / 80, 90);
    }
    r.through(79);
    let c = r.status().compliance;
    // the ride start's settle is the only exclusion: continuous updates are no discontinuity
    assert_eq!((c.excluded_s, c.counted_s), (10, 10), "{c:?}");
}

#[test]
fn a_block_boundary_settles_only_when_the_target_jumps() {
    let mut r = rig();
    // the ride start (10 s settle, #250); then 10 s at 200 W continuing the previous block's
    // end (no settle), then 300 W (a step: 5 s settle)
    r.load(vec![
        block(10.0, 200.0, 200.0, Phase::Work),
        block(10.0, 200.0, 200.0, Phase::Work),
        block(20.0, 300.0, 300.0, Phase::Work),
    ]);
    r.start();
    for k in 0..120 {
        r.sample(k, if k < 80 { 200 } else { 300 }, 90);
    }
    r.through(119);
    let c = r.status().compliance;
    assert_eq!((c.excluded_s, c.counted_s), (15, 15), "{c:?}");
    assert_eq!(c.within_s, c.counted_s);
}
#[test]
fn an_engagement_settles_the_ramp_plus_five_seconds() {
    let mut r = rig();
    r.load(vec![block(600.0, 0.0, 0.0, Phase::Free), work(200.0)]);
    r.start();
    pedal(&r, 0, 16, 200, 90);
    r.through(15);
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    pedal(&r, 16, 72, 200, 90);
    r.through(71);
    let c = r.status().compliance;
    // 4 free seconds, then the 5 s soft start and 5 s more are settle: 14 excluded, 4 counted
    assert_eq!((c.excluded_s, c.counted_s), (14, 4), "{c:?}");
}
#[test]
fn paused_and_released_seconds_are_excluded() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    // the ride start's 10 s settle (#250), then 4 counted seconds
    pedal(&r, 0, 56, 200, 90);
    r.through(55);
    assert_eq!(r.status().compliance.counted_s, 4);
    // 20 rpm: released on the eighth tick (63), which closes second 15; seconds 16 and 17
    // are released throughout. Second 14 still counts (150 W against 200 W: off target).
    pedal(&r, 56, 72, 150, 20);
    r.through(71);
    let c = r.status().compliance;
    assert_eq!((c.counted_s, c.within_s, c.excluded_s), (5, 4, 13), "{c:?}");
    assert!(r.status().erg_released);
}

#[test]
fn a_ready_event_restarts_the_release_count() {
    // 6 low-cadence ticks, a `Ready` event (no loss), 2 more: 8 ticks in a row would
    // release, but nothing sampled on the old link counts across the event (§7.5 spirit).
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 8, 190, 90);
    pedal(&r, 8, 14, 150, 39);
    r.through(13);
    r.link(LinkState::Ready);
    pedal(&r, 14, 16, 150, 39);
    r.through(15);
    assert!(!r.status().erg_released);
    // the re-issued ramp step is applied at tick 14; eight fresh ticks follow (15..=22)
    pedal(&r, 16, 24, 150, 39);
    r.through(22);
    assert!(r.status().erg_released, "8 fresh ticks release");
}

#[test]
fn a_reply_that_arrives_after_its_tick_was_processed_still_counts() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 24, 200, 90);
    r.through(23);
    assert_eq!(r.status().trainer_following, F::Unknown);
    // stamped in tick 10, delivered after tick 23 was processed: clamped to the next tick
    echo_at(&r, 10, echo_erg(100));
    r.through(24);
    assert_eq!(r.status().trainer_following, F::Suspect);
}

/// A policy context for tick `k` of a 600 s block.
fn ctx_at(b: &crank_domain::workout::FlatBlock, k: u64) -> crate::control::PolicyContext<'_> {
    crate::control::PolicyContext {
        block: Some(b),
        block_index: 0,
        block_elapsed_ticks: k,
        tick: k,
        link_epoch: 0,
        flow_start: Some(0),
        power_3s_w: Some(100.0),
        cadence_rpm: Some(90),
        last_sent: None,
        riding: true,
        user_paused: false,
        following: F::Following,
        max_target_w: 2000,
        cap_w: Some(180),
    }
}
#[test]
fn the_cap_clamps_every_erg_command_of_a_ramp() {
    use crate::control::TargetPolicy;
    let b = block(600.0, 100.0, 300.0, Phase::Work);
    let mut p = crate::erg::ErgPolicy::new();
    let mut sent = vec![];
    for k in 0..2400 {
        if let Some(ControlMode::Erg { target_w }) = p.desired(&ctx_at(&b, k)) {
            sent.push(target_w);
        }
    }
    assert!(sent.iter().all(|w| *w <= 180), "{sent:?}");
    assert_eq!(sent.first(), Some(&100));
    assert_eq!(
        sent.iter().max(),
        Some(&180),
        "the ramp reaches and holds the cap"
    );
    assert!(
        sent.windows(2).filter(|w| w[0] != w[1]).count() > 50,
        "it did ramp"
    );
}

// -- compliance exclusions at the policy --

fn tally(f: F, riding: bool) -> crate::erg::Compliance {
    use crate::control::TargetPolicy;
    let b = block(600.0, 200.0, 200.0, Phase::Work);
    let mut p = crate::erg::ErgPolicy::settled();
    for k in 0..8 {
        let mut c = ctx_at(&b, k);
        c.following = f;
        c.riding = riding;
        c.cap_w = None;
        c.power_3s_w = Some(200.0);
        p.desired(&c);
    }
    p.status().compliance
}
#[test]
fn compliance_excludes_suspect_and_not_following_seconds_only() {
    // ADR-0002 §7.8: "every second with a `suspect` or `not_following` tick"
    for (f, counted) in [
        (F::Following, true),
        (F::Checking, true),
        (F::Unknown, true),
        (F::Suspect, false),
        (F::NotFollowing, false),
    ] {
        let c = tally(f, true);
        assert_eq!(
            (c.counted_s > 0, c.counted_s + c.excluded_s),
            (counted, 2),
            "{f:?}"
        );
    }
    assert_eq!(tally(F::Following, false).excluded_s, 2, "paused");
}
#[test]
fn a_ride_start_is_an_engagement_and_its_settle_is_excluded() {
    // ADR-0002 §7.8 (c) as amended for #250: a ride start is an ERG engagement, its soft
    // start (§7.5) plus 5 s are settle. 10 s at 100 W against a 200 W target: all excluded.
    // The same rider after the settle is counted, and off target.
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 40, 100, 90);
    r.through(39);
    let c = r.status().compliance;
    assert_eq!((c.counted_s, c.within_s, c.excluded_s), (0, 0, 10), "{c:?}");
    pedal(&r, 40, 80, 100, 90);
    r.through(79);
    let c = r.status().compliance;
    assert_eq!(
        (c.counted_s, c.within_s, c.excluded_s),
        (10, 0, 10),
        "{c:?}"
    );
}

// -- rides on one engine share nothing (the policy is per ride) --

/// Run one ride on `r` starting at absolute tick `off`; returns the commands sent and
/// the final status. `stop_at`: end the ride (Stop) after that many ticks instead.
fn run_ride(
    r: &mut Rig,
    off: u64,
    blocks: Vec<crank_domain::workout::FlatBlock>,
    ticks: u64,
    power: u16,
    rpm: u8,
) -> (Vec<ControlMode>, crate::RideStatus) {
    r.modes(); // the previous ride's commands are not this ride's
    r.link(LinkState::Ready);
    r.at(off * TICK_US);
    r.load(blocks);
    r.start();
    pedal(r, off, off + ticks, power, rpm);
    r.through(off + ticks - 1);
    (r.modes(), r.status())
}
fn fresh_ride(
    blocks: Vec<crank_domain::workout::FlatBlock>,
    ticks: u64,
    power: u16,
    rpm: u8,
) -> (Vec<ControlMode>, crate::RideStatus) {
    run_ride(&mut rig(), 0, blocks, ticks, power, rpm)
}
fn same_as_fresh(
    got: &(Vec<ControlMode>, crate::RideStatus),
    blocks: Vec<crank_domain::workout::FlatBlock>,
    ticks: u64,
) {
    let (modes, st) = fresh_ride(blocks, ticks, 150, 90);
    assert_eq!(got.0, modes, "commands");
    assert_eq!(got.1.compliance, st.compliance, "compliance");
    assert_eq!(got.1.erg_released, st.erg_released);
}
#[test]
fn a_ride_stopped_mid_soft_start_leaves_nothing_to_the_next() {
    // ride 1: Free 10 s then Work 200 W, stopped at tick 49 inside the soft start
    let mut r = rig();
    r.load(vec![block(10.0, 0.0, 0.0, Phase::Free), work(200.0)]);
    r.start();
    pedal(&r, 0, 50, 100, 90);
    r.through(49);
    r.engine.command(EngineCmd::Stop).expect("stop");
    let second = run_ride(&mut r, 50, vec![work(300.0)], 60, 150, 90);
    same_as_fresh(&second, vec![work(300.0)], 60);
}
#[test]
fn a_ride_that_ends_released_leaves_the_next_unreleased() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 8, 190, 90);
    pedal(&r, 8, 24, 100, 20);
    r.through(23);
    assert!(r.status().erg_released);
    r.engine.command(EngineCmd::Stop).expect("stop");
    let second = run_ride(&mut r, 24, vec![work(200.0)], 40, 150, 90);
    assert!(!second.1.erg_released);
    assert!(!second.0.contains(&ControlMode::Resistance { pct: 0.0 }));
    same_as_fresh(&second, vec![work(200.0)], 40);
}
#[test]
fn a_new_ride_starts_with_an_empty_compliance_tally() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 80, 200, 90);
    r.through(79);
    assert_eq!(r.status().compliance.counted_s, 10);
    r.engine.command(EngineCmd::Stop).expect("stop");
    let second = run_ride(&mut r, 80, vec![work(200.0)], 8, 150, 90);
    assert_eq!(
        second.1.compliance.counted_s + second.1.compliance.excluded_s,
        2
    );
    same_as_fresh(&second, vec![work(200.0)], 8);
}

#[test]
fn resuming_after_exactly_t_unknown_keeps_the_episode_and_one_microsecond_later_does_not() {
    // C9 "within T_unknown(c)" and C1 "more than T_unknown(c)": the boundary is inclusive
    let t = C::Erg.t_unknown();
    let disagree = |m: &mut Machine, at: u64| {
        m.observe(&obs(at, Some(echo_erg(99)), Some(CommandStatus::Pass)), at);
    };
    let run = |back: u64| {
        let mut m = erg_machine(100);
        disagree(&mut m, SEC);
        disagree(&mut m, 3 * SEC);
        assert_eq!(m.verdict(), F::NotFollowing);
        m.set_desired(key(sim_mode(0.0)), 4 * SEC);
        m.set_desired(erg_key(100), 4 * SEC + back);
        m.verdict()
    };
    assert_eq!(
        run(t),
        F::NotFollowing,
        "exactly T_unknown: resumed, episode kept"
    );
    assert_eq!(
        run(t + 1),
        F::Checking,
        "one microsecond more: a fresh start (C1)"
    );
}

// ---- a link epoch never re-engages ERG by itself (architect decision, round 4) ----------

/// 300 W for 8 ticks at 90 rpm, then 20 rpm until released (8 ticks): returns the rig at
/// tick 16 with `erg_released` and the commands drained.
fn released_rig() -> Rig {
    let mut r = rig();
    r.load(vec![work(300.0)]);
    r.start();
    pedal(&r, 0, 8, 200, 90);
    pedal(&r, 8, 16, 150, 20);
    r.through(15);
    assert!(r.status().erg_released);
    assert_eq!(
        r.modes().last(),
        Some(&ControlMode::Resistance { pct: 0.0 })
    );
    r
}
fn positive_erg(modes: &[ControlMode]) -> Vec<u16> {
    modes
        .iter()
        .filter_map(|m| match m {
            ControlMode::Erg { target_w } if *target_w > 0 => Some(*target_w),
            _ => None,
        })
        .collect()
}
#[test]
fn a_ready_while_released_at_low_cadence_never_commands_erg() {
    // the falsifier's repro: Ready at 20 rpm used to command Erg{300} for ~2 s
    let mut r = released_rig();
    r.link(LinkState::Ready);
    pedal(&r, 16, 32, 150, 20);
    r.through(31);
    let sent = r.modes();
    assert!(positive_erg(&sent).is_empty(), "{sent:?}");
    assert!(r.status().erg_released);
    assert_eq!(
        sent.last(),
        Some(&ControlMode::Resistance { pct: 0.0 }),
        "re-issued"
    );
}
#[test]
fn a_bare_ready_while_released_leaves_release_only_through_the_cadence_criteria() {
    let mut r = released_rig();
    r.link(LinkState::Ready);
    // 90 rpm: 7 ticks are not 2 s, still released and nothing above 0 W
    pedal(&r, 16, 23, 150, 90);
    r.through(22);
    assert!(r.status().erg_released);
    assert!(positive_erg(&r.modes()).is_empty());
    // the eighth tick re-engages with the §7.5 soft start from the 3 s power (150 W)
    pedal(&r, 23, 48, 150, 90);
    r.through(47);
    assert!(!r.status().erg_released);
    let sent = positive_erg(&r.modes());
    assert_eq!(sent, vec![150, 180, 210, 240, 270, 300], "{sent:?}");
}
#[test]
fn connecting_then_ready_while_released_is_the_same() {
    let mut r = released_rig();
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    pedal(&r, 16, 30, 150, 20);
    r.through(29);
    assert!(r.status().erg_released);
    assert!(positive_erg(&r.modes()).is_empty());
}
#[test]
fn a_ready_in_the_middle_of_a_ramp_owes_a_new_soft_start() {
    let mut r = rig();
    r.load(vec![work(300.0)]);
    r.start();
    pedal(&r, 0, 8, 100, 90);
    r.through(7);
    r.modes();
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    pedal(&r, 8, 18, 100, 90); // 2.5 s into the ramp
    r.through(17);
    r.link(LinkState::Ready); // mid-ramp
    pedal(&r, 18, 26, 130, 90);
    r.through(25);
    let sent = positive_erg(&r.modes());
    // the first ramp (100 → …) was cut; the second restarts from the then-current 3 s power
    assert_eq!(sent.first(), Some(&100), "{sent:?}");
    let second: Vec<_> = sent
        .iter()
        .copied()
        .skip_while(|w| *w != 100)
        .skip(1)
        .collect();
    assert!(
        !second.contains(&300),
        "no jump to the target mid-ramp: {sent:?}"
    );
}

// ---- killers of the review's surviving mutants --------------------------------------------

fn policy_tally(power: f64, flow: Option<u64>, link_epoch: u64) -> crate::erg::Compliance {
    use crate::control::TargetPolicy;
    let b = block(600.0, 200.0, 200.0, Phase::Work);
    let mut p = crate::erg::ErgPolicy::settled();
    for k in 0..4 {
        let mut c = ctx_at(&b, k);
        c.cap_w = None;
        c.power_3s_w = Some(power);
        c.flow_start = flow;
        c.link_epoch = link_epoch;
        p.desired(&c);
    }
    p.status().compliance
}
#[test]
fn compliance_tolerance_is_inclusive_at_five_percent() {
    assert_eq!(policy_tally(210.0, Some(0), 0).within_s, 1, "exactly +5 %");
    assert_eq!(policy_tally(190.0, Some(0), 0).within_s, 1, "exactly -5 %");
    assert_eq!(policy_tally(210.5, Some(0), 0).within_s, 0);
    assert_eq!(policy_tally(210.0, Some(0), 0).counted_s, 1);
}
#[test]
fn seconds_waiting_at_zero_watts_do_not_count_against_the_block_target() {
    // a link epoch owes a soft start; nobody pedals: the engine holds 0 W
    let c = policy_tally(200.0, None, 1);
    assert_eq!((c.counted_s, c.excluded_s), (0, 1), "{c:?}");
}
#[test]
fn exactly_forty_rpm_is_not_low_cadence() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 40, 150, 40);
    r.through(39);
    assert!(!r.status().erg_released);
    pedal(&r, 80, 100, 150, 39);
    r.through(99);
    assert!(r.status().erg_released);
}
#[test]
fn the_first_tick_already_reports_checking_after_its_command() {
    // the mode sent on tick 0 (the start's ramp, from the 3 s power) activates the machine
    // on that same tick (second `follow`)
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    r.sample(0, 100, 90);
    r.through(0);
    assert_eq!(r.modes(), vec![erg(100)]);
    assert_eq!(r.status().trainer_following, F::Checking);
}
#[test]
fn a_silent_trainer_names_no_component() {
    // A2: every component silent is `unknown`, and nothing is blamed (A3 only)
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 60, 200, 90);
    r.through(59);
    let s = r.status();
    assert_eq!(s.trainer_following, F::Unknown);
    assert_eq!(s.not_following_component, None);
}
#[test]
fn every_ride_second_lands_in_exactly_one_compliance_bucket() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 20, 200, 90);
    r.through(19);
    r.link(LinkState::Lost);
    r.through(59); // 10 s without a link
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    pedal(&r, 60, 80, 200, 90);
    r.through(79);
    let st = r.status();
    assert_eq!(st.session.0, 80);
    let c = st.compliance;
    assert_eq!(c.counted_s + c.excluded_s, 20, "{c:?}");
    assert!(c.excluded_s >= 10, "the lost seconds are excluded: {c:?}");
}

// ---- release is judged against what is applied on the trainer (round 5) --------------------

fn last_mode(r: &mut Rig) -> Option<ControlMode> {
    r.modes().last().copied()
}
#[test]
fn stopping_during_a_post_loss_soft_start_releases() {
    // the falsifier's repro: 300 W, 100 W at 90 rpm, Connecting, Ready, 6 ticks, then 0 rpm
    let mut r = rig();
    r.load(vec![work(300.0)]);
    r.start();
    pedal(&r, 0, 8, 100, 90);
    r.through(7);
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    pedal(&r, 8, 14, 100, 90);
    r.through(13);
    r.modes();
    pedal(&r, 14, 120, 0, 0);
    r.through(119);
    assert!(r.status().erg_released, "torque held at 0 rpm for 26 s");
    assert_eq!(
        last_mode(&mut r),
        Some(ControlMode::Resistance { pct: 0.0 })
    );
}
#[test]
fn stopping_at_every_ramp_step_releases_within_two_seconds() {
    for stop in 9..8 + 26 {
        let mut r = rig();
        r.load(vec![work(300.0)]);
        r.start();
        pedal(&r, 0, 8, 100, 90);
        r.through(7);
        r.link(LinkState::Connecting);
        r.link(LinkState::Ready);
        pedal(&r, 8, stop, 100, 90);
        r.through(stop - 1);
        r.modes();
        pedal(&r, stop, stop + 12, 0, 0);
        r.through(stop + 11);
        assert!(r.status().erg_released, "stop at tick {stop}");
        let sent = r.modes();
        assert_eq!(
            sent.last(),
            Some(&ControlMode::Resistance { pct: 0.0 }),
            "stop at tick {stop}: {sent:?}"
        );
    }
}
#[test]
fn stopping_during_the_post_free_ramp_releases() {
    let mut r = rig();
    r.load(vec![block(600.0, 0.0, 0.0, Phase::Free), work(300.0)]);
    r.start();
    pedal(&r, 0, 8, 100, 90);
    r.through(7);
    r.modes();
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    pedal(&r, 8, 14, 100, 90); // two ramp steps in
    r.through(13);
    assert!(!positive_erg(&r.modes()).is_empty());
    pedal(&r, 14, 40, 0, 0);
    r.through(39);
    assert!(r.status().erg_released);
    assert_eq!(
        last_mode(&mut r),
        Some(ControlMode::Resistance { pct: 0.0 })
    );
}
#[test]
fn stopping_during_a_post_release_ramp_releases_again() {
    let mut r = released_rig(); // released at tick 15
    pedal(&r, 16, 24, 150, 90); // 8 ticks ≥ 60 rpm: re-engaged at tick 23
    r.through(23);
    assert!(!r.status().erg_released);
    pedal(&r, 24, 28, 150, 90); // one step into the ramp
    r.through(27);
    r.modes();
    pedal(&r, 28, 44, 0, 0);
    r.through(43);
    assert!(r.status().erg_released);
    assert_eq!(
        last_mode(&mut r),
        Some(ControlMode::Resistance { pct: 0.0 })
    );
}
#[test]
fn a_release_survives_a_free_block() {
    // released → Free → ERG at 20 rpm: stays released, no ERG torque (§7.4 leaves only by 60 rpm)
    let mut r = rig();
    r.load(vec![
        work(300.0),
        block(600.0, 0.0, 0.0, Phase::Free),
        work(300.0),
    ]);
    r.start();
    pedal(&r, 0, 8, 200, 90);
    pedal(&r, 8, 16, 150, 20);
    r.through(15);
    assert!(r.status().erg_released);
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    pedal(&r, 16, 24, 150, 20);
    r.through(23);
    r.modes();
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    pedal(&r, 24, 48, 150, 20);
    r.through(47);
    let sent = r.modes();
    assert!(r.status().erg_released);
    assert!(positive_erg(&sent).is_empty(), "{sent:?}");
}
#[test]
fn leaving_a_release_resets_the_ramp_so_the_target_is_never_a_jump() {
    // ramp started, cut by a release, re-engaged ≥ 20 ticks after it started: it restarts
    let mut r = rig();
    r.load(vec![work(300.0)]);
    r.start();
    pedal(&r, 0, 8, 100, 90);
    r.through(7);
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    pedal(&r, 8, 10, 100, 90); // ramp starts at tick 8
    pedal(&r, 10, 24, 100, 20); // released
    pedal(&r, 24, 40, 100, 90); // re-engaged at tick 31
    r.through(39);
    let sent = positive_erg(&r.modes());
    let after = &sent[sent.len().saturating_sub(3)..];
    assert!(sent.iter().all(|w| *w <= 300) && !after.is_empty());
    let first_after_release = sent.iter().rev().take_while(|w| **w != 100).count();
    assert!(
        first_after_release <= 2,
        "restarts from 100 W, not the target: {sent:?}"
    );
    assert_ne!(
        sent.get(sent.len().saturating_sub(first_after_release + 1)),
        Some(&300)
    );
}
#[test]
fn old_link_cadence_does_not_count_toward_re_engagement() {
    let mut r = released_rig();
    pedal(&r, 16, 19, 150, 90); // 3 ticks ≥ 60 rpm
    r.through(18);
    r.link(LinkState::Ready); // stamped in tick 19
    pedal(&r, 19, 24, 150, 90); // 5 more: 8 in total, but only 5 on the new epoch
    r.through(23);
    assert!(r.status().erg_released, "needs 8 ticks on the new link");
    pedal(&r, 24, 27, 150, 90);
    r.through(26);
    assert!(!r.status().erg_released);
}

#[test]
fn a_finished_ride_accounts_its_final_second() {
    let mut r = rig();
    r.load(vec![block(15.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    pedal(&r, 0, 60, 200, 90);
    r.through(59);
    let st = r.status();
    assert_eq!(st.state, crate::RideState::Finished);
    let c = st.compliance;
    // 10 s of the start's settle, then 5 counted seconds, the last one the finishing tick's
    assert_eq!((c.counted_s, c.within_s, c.excluded_s), (5, 5, 10), "{c:?}");
}
#[test]
fn the_re_engagement_criteria_are_evaluated_in_a_free_block_too() {
    // released, then 14 ticks ≥ 60 rpm inside a Free block: the release is left there
    // (§7.4), so the next ERG block opens with the soft start, not with Resistance.
    let mut r = rig();
    r.load(vec![
        work(300.0),
        block(600.0, 0.0, 0.0, Phase::Free),
        work(300.0),
    ]);
    r.start();
    pedal(&r, 0, 8, 200, 90);
    pedal(&r, 8, 16, 150, 20);
    r.through(15);
    assert!(r.status().erg_released);
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    pedal(&r, 16, 30, 150, 90);
    r.through(29);
    r.modes();
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    pedal(&r, 30, 34, 150, 90);
    r.through(33);
    let sent = r.modes();
    assert!(!r.status().erg_released);
    assert_eq!(positive_erg(&sent).first(), Some(&150), "{sent:?}");
}

// -- ride start, user pause and ride end (#250, #264, #266) --

const RESISTANCE_0: ControlMode = ControlMode::Resistance { pct: 0.0 };

#[test]
fn a_ride_start_holds_zero_watts_until_cadence_then_ramps_from_the_3s_power() {
    // ADR-0002 §7.5 as amended for #250: a ride start is a reconnect. No torque for a rider
    // who is not pedalling, then five 1 Hz steps from the 3 s power to the block target.
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 8, 0, 0);
    r.through(7);
    assert_eq!(r.modes(), vec![erg(0)], "0 W, sent once, never the target");
    pedal(&r, 8, 32, 100, 90);
    r.through(31);
    // the origin is the 3 s mean at the first pedalling tick: eight 0 W samples and 100 W
    assert_eq!(
        r.modes(),
        vec![erg(11), erg(49), erg(87), erg(124), erg(162), erg(200)]
    );
}
#[test]
fn a_free_first_block_is_unchanged_and_the_next_erg_block_owes_the_soft_start() {
    // ADR-0002 §7.5 item 5: leaving free/SIM owes the soft start; the start itself has no ERG.
    let mut r = rig();
    r.load(vec![block(600.0, 0.0, 0.0, Phase::Free), work(200.0)]);
    r.start();
    pedal(&r, 0, 8, 150, 90);
    r.through(7);
    assert_eq!(r.modes(), vec![ControlMode::Free]);
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    pedal(&r, 8, 16, 150, 90);
    r.through(15);
    assert_eq!(r.modes(), vec![erg(150), erg(160)]);
}
#[test]
fn a_user_pause_commands_zero_resistance_and_the_resume_soft_starts() {
    // DESIGN §5.2 "Paused (user)": 0 % resistance, ERG re-engages with the soft start (#264)
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 24, 200, 90);
    r.through(23);
    assert_eq!(r.modes().last(), Some(&erg(200)));
    r.engine.command(EngineCmd::Pause).expect("pause");
    // the rider keeps pedalling at 150 W: no command until the resume
    pedal(&r, 24, 40, 150, 90);
    r.through(39);
    assert_eq!(r.modes(), vec![RESISTANCE_0]);
    assert_eq!(
        r.state(),
        crate::RideState::Paused(crate::PauseReason::User)
    );
    r.engine.command(EngineCmd::Resume).expect("resume");
    pedal(&r, 40, 60, 150, 90);
    r.through(59);
    assert_eq!(
        r.modes(),
        vec![erg(150), erg(160), erg(170), erg(180), erg(190)],
        "the soft start from the 3 s power, not the 200 W target"
    );
}
#[test]
fn a_resume_without_cadence_holds_zero_watts_until_the_rider_pedals() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 24, 200, 90);
    r.through(23);
    r.modes();
    r.engine.command(EngineCmd::Pause).expect("pause");
    pedal(&r, 24, 28, 0, 0);
    r.through(27);
    assert_eq!(r.modes(), vec![RESISTANCE_0]);
    r.engine.command(EngineCmd::Resume).expect("resume");
    pedal(&r, 28, 32, 0, 0);
    r.through(31);
    assert_eq!(
        r.modes(),
        vec![erg(0)],
        "no torque for a rider who is not pedalling"
    );
    pedal(&r, 32, 36, 120, 90);
    r.through(35);
    // origin: the 3 s mean (12 ticks) = (3 x 200 W + 8 x 0 W + 120 W) / 12
    assert_eq!(r.modes(), vec![erg(60)]);
}
#[test]
fn a_release_survives_a_user_pause_and_is_left_only_by_sixty_rpm() {
    // #249 ruling: a release is left only through §7.4, also across a pause (#264). The
    // pause is applied by ticks (a pause and resume between two ticks change nothing).
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 8, 190, 90);
    pedal(&r, 8, 24, 100, 20);
    r.through(23);
    assert!(r.status().erg_released);
    r.modes();
    r.engine.command(EngineCmd::Pause).expect("pause");
    pedal(&r, 24, 32, 100, 20);
    r.through(31);
    assert!(r.status().erg_released, "paused: still released");
    r.engine.command(EngineCmd::Resume).expect("resume");
    pedal(&r, 32, 48, 100, 20);
    r.through(47);
    assert!(r.status().erg_released, "20 rpm does not re-engage");
    assert!(
        r.modes().iter().all(|m| *m == RESISTANCE_0),
        "no ERG after the resume while released"
    );
    pedal(&r, 48, 56, 100, 90);
    r.through(55);
    assert!(!r.status().erg_released);
    assert_eq!(r.modes(), vec![erg(100)], "re-engaged with the soft start");
}
#[test]
fn stopped_pedalling_leaves_no_torque_on_the_trainer() {
    // auto-pause (cadence and power 0 for 3 s): the §7.4 release is out before the pause
    // begins, so DESIGN §5.2's 0 % resistance already holds (#264)
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 24, 200, 90);
    r.through(23);
    r.modes();
    pedal(&r, 24, 40, 0, 0);
    r.through(39);
    assert_eq!(
        r.state(),
        crate::RideState::Paused(crate::PauseReason::StoppedPedalling)
    );
    assert_eq!(r.modes(), vec![RESISTANCE_0]);
}
#[test]
fn a_finished_ride_commands_the_end_mode_before_the_gate_clears() {
    // ADR-0002 §7.9 / #266: "silence after ERG could leave the ERG target active"
    let mut r = rig();
    r.load(vec![block(2.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    pedal(&r, 0, 8, 200, 90);
    r.through(7);
    assert_eq!(r.state(), crate::RideState::Finished);
    assert_eq!(r.modes().last(), Some(&RESISTANCE_0));
    assert_eq!(
        r.gate.lock().unwrap().last(),
        Some(&true),
        "waits for delivery"
    );
    r.delivered(RESISTANCE_0);
    assert_eq!(r.gate.lock().unwrap().last(), Some(&false));
}
#[test]
fn stopping_mid_ride_commands_the_end_mode_before_the_gate_clears() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 24, 200, 90);
    r.through(23);
    assert_eq!(r.modes().last(), Some(&erg(200)));
    r.engine.command(EngineCmd::Stop).expect("stop");
    assert_eq!(r.modes(), vec![RESISTANCE_0]);
    assert_eq!(r.gate.lock().unwrap().last(), Some(&true));
    r.delivered(RESISTANCE_0);
    assert_eq!(r.gate.lock().unwrap().last(), Some(&false));
}
#[test]
fn the_gate_does_not_wait_for_a_trainer_that_is_gone_or_silent() {
    // link lost while the end mode is in flight: it cannot be delivered
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 8, 200, 90);
    r.through(7);
    r.engine.command(EngineCmd::Stop).expect("stop");
    assert_eq!(r.gate.lock().unwrap().last(), Some(&true));
    r.link(LinkState::Lost);
    assert_eq!(r.gate.lock().unwrap().last(), Some(&false));
    // and a driver that never answers is given 2 s, not forever
    r.link(LinkState::Ready);
    r.at(8 * TICK_US);
    r.load(vec![work(200.0)]);
    r.start();
    r.engine.command(EngineCmd::Stop).expect("stop");
    assert_eq!(r.gate.lock().unwrap().last(), Some(&true));
    r.at(8 * TICK_US + 1_999_000);
    r.engine.catch_up();
    assert_eq!(
        r.gate.lock().unwrap().last(),
        Some(&true),
        "inside the bound"
    );
    r.at(8 * TICK_US + 2_000_000);
    r.engine.catch_up();
    assert_eq!(r.gate.lock().unwrap().last(), Some(&false));
}
#[test]
fn an_unapplied_link_transition_owes_the_end_mode_again() {
    // a transition wipes the trainer's mode (ADR-0002 §6): the end mode is sent although the
    // engine's last command is the same 0 % resistance (a release)
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 8, 190, 90);
    pedal(&r, 8, 24, 100, 20);
    r.through(23);
    assert!(r.status().erg_released);
    r.modes();
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    r.engine.command(EngineCmd::Stop).expect("stop");
    assert_eq!(r.modes(), vec![RESISTANCE_0]);
}
#[test]
fn a_late_delivery_of_an_earlier_mode_does_not_clear_the_gate() {
    // only the end mode's own delivery releases the gate
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 24, 200, 90);
    r.through(23);
    r.engine.command(EngineCmd::Stop).expect("stop");
    // (a driver reports only commands it was given: `Free` was never sent)
    r.delivered(erg(200));
    assert_eq!(r.gate.lock().unwrap().last(), Some(&true));
    r.delivered(RESISTANCE_0);
    assert_eq!(r.gate.lock().unwrap().last(), Some(&false));
}
#[test]
fn a_late_delivery_of_an_earlier_equal_mode_does_not_clear_the_gate() {
    // #345: the end mode is confirmed by its own sequence, never by an equal mode: the
    // pause's `Resistance{0}`, delivered late, says nothing about the end mode's
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 24, 200, 90);
    r.through(23);
    assert_eq!(r.modes().last(), Some(&erg(200)));
    r.engine.command(EngineCmd::Pause).expect("pause");
    pedal(&r, 24, 28, 150, 90);
    r.through(27);
    assert_eq!(r.modes(), vec![RESISTANCE_0]);
    let (_, paused) = *r.sent.last().unwrap();
    r.engine.command(EngineCmd::Stop).expect("stop");
    assert_eq!(r.modes(), vec![RESISTANCE_0]);
    let (_, ended) = *r.sent.last().unwrap();
    assert_ne!(paused, ended);
    r.delivered_seq(RESISTANCE_0, paused);
    assert_eq!(
        r.gate.lock().unwrap().last(),
        Some(&true),
        "a stale delivery confirmed the end mode"
    );
    r.delivered_seq(RESISTANCE_0, ended);
    assert_eq!(r.gate.lock().unwrap().last(), Some(&false));
}
#[test]
fn leaving_free_after_a_repaid_soft_start_owes_it_again() {
    // ADR-0002 §7.5: the start's soft start is repaid at tick 20; a free block in between
    // and then ERG again must ramp from the 3 s power, not step to the target (#250)
    let mut r = rig();
    r.load(vec![
        work(200.0),
        block(600.0, 0.0, 0.0, Phase::Free),
        work(200.0),
    ]);
    r.start();
    pedal(&r, 0, 24, 100, 90);
    r.through(23);
    assert_eq!(
        r.modes().last(),
        Some(&erg(200)),
        "the start's ramp is repaid"
    );
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    pedal(&r, 24, 28, 100, 90);
    r.through(27);
    assert_eq!(r.modes(), vec![ControlMode::Free]);
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    pedal(&r, 28, 36, 100, 90);
    r.through(35);
    assert_eq!(r.modes(), vec![erg(100), erg(120)]);
}
#[test]
fn a_load_keeps_the_previous_end_mode_owed_and_holds_the_new_ride_back() {
    // #279: Stop → Load → Start. The end mode stays owed, the gate stays up through the
    // delivery (a ride is loaded), and nothing is sent for the new ride before it.
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 24, 200, 90);
    r.through(23);
    r.modes();
    r.engine.command(EngineCmd::Stop).expect("stop");
    assert_eq!(r.modes(), vec![RESISTANCE_0]);
    r.load(vec![work(200.0)]);
    r.at(24 * TICK_US);
    r.start();
    pedal(&r, 24, 40, 150, 90);
    r.through(29); // 1.5 s: inside the 2 s deadline
    assert!(
        r.modes().is_empty(),
        "held back until the end mode is delivered"
    );
    assert_eq!(r.gate.lock().unwrap().last(), Some(&true));
    r.delivered(RESISTANCE_0);
    assert_eq!(
        r.gate.lock().unwrap().last(),
        Some(&true),
        "the new ride is open"
    );
    r.through(39);
    assert_eq!(r.modes().first(), Some(&erg(150)), "its own soft start");
}
#[test]
fn unloading_a_loaded_ride_waits_for_the_previous_end_mode() {
    let mut r = rig();
    r.load(vec![work(200.0)]);
    r.start();
    pedal(&r, 0, 8, 200, 90);
    r.through(7);
    r.engine.command(EngineCmd::Stop).expect("stop");
    r.load(vec![work(200.0)]);
    r.engine.command(EngineCmd::Stop).expect("unload");
    assert_eq!(
        r.gate.lock().unwrap().last(),
        Some(&true),
        "end mode in flight"
    );
    r.delivered(RESISTANCE_0);
    assert_eq!(r.gate.lock().unwrap().last(), Some(&false));
}
