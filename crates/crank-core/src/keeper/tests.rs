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
    k.step(
        at(0),
        Input::Pairings([Some("neo".into()), Some("hrm".into())]),
    );
    k
}
/// Both devices warm at 5 s after a `Likely` at 0 s.
fn warm() -> Keeper {
    let mut k = paired();
    k.step(at(0), likely());
    k.step(at(5), Input::Attempted(Trainer, 1, true));
    k.step(at(5), Input::Attempted(Strap, 1, true));
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
        .map(|s| (s, k.step(at(s), Input::Tick)))
        .filter(|(_, e)| !e.is_empty())
        .collect()
}
/// A ride loaded at `s` over both warm devices.
fn loaded(s: u64) -> Keeper {
    let mut k = warm();
    assert_eq!(k.step(at(s), Input::Acquire), [Effect::HandOver]);
    k.step(at(s), Input::Loaded("ride".into()));
    k
}

/// §5.1.2: `Likely` attempts both paired devices at once; an unpaired one has no phase.
#[test]
fn likely_starts_both_attempts_in_parallel() {
    let mut k = paired();
    assert_eq!(
        k.step(at(0), likely()),
        [Effect::Attempt(Trainer, 1), Effect::Attempt(Strap, 1)]
    );
    let mut alone = Keeper::new(RIDER);
    alone.step(at(0), Input::Pairings([Some("neo".into()), None]));
    assert_eq!(alone.step(at(0), likely()), [Effect::Attempt(Trainer, 1)]);
    assert_eq!(alone.phase(Strap), None);
}

/// §5.1.2: a refresh of a standing `Likely` retries no `not_found` device (no connect loop);
/// a fresh one does, and so does Retry.
#[test]
fn only_a_fresh_likely_retries_a_device_not_found() {
    let mut k = paired();
    k.step(at(0), likely());
    k.step(at(4), Input::Attempted(Trainer, 1, false));
    k.step(at(4), Input::Attempted(Strap, 1, true));
    assert_eq!(k.step(at(30), likely()), []);
    k.step(at(31), Input::Intent(LinkIntent::Left));
    assert_eq!(k.step(at(32), likely()), [Effect::Attempt(Trainer, 2)]);
    k.step(at(36), Input::Attempted(Trainer, 2, false));
    assert_eq!(
        k.step(at(40), Input::Intent(LinkIntent::Retry)),
        [Effect::Attempt(Trainer, 3)]
    );
}

/// §5.1.8 "Idle cool-down disconnects": after `Left` the devices are still warm at
/// `T_cool` − 1 s and released at `T_cool`; a `Likely` in between restarts the timer.
#[test]
fn left_cools_down_at_t_cool_and_a_likely_restarts_it() {
    let mut k = warm();
    k.step(at(10), Input::Intent(LinkIntent::Left));
    assert_eq!(run(&mut k, 11, 309), []);
    assert_eq!(phases(&k), [Some(Phase::Warm); 2]);
    assert_eq!(k.next_wake(at(309)), Some(at(310)));
    let cooled = k.step(at(310), Input::Tick);
    assert_eq!(
        cools(&cooled),
        [
            (Trainer, Phase::Warm, Cool::Idle),
            (Strap, Phase::Warm, Cool::Idle)
        ]
    );
    assert_eq!(phases(&k), [Some(Phase::Cold); 2]);

    let mut k = warm();
    k.step(at(10), Input::Intent(LinkIntent::Left));
    k.step(at(200), likely());
    k.step(at(201), Input::Intent(LinkIntent::Left));
    assert_eq!(run(&mut k, 202, 500), []);
    assert_eq!(cools(&k.step(at(501), Input::Tick)).len(), 2);
}

/// §5.1.3: on a warm view, `T_cool` without rider input (the throttled refresh) cools down
/// too; each refresh restarts it.
#[test]
fn a_warm_view_without_input_cools_down_at_t_cool() {
    let mut k = warm();
    k.step(at(100), likely());
    assert_eq!(run(&mut k, 101, 399), []);
    assert_eq!(cools(&k.step(at(400), Input::Tick)).len(), 2);
}

