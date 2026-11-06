//! The keeper's transitions on their own: time is an input, so every timer is checked to
//! the second with no sleep. Each test names the ADR-0002 §5.1.8 bullet it covers; the same
//! bullets are covered again through the core, the drivers and the clock in `core_tests.rs`.
use super::*;
use LinkDevice::{Strap, Trainer};

const RIDER: RiderConfig = RiderConfig {
    rider_kg: 72.0,
    bike_kg: 9.0,
    wheel_m: 2.096,
};
/// [`Keeper::step`] with the plain `Ack(Applied)` of an intent left out: the tests below assert
/// the other effects, `the_transition_table_is_total` every answer.
trait Steps {
    fn st(&mut self, now: MonoTime, input: Input<'_>) -> Vec<Effect>;
}
impl Steps for Keeper {
    fn st(&mut self, now: MonoTime, input: Input<'_>) -> Vec<Effect> {
        let mut effects = self.step(now, input);
        effects.retain(|e| *e != Effect::Ack(Ack::Applied));
        effects
    }
}
fn at(s: u64) -> MonoTime {
    MonoTime(s * 1_000_000)
}
fn likely() -> Input<'static> {
    Input::Intent(LinkIntent::Likely { ready: false })
}
fn release(reason: ReleaseReason) -> Input<'static> {
    Input::Intent(LinkIntent::Release { reason })
}
/// A keeper with a trainer and a strap paired.
fn paired() -> Keeper {
    let mut k = Keeper::new(RIDER);
    k.st(
        at(0),
        Input::Pairings([Some("neo".into()), Some("hrm".into())]),
    );
    k
}
/// Both devices warm at 5 s after a `Likely` at 0 s.
fn warm() -> Keeper {
    let mut k = paired();
    k.st(at(0), likely());
    k.st(at(5), Input::Attempted(Trainer, 1, true));
    k.st(at(5), Input::Attempted(Strap, 1, true));
    k
}
fn phases(k: &Keeper) -> [Option<Phase>; 2] {
    [k.phase(Trainer), k.phase(Strap)]
}
fn cools(effects: &[Effect]) -> Vec<(LinkDevice, Phase, Cool)> {
    effects
        .iter()
        .filter_map(|e| match *e {
            Effect::Cool {
                device,
                was,
                reason,
            } => Some((device, was, reason)),
            _ => None,
        })
        .collect()
}
/// Step through `from..=to` one second at a time; the effects of each tick.
fn run(k: &mut Keeper, from: u64, to: u64) -> Vec<(u64, Vec<Effect>)> {
    (from..=to)
        .map(|s| (s, k.st(at(s), Input::Tick)))
        .filter(|(_, e)| !e.is_empty())
        .collect()
}
/// A ride loaded at `s` over both warm devices.
fn loaded(s: u64) -> Keeper {
    let mut k = warm();
    assert_eq!(k.st(at(s), Input::Acquire), [Effect::HandOver]);
    k.st(at(s), Input::Loaded("ride".into()));
    k
}

/// §5.1.2: `Likely` attempts both paired devices at once; an unpaired one has no phase.
#[test]
fn likely_starts_both_attempts_in_parallel() {
    let mut k = paired();
    assert_eq!(
        k.st(at(0), likely()),
        [Effect::Attempt(Trainer, 1), Effect::Attempt(Strap, 1)]
    );
    let mut alone = Keeper::new(RIDER);
    alone.st(at(0), Input::Pairings([Some("neo".into()), None]));
    assert_eq!(alone.st(at(0), likely()), [Effect::Attempt(Trainer, 1)]);
    assert_eq!(alone.phase(Strap), None);
}

/// §5.1.2: a refresh of a standing `Likely` retries no `not_found` device (no connect loop);
/// a fresh one does, and so does Retry.
#[test]
fn only_a_fresh_likely_retries_a_device_not_found() {
    let mut k = paired();
    k.st(at(0), likely());
    k.st(at(4), Input::Attempted(Trainer, 1, false));
    k.st(at(4), Input::Attempted(Strap, 1, true));
    assert_eq!(k.st(at(30), likely()), []);
    k.st(at(31), Input::Intent(LinkIntent::Left));
    assert_eq!(k.st(at(32), likely()), [Effect::Attempt(Trainer, 2)]);
    k.st(at(36), Input::Attempted(Trainer, 2, false));
    assert_eq!(
        k.st(at(40), Input::Intent(LinkIntent::Retry)),
        [Effect::Attempt(Trainer, 3)]
    );
}

/// §5.1.8 "Idle cool-down disconnects": after `Left` the devices are still warm at
/// `T_cool` − 1 s and released at `T_cool`; a `Likely` in between restarts the timer.
#[test]
fn left_cools_down_at_t_cool_and_a_likely_restarts_it() {
    let mut k = warm();
    k.st(at(10), Input::Intent(LinkIntent::Left));
    assert_eq!(run(&mut k, 11, 309), []);
    assert_eq!(phases(&k), [Some(Phase::Warm); 2]);
    assert_eq!(k.next_wake(at(309)), Some(at(310)));
    let cooled = k.st(at(310), Input::Tick);
    assert_eq!(
        cools(&cooled),
        [
            (Trainer, Phase::Warm, Cool::Idle),
            (Strap, Phase::Warm, Cool::Idle)
        ]
    );
    assert_eq!(phases(&k), [Some(Phase::Cold); 2]);

    let mut k = warm();
    k.st(at(10), Input::Intent(LinkIntent::Left));
    k.st(at(200), likely());
    k.st(at(201), Input::Intent(LinkIntent::Left));
    assert_eq!(run(&mut k, 202, 500), []);
    assert_eq!(cools(&k.st(at(501), Input::Tick)).len(), 2);
}

