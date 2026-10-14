//! Behaviour of `SimTrainer`/`SimHeart` per mode and per injected fault (#67).
//! Time is tokio's paused clock behind the injected [`Clock`]; nothing sleeps for real.
use crank_devices::sim::{
    Drive, EchoFault, SPIRAL_ONSET_RPM, SimComponent, SimConfig, SimCore, SimFault, SimRig,
};
use crank_devices::*;
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};
use tokio::time::{Instant, sleep};

struct TestClock(Instant);
impl Clock for TestClock {
    fn now(&self) -> MonoTime {
        MonoTime(u64::try_from(self.0.elapsed().as_micros()).expect("sim"))
    }
    fn sleep_until(&self, at: MonoTime) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep_until(
            self.0 + Duration::from_micros(at.0),
        ))
    }
}
const RIDER: RiderConfig = RiderConfig {
    rider_kg: 72.0,
    bike_kg: 9.0,
    wheel_m: 2.096,
};
fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}
fn rig(faults: Vec<SimFault>) -> SimRig {
    let cfg = SimConfig {
        faults,
        ..SimConfig::default()
    };
    SimRig::new(cfg, Arc::new(TestClock(Instant::now())))
}
async fn connect(rig: &SimRig) -> TrainerLink {
    rig.trainer()
        .connect(&DeviceRef("sim".into()), RIDER)
        .await
        .expect("sim")
}
/// Let `secs` of virtual time pass, then take everything delivered so far.
async fn run(link: &mut TrainerLink, secs: u64) -> Vec<TrainerEvent> {
    // Drain every second like the engine would: the event channel is bounded and lossless.
    let mut out = Vec::new();
    for _ in 0..secs {
        sleep(s(1)).await;
        while let Ok(e) = link.events.try_recv() {
            out.push(e);
        }
    }
    sleep(Duration::from_millis(1)).await;
    while let Ok(e) = link.events.try_recv() {
        out.push(e);
    }
    out
}
async fn set(link: &TrainerLink, mode: ControlMode) {
    link.control
        .send(TrainerCommand::SetMode(mode))
        .await
        .expect("sim");
}
fn samples(ev: &[TrainerEvent]) -> Vec<&TrainerSample> {
    ev.iter()
        .filter_map(|e| {
            if let TrainerEvent::Sample(x) = e {
                Some(x)
            } else {
                None
            }
        })
        .collect()
}
fn echoes(ev: &[TrainerEvent]) -> Vec<(MonoTime, ObservedControl)> {
    ev.iter()
        .filter_map(|e| match e {
            TrainerEvent::Observed(o) if o.source == ObservationSource::Page71Echo => {
                Some((o.at, o.control.expect("sim")))
            }
            _ => None,
        })
        .collect()
}
fn mean_power(ev: &[TrainerEvent], from_s: u64, to_s: u64) -> f64 {
    let v: Vec<f64> = samples(ev)
        .iter()
        .filter(|x| (from_s * 1_000_000..to_s * 1_000_000).contains(&x.at.0))
        .map(|x| f64::from(x.power_w.expect("sim")))
        .collect();
    assert!(!v.is_empty());
    v.iter().sum::<f64>() / f64::from(u32::try_from(v.len()).expect("sim"))
}
fn last_sample(ev: &[TrainerEvent]) -> &TrainerSample {
    samples(ev).pop().expect("sim")
}
fn sim_mode(grade_pct: f32) -> ControlMode {
    ControlMode::Sim {
        grade_pct,
        crr: 0.004,
        wind: Wind::default(),
    }
}