/// §5.1.8 "Background grace": still warm at 59 s, released at 60 s; a `Likely` at 30 s
/// cancels the release.
#[test]
fn the_background_grace_is_sixty_seconds_and_a_likely_cancels_it() {
    let mut k = warm();
    k.step(at(10), Input::Intent(LinkIntent::Background));
    assert_eq!(run(&mut k, 11, 69), []);
    assert_eq!(
        cools(&k.step(at(70), Input::Tick))
            .iter()
            .map(|c| c.2)
            .collect::<Vec<_>>(),
        [Cool::Background; 2]
    );

    let mut k = warm();
    k.step(at(10), Input::Intent(LinkIntent::Background));
    k.step(at(40), likely());
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
        k.step(at(0), likely());
        k.step(at(1), Input::Attempted(Trainer, 1, true));
        k.step(at(1), Input::Phase(Strap, 1, Phase::Connecting));
        assert_eq!(
            cools(&k.step(at(2), release(reason))),
            [
                (Trainer, Phase::Warm, Cool::Release(reason)),
                (Strap, Phase::Connecting, Cool::Release(reason)),
            ]
        );
        assert_eq!(phases(&k), [Some(Phase::Cold); 2]);
        // the cancelled attempt's late result is stale
        assert_eq!(k.step(at(3), Input::Attempted(Strap, 1, true)), []);
        assert_eq!(k.phase(Strap), Some(Phase::Cold));
    }
}

/// §5.1.8 "Idle cool-down disconnects": the latch blocks `Likely` for the trainer until Retry
/// or "Ride this".
#[test]
fn the_rider_release_latch_holds_until_retry_or_a_load() {
    let mut k = warm();
    k.step(at(10), release(ReleaseReason::RiderRelease));
    // (a cool-down ends an attempt's era too: these are the third)
    assert_eq!(k.step(at(20), likely()), [Effect::Attempt(Strap, 3)]);
    assert_eq!(k.phase(Trainer), Some(Phase::Cold));
    assert_eq!(
        k.step(at(30), Input::Intent(LinkIntent::Retry)),
        [Effect::Attempt(Trainer, 3)]
    );

    let mut k = warm();
    k.step(at(10), release(ReleaseReason::RiderRelease));
    k.step(at(20), Input::Intent(LinkIntent::Left));
    assert_eq!(
        k.step(at(30), Input::Acquire),
        [Effect::Attempt(Trainer, 3), Effect::Attempt(Strap, 3)]
    );
}

/// §5.1.8 "The Ready sheet holds": with a failed load and `Likely { ready: true }`, no input
/// for 30 min leaves the keeper's links warm; hiding still releases them.
#[test]
fn the_ready_sheet_holds_without_input_until_hidden() {
    let mut k = paired();
    k.step(at(0), likely());
    k.step(at(0), Input::Acquire);
    k.step(at(1), Input::Attempted(Strap, 1, true));
    assert_eq!(
        k.step(at(15), Input::Attempted(Trainer, 1, false)),
        [Effect::FailLoad]
    );
    k.step(at(15), Input::Intent(LinkIntent::Likely { ready: true }));
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
        cools(&k.step(at(1900), release(ReleaseReason::Hidden))).len(),
        1
    );
}

/// §5.1.4: a load takes warm links at once; a running attempt is awaited, never doubled; a
/// cold device is connected now.
#[test]
fn a_load_takes_warm_links_awaits_running_attempts_and_connects_cold_ones() {
    let mut k = warm();
    assert_eq!(k.step(at(6), Input::Acquire), [Effect::HandOver]);
    assert_eq!(phases(&k), [Some(Phase::Riding); 2]);

    let mut k = paired();
    k.step(at(0), likely());
    k.step(at(1), Input::Phase(Trainer, 1, Phase::Handshake));
    assert_eq!(
        k.step(at(2), Input::Acquire),
        [],
        "both attempts are awaited"
    );
    k.step(at(3), Input::Attempted(Trainer, 1, true));
    assert_eq!(
        k.step(at(4), Input::Attempted(Strap, 1, false)),
        [Effect::HandOver]
    );
    assert_eq!(phases(&k), [Some(Phase::Riding), Some(Phase::NotFound)]);

    let mut k = paired();
    assert_eq!(
        k.step(at(0), Input::Acquire),
        [Effect::Attempt(Trainer, 1), Effect::Attempt(Strap, 1)]
    );
}