/// §5.1.3: on a warm view, `T_cool` without rider input (the throttled refresh) cools down
/// too; each refresh restarts it.
#[test]
fn a_warm_view_without_input_cools_down_at_t_cool() {
    let mut k = warm();
    k.st(at(100), likely());
    assert_eq!(run(&mut k, 101, 399), []);
    assert_eq!(cools(&k.st(at(400), Input::Tick)).len(), 2);
}

/// §5.1.8 "Background grace": still warm at 59 s, released at 60 s; a `Likely` at 30 s
/// cancels the release.
#[test]
fn the_background_grace_is_sixty_seconds_and_a_likely_cancels_it() {
    let mut k = warm();
    k.st(at(10), Input::Intent(LinkIntent::Background));
    assert_eq!(run(&mut k, 11, 69), []);
    assert_eq!(
        cools(&k.st(at(70), Input::Tick))
            .iter()
            .map(|c| c.2)
            .collect::<Vec<_>>(),
        [Cool::Background; 2]
    );

    let mut k = warm();
    k.st(at(10), Input::Intent(LinkIntent::Background));
    k.st(at(40), likely());
    assert_eq!(run(&mut k, 41, 120), []);
    assert_eq!(phases(&k), [Some(Phase::Warm); 2]);
}

/// §5.1.8 "Idle cool-down disconnects": every `Release` reason lets go at once, an attempt
/// still running included.
#[test]
fn every_release_reason_lets_go_at_once_mid_attempt_included() {
    use ReleaseReason::{Devices, Hidden, RiderRelease, Sleep};
    for reason in [Hidden, Sleep, Devices, RiderRelease] {
        let mut k = paired();
        k.st(at(0), likely());
        k.st(at(1), Input::Attempted(Trainer, 1, true));
        k.st(at(1), Input::Phase(Strap, 1, Phase::Connecting));
        assert_eq!(
            cools(&k.st(at(2), release(reason))),
            [
                (Trainer, Phase::Warm, Cool::Release(reason)),
                (Strap, Phase::Connecting, Cool::Release(reason)),
            ]
        );
        assert_eq!(phases(&k), [Some(Phase::Cold); 2]);
        // the cancelled attempt's late result is stale
        assert_eq!(k.st(at(3), Input::Attempted(Strap, 1, true)), []);
        assert_eq!(k.phase(Strap), Some(Phase::Cold));
    }
}

/// §5.1.8 "Idle cool-down disconnects": the latch blocks `Likely` for the trainer until Retry
/// or "Ride this".
#[test]
fn the_rider_release_latch_holds_until_retry_or_a_load() {
    let mut k = warm();
    k.st(at(10), release(ReleaseReason::RiderRelease));
    // (a cool-down ends an attempt's era too: these are the third)
    assert_eq!(k.st(at(20), likely()), [Effect::Attempt(Strap, 3)]);
    assert_eq!(k.phase(Trainer), Some(Phase::Cold));
    assert_eq!(
        k.st(at(30), Input::Intent(LinkIntent::Retry)),
        [Effect::Attempt(Trainer, 3)]
    );

    let mut k = warm();
    k.st(at(10), release(ReleaseReason::RiderRelease));
    k.st(at(20), Input::Intent(LinkIntent::Left));
    assert_eq!(
        k.st(at(30), Input::Acquire),
        [Effect::Attempt(Trainer, 3), Effect::Attempt(Strap, 3)]
    );
}

/// §5.1.8 "The Ready sheet holds": with a failed load and `Likely { ready: true }`, no input
/// for 30 min leaves the keeper's links warm; hiding still releases them.
#[test]
fn the_ready_sheet_holds_without_input_until_hidden() {
    let mut k = paired();
    k.st(at(0), likely());
    k.st(at(0), Input::Acquire);
    k.st(at(1), Input::Attempted(Strap, 1, true));
    assert_eq!(
        k.st(at(15), Input::Attempted(Trainer, 1, false)),
        [Effect::FailLoad]
    );
    k.st(at(15), Input::Intent(LinkIntent::Likely { ready: true }));
    assert_eq!(
        k.next_wake(at(15)),
        Some(at(45)),
        "only the retry scan is due"
    );
    assert!(
        run(&mut k, 16, 15 + 30 * 60)
            .iter()
            .all(|(_, e)| cools(e).is_empty())
    );
    assert_eq!(k.phase(Strap), Some(Phase::Warm));
    assert_eq!(
        cools(&k.st(at(1900), release(ReleaseReason::Hidden))).len(),
        1
    );
}