#[tokio::test(start_paused = true)]
async fn connect_reports_identity_caps_and_ready_before_returning() {
    let r = rig(vec![]);
    let mut link = connect(&r).await;
    assert_eq!(r.trainer().kind(), DriverKind::Sim);
    assert!(link.caps.erg && link.caps.sim && link.caps.resistance);
    assert_eq!(
        link.events.try_recv().expect("sim"),
        TrainerEvent::Link(LinkState::Connecting)
    );
    assert_eq!(
        link.events.try_recv().expect("sim"),
        TrainerEvent::Link(LinkState::Ready)
    );
}
#[tokio::test(start_paused = true)]
async fn second_connection_is_refused_while_the_first_lives() {
    let r = rig(vec![]);
    let _link = connect(&r).await;
    let err = r
        .trainer()
        .connect(&DeviceRef("sim".into()), RIDER)
        .await
        .err()
        .expect("sim");
    assert!(matches!(err, DeviceError::ConnectFailed(_)));
}
#[tokio::test(start_paused = true)]
async fn erg_tracks_target_and_echoes_page_49_each_second() {
    let r = rig(vec![]);
    let mut link = connect(&r).await;
    set(&link, ControlMode::Erg { target_w: 200 }).await;
    let ev = run(&mut link, 30).await;
    let p = mean_power(&ev, 20, 30);
    assert!((p - 200.0).abs() < 12.0, "mean power {p}");
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(e, TrainerEvent::Delivered { .. }))
            .count(),
        1
    );
    let e = echoes(&ev);
    assert!((26..=30).contains(&e.len()), "{} echoes", e.len());
    assert!(
        e.iter()
            .all(|(_, c)| *c == ObservedControl::Erg { quarter_w: 800 })
    );
    // one page-25 flags observation per sample, in arrival order
    assert_eq!(last_sample(&ev).target_limit, TargetLimit::AtTarget);
}
#[tokio::test(start_paused = true)]
async fn sim_grade_slows_the_rider_and_echoes_wind_then_track() {
    let flat = rig(vec![]);
    let mut a = connect(&flat).await;
    set(&a, sim_mode(0.0)).await;
    let flat_ev = run(&mut a, 40).await;
    let hill = rig(vec![]);
    let mut b = connect(&hill).await;
    set(&b, sim_mode(8.0)).await;
    let hill_ev = run(&mut b, 40).await;
    assert!(
        last_sample(&hill_ev).speed_mps.expect("sim")
            < 0.6 * last_sample(&flat_ev).speed_mps.expect("sim")
    );
    assert_eq!(
        hill_ev
            .iter()
            .filter(|e| matches!(e, TrainerEvent::Delivered { .. }))
            .count(),
        2,
        "one Delivered per SIM component"
    );
    let e = echoes(&hill_ev);
    assert!(matches!(e[0].1, ObservedControl::Wind { .. }));
    assert!(matches!(
        e[1].1,
        ObservedControl::Track {
            grade_centi_pct: 800,
            ..
        }
    ));
    assert!(
        e.windows(2)
            .all(|w| std::mem::discriminant(&w[0].1) != std::mem::discriminant(&w[1].1))
    );
}
#[tokio::test(start_paused = true)]
async fn resistance_scales_effort_and_echoes_page_48() {
    let r = rig(vec![]);
    let mut link = connect(&r).await;
    set(&link, ControlMode::Resistance { pct: 100.0 }).await;
    let hard = run(&mut link, 30).await;
    assert!(
        echoes(&hard)
            .iter()
            .all(|(_, c)| *c == ObservedControl::Resistance { half_pct: 200 })
    );
    let r2 = rig(vec![]);
    let mut easy_link = connect(&r2).await;
    set(&easy_link, ControlMode::Resistance { pct: 0.0 }).await;
    let easy = run(&mut easy_link, 30).await;
    assert!(mean_power(&hard, 20, 30) > 1.6 * mean_power(&easy, 20, 30));
}
#[tokio::test(start_paused = true)]
async fn free_is_explicit_flat_sim() {
    let r = rig(vec![]);
    let mut link = connect(&r).await;
    set(&link, ControlMode::Erg { target_w: 150 }).await;
    run(&mut link, 5).await;
    set(&link, ControlMode::Free).await;
    let ev = run(&mut link, 10).await;
    let e = echoes(&ev);
    assert!(e.iter().any(|(_, c)| matches!(
        c,
        ObservedControl::Track {
            grade_centi_pct: 0,
            crr_units: 80
        }
    )));
    assert!(
        e.iter()
            .all(|(_, c)| !matches!(c, ObservedControl::Erg { .. }))
    );
}
#[tokio::test(start_paused = true)]
async fn invalid_mode_and_set_rider_surface_as_faults() {
    let r = rig(vec![]);
    let mut link = connect(&r).await;
    set(&link, ControlMode::Resistance { pct: 150.0 }).await;
    link.control
        .send(TrainerCommand::SetRider(RIDER))
        .await
        .expect("sim");
    let ev = run(&mut link, 1).await;
    let faults: Vec<_> = ev
        .iter()
        .filter(|e| matches!(e, TrainerEvent::Fault(_)))
        .collect();
    assert_eq!(faults.len(), 2);
    assert!(matches!(
        faults[0],
        TrainerEvent::Fault(DeviceError::Protocol {
            kind: ProtocolErrorKind::Encode,
            ..
        })
    ));
    assert!(echoes(&ev).is_empty(), "a rejected mode must not poll");
}
#[tokio::test(start_paused = true)]
async fn disconnect_ends_the_link() {
    let r = rig(vec![]);
    let mut link = connect(&r).await;
    link.control
        .send(TrainerCommand::Disconnect)
        .await
        .expect("sim");
    let ev = run(&mut link, 2).await;
    assert!(ev.contains(&TrainerEvent::Link(LinkState::Lost)));
    assert!(samples(&ev).len() <= 4);
}
#[tokio::test(start_paused = true)]
async fn driver_stream_equals_the_core_fed_the_applied_target() {
    let r = rig(vec![]);
    let mut link = connect(&r).await;
    set(&link, ControlMode::Erg { target_w: 200 }).await;
    let ev = run(&mut link, 20).await;
    let cfg = SimConfig::default();
    let mut core = SimCore::new(cfg.rider, 81.0, cfg.seed);
    let want: Vec<f64> = (0..80)
        .map(|_| {
            let t = core.step(Drive {
                target_w: 200.0,
                grade_pct: 0.0,
                cadence_pref: None,
                cadence_force: None,
            });
            t.power_w.expect("sim")
        })
        .collect();
    let got: Vec<f64> = samples(&ev)
        .iter()
        .take(80)
        .map(|x| f64::from(x.power_w.expect("sim")))
        .collect();
    assert_eq!(got, want);
    let t: Vec<u64> = samples(&ev).iter().take(3).map(|x| x.at.0).collect();
    assert_eq!(
        t,
        vec![250_000, 500_000, 750_000],
        "grid-stamped, not scheduler-stamped"
    );
}
#[tokio::test(start_paused = true)]
async fn same_seed_and_faults_reproduce_the_same_events() {
    let faults = vec![SimFault::Echo {
        component: SimComponent::Erg,
        fault: EchoFault::Lag(s(2)),
        from: s(3),
        until: s(30),
    }];
    let mut runs = Vec::new();
    for _ in 0..2 {
        let r = rig(faults.clone());
        let mut link = connect(&r).await;
        set(&link, ControlMode::Erg { target_w: 180 }).await;
        runs.push(run(&mut link, 30).await);
    }
    assert!(!runs[0].is_empty());
    assert_eq!(runs[0], runs[1]);
}