/// §5.1.4: a trainer that fails fails the load at once, beside a strap still connecting,
/// and the strap stays the keeper's.
#[test]
fn a_failed_trainer_fails_the_load_at_once_and_the_strap_stays_with_the_keeper() {
    let mut k = paired();
    k.step(at(0), Input::Acquire);
    assert_eq!(
        k.step(at(3), Input::Attempted(Trainer, 1, false)),
        [Effect::FailLoad]
    );
    k.step(at(5), Input::Attempted(Strap, 1, true));
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
        assert_eq!(k.step(at(7), input), []);
    }
    // input keeps the ride from being abandoned; nothing else is due
    for s in (60..=1800).step_by(60) {
        k.step(at(s), likely());
        assert_eq!(k.step(at(s), Input::Tick), []);
    }
    assert_eq!(phases(&k), [Some(Phase::Riding); 2]);
}

/// §5.1.8 "An abandoned load releases": discarded at `T_abandon` without input; input at
/// 9 min restarts the timer.
#[test]
fn an_abandoned_load_is_discarded_at_t_abandon_and_input_restarts_it() {
    let mut k = loaded(10);
    assert_eq!(run(&mut k, 11, 609), []);
    assert_eq!(
        k.step(at(610), Input::Tick),
        [Effect::Discard("ride".into())]
    );
    assert_eq!(run(&mut k, 611, 2000), [], "asked for once");
    k.step(at(611), Input::Released);
    assert_eq!(phases(&k), [Some(Phase::Cold); 2]);

    let mut k = loaded(10);
    k.step(at(550), likely());
    assert_eq!(run(&mut k, 11, 1149), []);
    assert_eq!(
        k.step(at(1150), Input::Tick),
        [Effect::Discard("ride".into())]
    );
    // a started ride is never abandoned
    let mut k = loaded(10);
    k.step(at(20), Input::Started("ride".into()));
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
            k.step(at(20), release(reason)),
            [Effect::Discard("ride".into())]
        );
        k.step(at(21), Input::Released);
        let latched = reason == ReleaseReason::RiderRelease;
        let expect: &[Effect] = if latched {
            &[Effect::Attempt(Strap, 2)]
        } else {
            &[Effect::Attempt(Trainer, 2), Effect::Attempt(Strap, 2)]
        };
        assert_eq!(k.step(at(30), likely()), expect, "{reason:?}");

        let mut k = loaded(10);
        k.step(at(15), Input::Started("ride".into()));
        assert_eq!(k.step(at(20), release(reason)), []);
        assert_eq!(phases(&k), [Some(Phase::Riding); 2]);
    }
}

/// §5.1.8 "`Likely` during a ride warms nothing": with a ride loaded or started and the strap
/// not linked, `Likely` and Retry start no attempt.
#[test]
fn likely_during_a_ride_warms_nothing() {
    let mut k = paired();
    k.step(at(0), Input::Acquire);
    k.step(at(1), Input::Attempted(Strap, 1, false));
    k.step(at(2), Input::Attempted(Trainer, 1, true));
    k.step(at(2), Input::Loaded("ride".into()));
    for started in [false, true] {
        if started {
            k.step(at(3), Input::Started("ride".into()));
        }
        assert_eq!(k.step(at(4), likely()), []);
        assert_eq!(k.step(at(4), Input::Intent(LinkIntent::Retry)), []);
        assert_eq!(k.step(at(5), Input::Advertised("hrm")), []);
        assert_eq!(k.scanning(at(5)), None);
    }
}