/// §5.1.4: a load takes warm links at once; a running attempt is awaited, never doubled; a
/// cold device is connected now.
#[test]
fn a_load_takes_warm_links_awaits_running_attempts_and_connects_cold_ones() {
    let mut k = warm();
    assert_eq!(k.st(at(6), Input::Acquire), [Effect::HandOver]);
    assert_eq!(phases(&k), [Some(Phase::Riding); 2]);

    let mut k = paired();
    k.st(at(0), likely());
    k.st(at(1), Input::Phase(Trainer, 1, Phase::Handshake));
    assert_eq!(k.st(at(2), Input::Acquire), [], "both attempts are awaited");
    k.st(at(3), Input::Attempted(Trainer, 1, true));
    assert_eq!(
        k.st(at(4), Input::Attempted(Strap, 1, false)),
        [Effect::HandOver]
    );
    assert_eq!(phases(&k), [Some(Phase::Riding), Some(Phase::NotFound)]);

    let mut k = paired();
    assert_eq!(
        k.st(at(0), Input::Acquire),
        [Effect::Attempt(Trainer, 1), Effect::Attempt(Strap, 1)]
    );
}

/// §5.1.4: a trainer that fails fails the load at once, beside a strap still connecting,
/// and the strap stays the keeper's.
#[test]
fn a_failed_trainer_fails_the_load_at_once_and_the_strap_stays_with_the_keeper() {
    let mut k = paired();
    k.st(at(0), Input::Acquire);
    assert_eq!(
        k.st(at(3), Input::Attempted(Trainer, 1, false)),
        [Effect::FailLoad]
    );
    k.st(at(5), Input::Attempted(Strap, 1, true));
    assert_eq!(phases(&k), [Some(Phase::NotFound), Some(Phase::Warm)]);
}

/// §5.1.8 "A loaded ride keeps its links": `Hidden`, `Background` past 60 s and `T_cool`
/// past 5 min leave a loaded, unstarted ride's links alone.
#[test]
fn a_loaded_ride_keeps_its_links_through_hiding_background_and_idle() {
    let mut k = loaded(6);
    for input in [
        release(ReleaseReason::Hidden),
        release(ReleaseReason::Devices),
        Input::Intent(LinkIntent::Background),
        Input::Intent(LinkIntent::Left),
    ] {
        assert_eq!(k.st(at(7), input), []);
    }
    // input keeps the ride from being abandoned; nothing else is due
    for s in (60..=1800).step_by(60) {
        k.st(at(s), likely());
        assert_eq!(k.st(at(s), Input::Tick), []);
    }
    assert_eq!(phases(&k), [Some(Phase::Riding); 2]);
}

/// §5.1.8 "An abandoned load releases": discarded at `T_abandon` without input; input at
/// 9 min restarts the timer.
#[test]
fn an_abandoned_load_is_discarded_at_t_abandon_and_input_restarts_it() {
    let mut k = loaded(10);
    assert_eq!(run(&mut k, 11, 609), []);
    assert_eq!(k.st(at(610), Input::Tick), [Effect::Discard("ride".into())]);
    assert_eq!(run(&mut k, 611, 2000), [], "asked for once");
    k.st(at(611), Input::Released);
    assert_eq!(phases(&k), [Some(Phase::Cold); 2]);

    let mut k = loaded(10);
    k.st(at(550), likely());
    assert_eq!(run(&mut k, 11, 1149), []);
    assert_eq!(
        k.st(at(1150), Input::Tick),
        [Effect::Discard("ride".into())]
    );
    // a started ride is never abandoned
    let mut k = loaded(10);
    k.st(at(20), Input::Started("ride".into()));
    assert_eq!(run(&mut k, 21, 5000), []);
}

/// §5.1.8 "Sleep discards an unstarted ride", and the architect's decision on "Release
/// trainer" with a ride loaded (ADR-0002 §5.1.3): both discard an unstarted ride ("Release
/// trainer" also latches the trainer); a started ride is untouched by either.
#[test]
fn sleep_and_rider_release_discard_only_an_unstarted_ride() {
    for reason in [ReleaseReason::Sleep, ReleaseReason::RiderRelease] {
        let mut k = loaded(10);
        assert_eq!(
            k.st(at(20), release(reason)),
            [
                Effect::Discard("ride".into()),
                Effect::Ack(Ack::AwaitDiscard)
            ]
        );
        k.st(at(21), Input::Discarded("ride".into(), true));
        k.st(at(21), Input::Released);
        let latched = reason == ReleaseReason::RiderRelease;
        let expect: &[Effect] = if latched {
            &[Effect::Attempt(Strap, 2)]
        } else {
            &[Effect::Attempt(Trainer, 2), Effect::Attempt(Strap, 2)]
        };
        assert_eq!(k.st(at(30), likely()), expect, "{reason:?}");

        let mut k = loaded(10);
        k.st(at(15), Input::Started("ride".into()));
        let refused: &[Effect] = if latched {
            &[Effect::Ack(Ack::Refused(RIDE_HOLDS_TRAINER))]
        } else {
            &[]
        };
        assert_eq!(k.st(at(20), release(reason)), refused);
        assert_eq!(phases(&k), [Some(Phase::Riding); 2]);
    }
}