// ---- faults ----

#[tokio::test(start_paused = true)]
async fn dropout_goes_silent_then_reconnects_without_a_control_mode() {
    let r = rig(vec![SimFault::Dropout {
        at: s(10),
        down: s(5),
    }]);
    let mut link = connect(&r).await;
    set(&link, ControlMode::Erg { target_w: 220 }).await;
    let ev = run(&mut link, 25).await;
    let lost = ev
        .iter()
        .position(|e| *e == TrainerEvent::Link(LinkState::Lost))
        .expect("sim");
    assert!(ev[lost + 1..].contains(&TrainerEvent::Link(LinkState::Connecting)));
    assert!(
        ev.iter()
            .rposition(|e| *e == TrainerEvent::Link(LinkState::Ready))
            .expect("sim")
            > lost
    );
    let gap: Vec<_> = samples(&ev)
        .into_iter()
        .filter(|x| (10_000_000..15_000_000).contains(&x.at.0))
        .collect();
    assert!(gap.is_empty(), "no telemetry while down");
    // standalone after reconnect: no echoes and no ERG power until the engine re-issues
    assert!(echoes(&ev).iter().all(|(at, _)| at.0 <= 10_000_000));
    assert!(mean_power(&ev, 20, 25) < 0.8 * 220.0);
    set(&link, ControlMode::Erg { target_w: 220 }).await;
    let after = run(&mut link, 20).await;
    assert!(!echoes(&after).is_empty());
    assert!(mean_power(&after, 35, 45) > 200.0);
}
#[tokio::test(start_paused = true)]
async fn echo_lag_delays_the_reported_value_by_the_configured_time() {
    let r = rig(vec![SimFault::Echo {
        component: SimComponent::Erg,
        fault: EchoFault::Lag(s(3)),
        from: s(0),
        until: s(60),
    }]);
    let mut link = connect(&r).await;
    set(&link, ControlMode::Erg { target_w: 200 }).await;
    let mut ev = run(&mut link, 10).await;
    set(&link, ControlMode::Erg { target_w: 250 }).await;
    ev.extend(run(&mut link, 10).await);
    let e = echoes(&ev);
    let at = |lo: u64, hi: u64| -> Vec<ObservedControl> {
        e.iter()
            .filter(|(t, _)| (lo..hi).contains(&t.0))
            .map(|(_, c)| *c)
            .collect()
    };
    // the 250 W write lands at ~10 s and must not be reported before ~13 s
    assert!(
        at(10_250_000, 12_900_000)
            .iter()
            .all(|c| *c == ObservedControl::Erg { quarter_w: 800 })
    );
    assert!(
        at(13_250_000, 20_000_000)
            .iter()
            .all(|c| *c == ObservedControl::Erg { quarter_w: 1000 })
    );
    assert!(mean_power(&ev, 17, 20) > 235.0, "physics is not lagged");
}
#[tokio::test(start_paused = true)]
async fn stuck_echo_repeats_the_value_from_window_start_while_physics_moves() {
    let r = rig(vec![SimFault::Echo {
        component: SimComponent::Erg,
        fault: EchoFault::Stuck,
        from: s(5),
        until: s(40),
    }]);
    let mut link = connect(&r).await;
    set(&link, ControlMode::Erg { target_w: 200 }).await;
    let mut ev = run(&mut link, 10).await;
    set(&link, ControlMode::Erg { target_w: 250 }).await;
    ev.extend(run(&mut link, 20).await);
    let late: Vec<_> = echoes(&ev)
        .into_iter()
        .filter(|(t, _)| t.0 > 11_000_000)
        .collect();
    assert!(late.len() > 10);
    assert!(
        late.iter()
            .all(|(_, c)| *c == ObservedControl::Erg { quarter_w: 800 })
    );
    assert!(mean_power(&ev, 25, 30) > 235.0);
}
#[tokio::test(start_paused = true)]
async fn silent_component_is_never_echoed_the_other_still_is() {
    let r = rig(vec![SimFault::Echo {
        component: SimComponent::Wind,
        fault: EchoFault::Silent,
        from: s(0),
        until: s(60),
    }]);
    let mut link = connect(&r).await;
    set(&link, sim_mode(2.0)).await;
    let e = echoes(&run(&mut link, 20).await);
    assert!(!e.is_empty());
    assert!(
        e.iter()
            .all(|(_, c)| !matches!(c, ObservedControl::Wind { .. }))
    );
    assert!(
        e.iter()
            .any(|(_, c)| matches!(c, ObservedControl::Track { .. }))
    );
}
#[tokio::test(start_paused = true)]
async fn dropped_track_write_is_neither_applied_nor_echoed() {
    let r = rig(vec![SimFault::Echo {
        component: SimComponent::Track,
        fault: EchoFault::DropWrites,
        from: s(20),
        until: s(120),
    }]);
    let mut link = connect(&r).await;
    set(&link, sim_mode(8.0)).await;
    run(&mut link, 20).await;
    set(&link, ControlMode::Free).await; // would level the road
    let ev = run(&mut link, 30).await;
    let track: Vec<_> = echoes(&ev)
        .into_iter()
        .filter_map(|(_, c)| match c {
            ObservedControl::Track {
                grade_centi_pct, ..
            } => Some(grade_centi_pct),
            _ => None,
        })
        .collect();
    assert!(
        !track.is_empty() && track.iter().all(|g| *g == 800),
        "{track:?}"
    );
    // the wind page was applied (echoed as the new default), the track page was not
    assert!(last_sample(&ev).speed_mps.expect("sim") < 4.0);
}
#[tokio::test(start_paused = true)]
async fn cadence_stall_stops_cadence_and_power_then_recovers() {
    let r = rig(vec![SimFault::CadenceStall {
        from: s(20),
        until: s(30),
    }]);
    let mut link = connect(&r).await;
    set(&link, ControlMode::Erg { target_w: 200 }).await;
    let ev = run(&mut link, 50).await;
    let at = |sec: u64| {
        samples(&ev)
            .into_iter()
            .find(|x| x.at.0 >= sec * 1_000_000)
            .expect("sim")
            .clone()
    };
    assert!(f64::from(at(19).power_w.expect("sim")) > 170.0);
    assert!(at(29).cadence_rpm.expect("sim") < 5 && at(29).power_w.expect("sim") < 10);
    assert!(at(49).cadence_rpm.expect("sim") > 70 && at(49).power_w.expect("sim") > 170);
}
#[tokio::test(start_paused = true)]
async fn deep_cadence_dip_spirals_in_erg_and_release_recovers() {
    let spiral = SimFault::ErgSpiral {
        from: s(20),
        dip_rpm: SPIRAL_ONSET_RPM - 10.0,
    };
    let r = rig(vec![spiral]);
    let mut link = connect(&r).await;
    set(&link, ControlMode::Erg { target_w: 250 }).await;
    let ev = run(&mut link, 45).await;
    let end = last_sample(&ev);
    assert!(
        end.cadence_rpm.expect("sim") < 10 && end.power_w.expect("sim") < 40,
        "{end:?}"
    );
    assert_eq!(end.target_limit, TargetLimit::SpeedTooLow);
    // ADR-0002 §7.4: releasing ERG to zero resistance lets the rider spin up again
    set(&link, ControlMode::Resistance { pct: 0.0 }).await;
    let rec = run(&mut link, 20).await;
    assert!(last_sample(&rec).cadence_rpm.expect("sim") > 60);
}
#[tokio::test(start_paused = true)]
async fn shallow_cadence_dip_recovers_by_itself() {
    let r = rig(vec![SimFault::ErgSpiral {
        from: s(20),
        dip_rpm: SPIRAL_ONSET_RPM + 25.0,
    }]);
    let mut link = connect(&r).await;
    set(&link, ControlMode::Erg { target_w: 250 }).await;
    let ev = run(&mut link, 45).await;
    let end = last_sample(&ev);
    assert!(
        end.cadence_rpm.expect("sim") > 70 && end.power_w.expect("sim") > 220,
        "{end:?}"
    );
}