/// §5.1.8 "Scan cap": with the trainer never advertising and intent `Likely` for 10 min,
/// the scan is continuous for `SCAN_CONTINUOUS`, then on for `RESOLVE_SCAN` in every
/// `SCAN_PERIOD`; it stops on `Left`; an advertisement in a window starts the attempt.
#[test]
fn the_scan_is_capped_and_stops_on_left() {
    let mut k = paired();
    k.step(at(0), likely());
    k.step(at(0), Input::Attempted(Strap, 1, true));
    k.step(at(4), Input::Attempted(Trainer, 1, false));
    let mut on = Vec::new();
    for s in 4..604 {
        if s % 30 == 0 {
            k.step(at(s), likely());
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
        k.step(at(605), Input::Advertised("neo")),
        [Effect::Attempt(Trainer, 2)]
    );
    assert_eq!(k.scanning(at(605)), None);
    k.step(at(609), Input::Attempted(Trainer, 2, false));
    assert_eq!(k.scanning(at(610)), Some(true), "continuous again");
    k.step(at(611), Input::Intent(LinkIntent::Left));
    assert_eq!(k.scanning(at(611)), None);
    assert_eq!(k.step(at(612), Input::Advertised("neo")), []);
}

/// §5.1.2: a change of rider configuration while the trainer is warm reconnects it.
#[test]
fn a_rider_change_reconnects_a_warm_trainer() {
    let mut k = warm();
    let heavier = RiderConfig {
        rider_kg: 80.0,
        ..RIDER
    };
    let effects = k.step(at(10), Input::Rider(heavier));
    assert_eq!(cools(&effects), [(Trainer, Phase::Warm, Cool::Rider)]);
    assert_eq!(effects.last(), Some(&Effect::Attempt(Trainer, 3)));
    assert_eq!(k.rider(), heavier);
    assert_eq!(k.step(at(11), Input::Rider(heavier)), []);
}

/// §5.1.8 "Handover": a warm link that is lost makes the next load connect cold; a ride's end
/// leaves the phase `cold`, never `warm`; links a cancelled load never took are warm again.
#[test]
fn a_lost_link_connects_cold_at_the_load_and_a_ride_end_leaves_cold() {
    let mut k = warm();
    assert_eq!(
        cools(&k.step(at(10), Input::Lost(Trainer, 1))),
        [(Trainer, Phase::Warm, Cool::Lost)]
    );
    assert_eq!(k.phase(Trainer), Some(Phase::NotFound));
    assert_eq!(
        k.step(at(11), Input::Acquire),
        [Effect::Attempt(Trainer, 3)]
    );
    k.step(at(12), Input::Attempted(Trainer, 3, true));
    assert_eq!(phases(&k), [Some(Phase::Riding); 2]);
    k.step(at(13), Input::Loaded("ride".into()));
    k.step(at(14), Input::Released);
    assert_eq!(phases(&k), [Some(Phase::Cold); 2]);
    assert_eq!(k.step(at(15), Input::Tick), [], "no intent after a ride");

    let mut k = warm();
    k.step(at(6), Input::Acquire);
    k.step(at(7), Input::Refused);
    assert_eq!(phases(&k), [Some(Phase::Warm); 2]);
}

/// §5.1.3: quitting lets every keeper link go and takes no new one.
#[test]
fn shutdown_lets_go_of_everything_and_takes_nothing_new() {
    let mut k = paired();
    k.step(at(0), likely());
    k.step(at(1), Input::Attempted(Trainer, 1, true));
    assert_eq!(
        cools(&k.step(at(2), Input::Shutdown)),
        [
            (Trainer, Phase::Warm, Cool::Shutdown),
            (Strap, Phase::Searching, Cool::Shutdown)
        ]
    );
    assert_eq!(k.step(at(3), likely()), []);
    assert_eq!(k.step(at(3), Input::Acquire), [Effect::FailLoad]);
}

/// §5.1.2: a pairing that changes lets the device go; one that is forgotten has no phase.
#[test]
fn a_changed_pairing_lets_the_device_go() {
    let mut k = warm();
    let effects = k.step(at(10), Input::Pairings([Some("neo".into()), None]));
    assert_eq!(cools(&effects), [(Strap, Phase::Warm, Cool::Unpaired)]);
    assert_eq!(phases(&k), [Some(Phase::Warm), None]);
}

/// §5.1.4: a load that comes while the links are still handed to an ended load (its release or
/// their return reported later, on another channel) waits for them instead of failing.
#[test]
fn a_load_waits_for_links_still_handed_to_an_ended_load() {
    let mut k = warm();
    k.step(at(6), Input::Acquire);
    assert_eq!(k.step(at(7), Input::Acquire), [], "waits");
    assert_eq!(
        k.step(at(8), Input::Released),
        [Effect::Attempt(Trainer, 2), Effect::Attempt(Strap, 2)]
    );
    k.step(at(9), Input::Attempted(Trainer, 2, true));
    assert_eq!(
        k.step(at(9), Input::Attempted(Strap, 2, true)),
        [Effect::HandOver]
    );

    let mut k = warm();
    k.step(at(6), Input::Acquire);
    assert_eq!(k.step(at(7), Input::Acquire), []);
    assert_eq!(k.step(at(8), Input::Refused), [Effect::HandOver]);
    // a loaded ride refuses a second load
    k.step(at(9), Input::Loaded("ride".into()));
    assert_eq!(k.step(at(10), Input::Acquire), [Effect::FailLoad]);
}