/// §5.1.8 "`Likely` during a ride warms nothing": with a ride loaded or started and the strap
/// not linked, `Likely` and Retry start no attempt.
#[test]
fn likely_during_a_ride_warms_nothing() {
    let mut k = paired();
    k.st(at(0), Input::Acquire);
    k.st(at(1), Input::Attempted(Strap, 1, false));
    k.st(at(2), Input::Attempted(Trainer, 1, true));
    k.st(at(2), Input::Loaded("ride".into()));
    for started in [false, true] {
        if started {
            k.st(at(3), Input::Started("ride".into()));
        }
        assert_eq!(k.st(at(4), likely()), []);
        assert_eq!(k.st(at(4), Input::Intent(LinkIntent::Retry)), []);
        assert_eq!(k.st(at(5), Input::Advertised("hrm")), []);
        assert_eq!(k.scanning(at(5)), None);
    }
}

/// §5.1.8 "Scan cap": with the trainer never advertising and intent `Likely` for 10 min,
/// the scan is continuous for `SCAN_CONTINUOUS`, then on for `RESOLVE_SCAN` in every
/// `SCAN_PERIOD`; it stops on `Left`; an advertisement in a window starts the attempt.
#[test]
fn the_scan_is_capped_and_stops_on_left() {
    let mut k = paired();
    k.st(at(0), likely());
    k.st(at(0), Input::Attempted(Strap, 1, true));
    k.st(at(4), Input::Attempted(Trainer, 1, false));
    let mut on = Vec::new();
    for s in 4..604 {
        if s % 30 == 0 {
            k.st(at(s), likely());
        }
        if k.scanning(at(s)).is_some() {
            on.push(s);
        }
    }
    let mut expect: Vec<u64> = (4..34).collect();
    for window in (64..604).step_by(30) {
        expect.extend(window..window + 4);
    }
    assert_eq!(on, expect);
    assert_eq!(k.scanning(at(5)), Some(true));
    assert_eq!(k.scanning(at(64)), Some(false));
    assert_eq!(k.next_wake(at(600)), Some(at(604)), "the next window");
    // an advertisement starts the attempt, and the scan stops while it runs
    assert_eq!(
        k.st(at(605), Input::Advertised("neo")),
        [Effect::Attempt(Trainer, 2)]
    );
    assert_eq!(k.scanning(at(605)), None);
    k.st(at(609), Input::Attempted(Trainer, 2, false));
    // the failure of an attempt an advertisement started does not restart the cap (N3)
    assert_eq!(k.scanning(at(610)), None);
    assert_eq!(k.scanning(at(634)), Some(false), "the next window");
    // `Left` in a window stops it
    k.st(at(635), Input::Intent(LinkIntent::Left));
    assert_eq!(k.scanning(at(635)), None);
    assert_eq!(k.st(at(636), Input::Advertised("neo")), []);
}

/// §5.1.2: a change of rider configuration while the trainer is warm reconnects it.
#[test]
fn a_rider_change_reconnects_a_warm_trainer() {
    let mut k = warm();
    let heavier = RiderConfig {
        rider_kg: 80.0,
        ..RIDER
    };
    let effects = k.st(at(10), Input::Rider(heavier));
    assert_eq!(cools(&effects), [(Trainer, Phase::Warm, Cool::Rider)]);
    assert_eq!(effects.last(), Some(&Effect::Attempt(Trainer, 3)));
    assert_eq!(k.rider(), heavier);
    assert_eq!(k.st(at(11), Input::Rider(heavier)), []);
}

/// §5.1.8 "Handover": a warm link that is lost makes the next load connect cold; a ride's end
/// leaves the phase `cold`, never `warm`; links a cancelled load never took are warm again.
#[test]
fn a_lost_link_connects_cold_at_the_load_and_a_ride_end_leaves_cold() {
    let mut k = warm();
    assert_eq!(
        cools(&k.st(at(10), Input::Lost(Trainer, 1))),
        [(Trainer, Phase::Warm, Cool::Lost)]
    );
    assert_eq!(k.phase(Trainer), Some(Phase::NotFound));
    assert_eq!(k.st(at(11), Input::Acquire), [Effect::Attempt(Trainer, 3)]);
    k.st(at(12), Input::Attempted(Trainer, 3, true));
    assert_eq!(phases(&k), [Some(Phase::Riding); 2]);
    k.st(at(13), Input::Loaded("ride".into()));
    k.st(at(14), Input::Released);
    assert_eq!(phases(&k), [Some(Phase::Cold); 2]);
    assert_eq!(k.st(at(15), Input::Tick), [], "no intent after a ride");

    let mut k = warm();
    k.st(at(6), Input::Acquire);
    k.st(at(7), Input::Refused);
    assert_eq!(phases(&k), [Some(Phase::Warm); 2]);
}