// ---- heart ----

async fn hr_events(rig: &SimRig, secs: u64) -> Vec<HrEvent> {
    let mut link = rig
        .heart()
        .connect(&DeviceRef("hr".into()))
        .await
        .expect("sim");
    sleep(s(secs) + Duration::from_millis(1)).await;
    let mut out = Vec::new();
    while let Ok(e) = link.events.try_recv() {
        out.push(e);
    }
    out
}
fn hr_samples(ev: &[HrEvent]) -> Vec<&HrSample> {
    ev.iter()
        .filter_map(|e| {
            if let HrEvent::Sample(x) = e {
                Some(x)
            } else {
                None
            }
        })
        .collect()
}
#[tokio::test(start_paused = true)]
async fn heart_emits_one_hz_samples_with_rr() {
    let r = rig(vec![]);
    let ev = hr_events(&r, 20).await;
    let s = hr_samples(&ev);
    assert_eq!(s.len(), 20);
    assert!(
        s.iter()
            .all(|x| x.bpm.is_some() && !x.rr_ms.is_empty() && x.contact == Some(true))
    );
    assert!(s.windows(2).all(|w| w[1].at.0 - w[0].at.0 == 1_000_000));
    assert!((600..1400).contains(&s[10].rr_ms[0]));
}
#[tokio::test(start_paused = true)]
async fn natural_strap_dropout_reports_no_contact_without_rr() {
    // with the rider's free effort, seed 32 drops out at 84–88 s (found by search; the
    // vector's seed 31 drop at 66–69 s needs its workout power, covered in sim_parity)
    let cfg = SimConfig {
        seed: 32,
        ..SimConfig::default()
    };
    let r = SimRig::new(cfg, Arc::new(TestClock(Instant::now())));
    let ev = hr_events(&r, 95).await;
    let dropped: Vec<_> = hr_samples(&ev)
        .into_iter()
        .filter(|x| x.bpm.is_none())
        .collect();
    assert_eq!(dropped.len(), 5);
    assert!(dropped.iter().all(|x| x.contact == Some(false)));
    // the first dropped reading may still carry beats from just before the drop
    assert!(dropped[1..].iter().all(|x| x.rr_ms.is_empty()));
}
#[tokio::test(start_paused = true)]
async fn injected_hr_loss_drops_the_link_and_recovers() {
    let r = rig(vec![SimFault::HrLoss {
        at: s(10),
        down: s(6),
    }]);
    let ev = hr_events(&r, 30).await;
    assert!(ev.contains(&HrEvent::Link(LinkState::Lost)));
    assert!(
        hr_samples(&ev)
            .iter()
            .all(|x| !(10_000_000..16_000_000).contains(&x.at.0))
    );
    assert!(hr_samples(&ev).iter().any(|x| x.at.0 > 16_000_000));
    assert_eq!(
        ev.iter()
            .filter(|e| **e == HrEvent::Link(LinkState::Ready))
            .count(),
        2
    );
}
#[tokio::test(start_paused = true)]
async fn trainer_and_strap_share_one_rider() {
    let r = rig(vec![]);
    let mut t = connect(&r).await;
    let mut h = r
        .heart()
        .connect(&DeviceRef("hr".into()))
        .await
        .expect("sim");
    set(&t, ControlMode::Erg { target_w: 100 }).await;
    run(&mut t, 120).await;
    set(&t, ControlMode::Erg { target_w: 300 }).await;
    run(&mut t, 120).await;
    let mut last = None;
    while let Ok(e) = h.events.try_recv() {
        if let HrEvent::Sample(x) = e {
            if x.at.0 < 100_000_000 {
                continue;
            }
            last = Some(x);
        }
    }
    assert!(
        last.expect("sim").bpm.expect("sim") > 130,
        "harder work raises the strap's HR"
    );
}