/// §5.1.3: quitting lets every keeper link go and takes no new one.
#[test]
fn shutdown_lets_go_of_everything_and_takes_nothing_new() {
    let mut k = paired();
    k.st(at(0), likely());
    k.st(at(1), Input::Attempted(Trainer, 1, true));
    assert_eq!(
        cools(&k.st(at(2), Input::Shutdown)),
        [
            (Trainer, Phase::Warm, Cool::Shutdown),
            (Strap, Phase::Searching, Cool::Shutdown)
        ]
    );
    assert_eq!(k.st(at(3), likely()), [Effect::Ack(Ack::Refused(QUITTING))]);
    assert_eq!(k.st(at(3), Input::Acquire), [Effect::FailLoad]);
}

/// §5.1.2: a pairing that changes lets the device go; one that is forgotten has no phase.
#[test]
fn a_changed_pairing_lets_the_device_go() {
    let mut k = warm();
    let effects = k.st(at(10), Input::Pairings([Some("neo".into()), None]));
    assert_eq!(cools(&effects), [(Strap, Phase::Warm, Cool::Unpaired)]);
    assert_eq!(phases(&k), [Some(Phase::Warm), None]);
}

/// §5.1.4: a load that comes while the links are still handed to an ended load (its release or
/// their return reported later, on another channel) waits for them instead of failing.
#[test]
fn a_load_waits_for_links_still_handed_to_an_ended_load() {
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    assert_eq!(k.st(at(7), Input::Acquire), [], "waits");
    assert_eq!(
        k.st(at(8), Input::Released),
        [Effect::Attempt(Trainer, 2), Effect::Attempt(Strap, 2)]
    );
    k.st(at(9), Input::Attempted(Trainer, 2, true));
    assert_eq!(
        k.st(at(9), Input::Attempted(Strap, 2, true)),
        [Effect::HandOver]
    );

    let mut k = warm();
    k.st(at(6), Input::Acquire);
    assert_eq!(k.st(at(7), Input::Acquire), []);
    assert_eq!(k.st(at(8), Input::Refused), [Effect::HandOver]);
    // a loaded ride refuses a second load
    k.st(at(9), Input::Loaded("ride".into()));
    assert_eq!(k.st(at(10), Input::Acquire), [Effect::FailLoad]);
}

/// Every state the keeper can be in, by a path of inputs from a paired keeper, with its name.
fn states() -> Vec<(&'static str, Keeper)> {
    let mut all = Vec::new();
    all.push(("free, nothing held", paired()));
    all.push(("free, both warm", warm()));
    let mut k = paired();
    k.st(at(0), likely());
    k.st(at(4), Input::Attempted(Trainer, 1, false));
    all.push(("free, scanning", k));
    let mut k = paired();
    k.st(at(0), Input::Acquire);
    all.push(("free, a load waits", k));
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    all.push(("handed", k.clone()));
    k.st(
        at(6),
        Input::Pairings([Some("neo2".into()), Some("hrm".into())]),
    );
    all.push(("handed, trainer re-paired", k));
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    k.st(at(6), Input::Shutdown);
    all.push(("handed, quitting", k));
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    k.st(at(6), release(ReleaseReason::Hidden));
    all.push(("handed, Hidden recorded", k));
    all.push(("loaded", loaded(6)));
    let mut k = loaded(6);
    k.st(at(616), Input::Tick);
    all.push(("loaded, discarding", k));
    let mut k = loaded(6);
    k.st(at(7), Input::Started("ride".into()));
    all.push(("loaded, started", k));
    let mut k = warm();
    k.st(at(6), Input::Shutdown);
    all.push(("free, quitting", k));
    all
}
fn intents() -> Vec<LinkIntent> {
    use ReleaseReason::{Devices, Hidden, RiderRelease, Sleep};
    let mut all = vec![
        LinkIntent::Likely { ready: false },
        LinkIntent::Likely { ready: true },
        LinkIntent::Retry,
        LinkIntent::Left,
        LinkIntent::Background,
    ];
    all.extend([Hidden, Sleep, Devices, RiderRelease].map(|reason| LinkIntent::Release { reason }));
    all
}
fn acks(effects: &[Effect]) -> Vec<Ack> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Ack(a) => Some(*a),
            _ => None,
        })
        .collect()
}
/// The answer the keeper owes `intent` in the state named `state` (module docs, **Totality**).
fn expected(state: &str, intent: LinkIntent) -> Ack {
    use ReleaseReason::{RiderRelease, Sleep};
    let likely = matches!(intent, LinkIntent::Likely { .. } | LinkIntent::Retry);
    let discarding = matches!(
        intent,
        LinkIntent::Release {
            reason: Sleep | RiderRelease
        }
    );
    match state {
        s if s.starts_with("handed") => Ack::Recorded,
        "loaded" | "loaded, discarding" if discarding => Ack::AwaitDiscard,
        "loaded, started" if intent == release_intent(RiderRelease) => {
            Ack::Refused(RIDE_HOLDS_TRAINER)
        }
        "free, quitting" if likely => Ack::Refused(QUITTING),
        _ => Ack::Applied,
    }
}
fn release_intent(reason: ReleaseReason) -> LinkIntent {
    LinkIntent::Release { reason }
}
/// Every input that is not an intent, as a ride and its devices may report it.
fn others() -> Vec<Input<'static>> {
    vec![
        Input::Tick,
        Input::Phase(Trainer, 1, Phase::Connecting),
        Input::Attempted(Trainer, 1, true),
        Input::Attempted(Strap, 1, false),
        Input::Lost(Trainer, 1),
        Input::Advertised("neo"),
        Input::Pairings([Some("neo".into()), None]),
        Input::Rider(RiderConfig {
            rider_kg: 90.0,
            ..RIDER
        }),
        Input::Acquire,
        Input::Refused,
        Input::Loaded("ride".into()),
        Input::Started("ride".into()),
        Input::Discarded("ride".into(), false),
        Input::Discarded("ride".into(), true),
        Input::Released,
        Input::Shutdown,
    ]
}
/// The load resolves in each of its three ways.
fn resolutions() -> [Input<'static>; 3] {
    [
        Input::Refused,
        Input::Loaded("ride".into()),
        Input::Released,
    ]
}

/// Totality (B1): in every state, every intent gets exactly the answer the table owes it, and
/// none is dropped: an intent recorded in `Handed` is answered when the load resolves, in each
/// of the ways it can. Every other input, in every state, keeps the invariants, also after the
/// load resolves.
#[test]
fn the_transition_table_is_total() {
    for (name, state) in states() {
        state.invariant().unwrap_or_else(|e| panic!("{name}: {e}"));
        for intent in intents() {
            let mut k = state.clone();
            let effects = k.step(at(700), Input::Intent(intent));
            let mut owed = vec![expected(name, intent)];
            // a Retry is a rider action: it supersedes a recorded release first
            if name == "handed, Hidden recorded" && intent == LinkIntent::Retry {
                owed.insert(0, Ack::Superseded);
            }
            assert_eq!(acks(&effects), owed, "{name} × {intent:?}");
            k.invariant()
                .unwrap_or_else(|e| panic!("{name} × {intent:?}: {e}"));
            if name.starts_with("handed") {
                for resolution in resolutions() {
                    let mut resolved = k.clone();
                    let effects = resolved.step(at(701), resolution.clone());
                    assert_eq!(
                        acks(&effects).len(),
                        1,
                        "{name} × {intent:?} × {resolution:?}: the recorded intent is answered"
                    );
                    assert_ne!(acks(&effects)[0], Ack::Recorded);
                    resolved
                        .invariant()
                        .unwrap_or_else(|e| panic!("{name} × {intent:?} × {resolution:?}: {e}"));
                }
            }
        }
        for input in others() {
            let mut k = state.clone();
            let effects = k.step(at(700), input.clone());
            let owed: &[Ack] = match (name, &input) {
                // a load supersedes the recorded release
                ("handed, Hidden recorded", Input::Acquire) => &[Ack::Superseded],
                // a resolution answers the recorded intent
                (
                    "handed, Hidden recorded",
                    Input::Refused | Input::Loaded(_) | Input::Released,
                ) => &[Ack::Applied],
                _ => &[],
            };
            assert_eq!(acks(&effects), owed, "{name} × {input:?}");
            k.invariant()
                .unwrap_or_else(|e| panic!("{name} × {input:?}: {e}"));
            for resolution in resolutions() {
                let mut resolved = k.clone();
                resolved.step(at(701), resolution.clone());
                resolved
                    .invariant()
                    .unwrap_or_else(|e| panic!("{name} × {input:?} × {resolution:?}: {e}"));
            }
        }
    }
}

/// B1: `Sleep` and "Release trainer" while the links are handed out are recorded and applied
/// when the ride is loaded: the unstarted ride is discarded, and "Release trainer" latches the
/// trainer once that went through.
#[test]
fn a_release_while_handed_is_applied_when_the_ride_loads() {
    for reason in [ReleaseReason::Sleep, ReleaseReason::RiderRelease] {
        let mut k = warm();
        k.st(at(6), Input::Acquire);
        assert_eq!(k.st(at(7), release(reason)), [Effect::Ack(Ack::Recorded)]);
        // a later intent that is no release does not override it
        assert_eq!(k.st(at(7), likely()), [Effect::Ack(Ack::Recorded)]);
        assert_eq!(
            k.st(at(8), Input::Loaded("ride".into())),
            [
                Effect::Discard("ride".into()),
                Effect::Ack(Ack::AwaitDiscard)
            ],
            "{reason:?}"
        );
        k.st(at(9), Input::Discarded("ride".into(), true));
        k.st(at(9), Input::Released);
        let expect: &[Effect] = if reason == ReleaseReason::RiderRelease {
            &[Effect::Attempt(Strap, 2)]
        } else {
            &[Effect::Attempt(Trainer, 2), Effect::Attempt(Strap, 2)]
        };
        assert_eq!(k.st(at(10), likely()), expect, "{reason:?}: latched or not");
    }
}

/// B1: `Hidden` while handed, then the load gone (`Refused`): the links come back and are
/// released, never left warm with nothing due.
#[test]
fn hidden_while_handed_then_refused_releases_the_links() {
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    k.st(at(7), release(ReleaseReason::Hidden));
    let effects = k.st(at(8), Input::Refused);
    assert_eq!(
        cools(&effects),
        [
            (Trainer, Phase::Warm, Cool::Release(ReleaseReason::Hidden)),
            (Strap, Phase::Warm, Cool::Release(ReleaseReason::Hidden)),
        ]
    );
    assert_eq!(phases(&k), [Some(Phase::Cold); 2]);
    k.invariant().unwrap();
}

/// B1: a pairing change while handed, then `Refused`: the link that comes back is the old
/// trainer's, and is released; the new pairing starts `cold`.
#[test]
fn a_pairing_change_while_handed_releases_the_returned_old_link() {
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    k.st(
        at(7),
        Input::Pairings([Some("neo2".into()), Some("hrm".into())]),
    );
    let effects = k.st(at(8), Input::Refused);
    assert_eq!(cools(&effects), [(Trainer, Phase::Warm, Cool::Unpaired)]);
    assert_eq!(k.id(Trainer), Some("neo2"));
    assert_eq!(phases(&k), [Some(Phase::Cold), Some(Phase::Warm)]);
}

/// N3: a device that advertises but never connects gets at most one attempt per scan window
/// (the continuous scan counts as one), and its failures do not restart the cap.
#[test]
fn a_device_that_advertises_but_never_connects_is_bounded_to_one_attempt_per_window() {
    let mut k = paired();
    k.st(at(0), likely());
    k.st(at(0), Input::Attempted(Strap, 1, true));
    k.st(at(4), Input::Attempted(Trainer, 1, false));
    let mut attempts = 0;
    for s in 4..604 {
        if s % 30 == 0 {
            k.st(at(s), likely());
        }
        for effect in k.st(at(s), Input::Advertised("neo")) {
            if let Effect::Attempt(Trainer, era) = effect {
                attempts += 1;
                // it fails a second later
                k.st(at(s), Input::Phase(Trainer, era, Phase::Connecting));
                k.st(at(s + 1), Input::Attempted(Trainer, era, false));
            }
        }
    }
    // the continuous scan (4..34 s), then windows at 64, 94, … 574 s
    let windows = (64..604).step_by(30).count();
    assert_eq!(attempts, 1 + windows);
    assert_ne!(
        k.scanning(at(605)),
        Some(true),
        "still capped, not continuous again"
    );
}

/// N4: a discard that did not go through (a Start was sent first, then failed) re-arms
/// `T_abandon` from then, and a later `Sleep` discards again.
#[test]
fn a_failed_discard_rearms_t_abandon_and_sleep() {
    let mut k = loaded(10);
    assert_eq!(k.st(at(610), Input::Tick), [Effect::Discard("ride".into())]);
    k.st(at(611), Input::Discarded("ride".into(), false));
    assert_eq!(k.next_wake(at(611)), Some(at(1211)));
    assert_eq!(
        k.st(at(1211), Input::Tick),
        [Effect::Discard("ride".into())]
    );
    k.st(at(1212), Input::Discarded("ride".into(), false));
    assert_eq!(
        k.st(at(1213), release(ReleaseReason::Sleep)),
        [
            Effect::Discard("ride".into()),
            Effect::Ack(Ack::AwaitDiscard)
        ]
    );
}

/// N2: "Release trainer" whose discard finds the ride started (a Start raced it) does not
/// latch the trainer.
#[test]
fn a_rider_release_whose_discard_failed_does_not_latch() {
    let mut k = loaded(10);
    k.st(at(20), release(ReleaseReason::RiderRelease));
    k.st(at(21), Input::Discarded("ride".into(), false));
    k.st(at(22), Input::Started("ride".into()));
    k.st(at(900), Input::Released);
    assert_eq!(
        k.st(at(901), likely()),
        [Effect::Attempt(Trainer, 2), Effect::Attempt(Strap, 2)]
    );
}

/// X1 (decided rule, ADR-0002 §5.1.1): a load ("Ride this") after a recorded release
/// supersedes it, in either order of the load and the return of the links: the new load gets
/// the links, never a `FailLoad`.
#[test]
fn a_load_after_a_recorded_release_supersedes_it_in_either_order() {
    // the load reaches the keeper before the links come back
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    k.st(at(7), release(ReleaseReason::Hidden));
    k.st(at(7), Input::Intent(LinkIntent::Likely { ready: true }));
    assert_eq!(k.st(at(8), Input::Acquire), [Effect::Ack(Ack::Superseded)]);
    let effects = k.st(at(9), Input::Refused);
    assert!(cools(&effects).is_empty(), "{effects:?}");
    assert!(effects.contains(&Effect::HandOver), "{effects:?}");
    assert!(!effects.contains(&Effect::FailLoad));

    // the links come back first: Hidden releases them, and the load connects them again
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    k.st(at(7), release(ReleaseReason::Hidden));
    k.st(at(7), Input::Intent(LinkIntent::Likely { ready: true }));
    assert_eq!(cools(&k.st(at(8), Input::Refused)).len(), 2);
    let effects = k.st(at(9), Input::Acquire);
    assert_eq!(
        effects,
        [Effect::Attempt(Trainer, 3), Effect::Attempt(Strap, 3)]
    );
    k.st(at(10), Input::Attempted(Trainer, 3, true));
    assert_eq!(
        k.st(at(10), Input::Attempted(Strap, 3, true)),
        [Effect::HandOver]
    );
}

/// X3: Retry after a recorded `Hidden` supersedes it too: the links come back warm and stay.
#[test]
fn retry_after_a_recorded_release_supersedes_it() {
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    k.st(at(7), release(ReleaseReason::Hidden));
    assert_eq!(
        k.st(at(8), Input::Intent(LinkIntent::Retry)),
        [Effect::Ack(Ack::Superseded), Effect::Ack(Ack::Recorded)]
    );
    let effects = k.st(at(9), Input::Refused);
    assert!(cools(&effects).is_empty(), "{effects:?}");
    assert_eq!(phases(&k), [Some(Phase::Warm); 2]);
    k.invariant().unwrap();
}

/// X2: a load after a recorded "Release trainer" supersedes it, latch included: with the
/// links released, the new load connects both, whichever comes first.
#[test]
fn a_load_after_a_recorded_rider_release_supersedes_it_in_either_order() {
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    k.st(at(7), release(ReleaseReason::RiderRelease));
    assert_eq!(k.st(at(8), Input::Acquire), [Effect::Ack(Ack::Superseded)]);
    assert_eq!(
        k.st(at(9), Input::Released),
        [Effect::Attempt(Trainer, 2), Effect::Attempt(Strap, 2)]
    );

    let mut k = warm();
    k.st(at(6), Input::Acquire);
    k.st(at(7), release(ReleaseReason::RiderRelease));
    k.st(at(8), Input::Released);
    assert_eq!(
        k.st(at(9), Input::Acquire),
        [Effect::Attempt(Trainer, 2), Effect::Attempt(Strap, 2)],
        "the load clears the latch"
    );
}

/// M3, M23: of two releases recorded in `Handed`, in either order, the stronger is applied
/// (`RiderRelease` > `Sleep` > `Devices` > `Hidden`), never the weaker or merely the later.
#[test]
fn the_stronger_of_two_recorded_releases_wins_in_every_order() {
    use ReleaseReason::{Devices, Hidden, RiderRelease, Sleep};
    let all = [Hidden, Devices, Sleep, RiderRelease];
    for a in all {
        for b in all {
            let stronger = if strength(a) >= strength(b) { a } else { b };
            let mut k = warm();
            k.st(at(6), Input::Acquire);
            k.st(at(7), release(a));
            k.st(at(7), release(b));
            let reasons: Vec<Cool> = cools(&k.st(at(8), Input::Refused))
                .into_iter()
                .map(|c| c.2)
                .collect();
            assert_eq!(reasons, [Cool::Release(stronger); 2], "{a:?} then {b:?}");
        }
    }
    // on the ride: Sleep and Hidden, in either order, discard (Hidden would keep the ride)
    for (first, second) in [(Sleep, Hidden), (Hidden, Sleep)] {
        let mut k = warm();
        k.st(at(6), Input::Acquire);
        k.st(at(7), release(first));
        k.st(at(7), release(second));
        assert!(
            k.st(at(8), Input::Loaded("ride".into()))
                .contains(&Effect::Discard("ride".into())),
            "{first:?} then {second:?}"
        );
    }
}

/// M21: "Release trainer" that joins a discard already under way (`T_abandon`, `Sleep`)
/// latches the trainer once that discard went through.
#[test]
fn a_rider_release_joining_a_discard_under_way_latches() {
    for cause in ["abandon", "sleep"] {
        let mut k = loaded(10);
        if cause == "abandon" {
            assert_eq!(k.st(at(610), Input::Tick), [Effect::Discard("ride".into())]);
        } else {
            k.st(at(20), release(ReleaseReason::Sleep));
        }
        assert_eq!(
            k.st(at(611), release(ReleaseReason::RiderRelease)),
            [Effect::Ack(Ack::AwaitDiscard)],
            "{cause}: joins, no second discard"
        );
        k.st(at(612), Input::Discarded("ride".into(), true));
        k.st(at(612), Input::Released);
        assert_eq!(
            k.st(at(613), likely()),
            [Effect::Attempt(Strap, 2)],
            "{cause}: latched"
        );
    }
}

/// §5.1.2 and the brief's point 3: a rider configuration that changes while the links are
/// handed out reconnects the trainer that comes back (`Refused`); the next attempt after a
/// ride uses it.
#[test]
fn a_rider_change_while_handed_reconnects_the_returned_trainer() {
    let heavier = RiderConfig {
        rider_kg: 80.0,
        ..RIDER
    };
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    assert_eq!(k.st(at(7), Input::Rider(heavier)), []);
    let effects = k.st(at(8), Input::Refused);
    assert_eq!(cools(&effects), [(Trainer, Phase::Warm, Cool::Rider)]);
    assert!(
        effects.contains(&Effect::Attempt(Trainer, 3)),
        "{effects:?}"
    );
    assert_eq!(k.rider(), heavier);

    // loaded instead: the ride keeps its link; the keeper's next trainer link uses the new one
    let mut k = warm();
    k.st(at(6), Input::Acquire);
    k.st(at(7), Input::Rider(heavier));
    assert_eq!(k.st(at(8), Input::Loaded("ride".into())), []);
    k.st(at(9), Input::Released);
    k.st(at(10), likely());
    assert_eq!(k.trainer_rider, heavier);
}
