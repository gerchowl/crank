//! #226: the page-71 reply of an unsupported page request, decoded by the real `crank-ble`
//! notification mapping from the captured NEO frame, is neutral to the trainer-following
//! machine (ADR-0002 §4.4.1 R1). `crank-engine` may not depend on this crate, so the test
//! lives on this side of the edge.
use crate::telemetry::receive;
use crank_devices::{
    CommandStatus, ControlMode, MonoTime, ObservedControl, TrainerEvent, TrainerObservation,
};
use crank_engine::TrainerFollowing;
use crank_engine::erg::following::{Machine, SEC};

/// `docs/hil/captures/check4.fecap` lines 48–50: the page-70 request for page 16, the
/// page-25 frame and the reply `47 10 DD 02 FF FF FF FF` (page 71, last page 0x10, status 2).
fn captured_reply() -> Vec<u8> {
    let cap = include_str!("../../../docs/hil/captures/check4.fecap");
    let line = cap.lines().nth(49).expect("line 50 of check4.fecap");
    assert!(line.contains("rx A4 09 4F 05 47 10 DD 02"), "{line}");
    line.split_whitespace()
        .skip(2)
        .map(|b| u8::from_str_radix(b, 16).expect("hex"))
        .collect()
}
fn observed(bytes: &[u8]) -> TrainerObservation {
    let mut events = receive(MonoTime(0), bytes);
    assert_eq!(events.len(), 1, "{events:?}");
    match events.remove(0) {
        TrainerEvent::Observed(o) => o,
        other => panic!("{other:?}"),
    }
}
#[test]
fn the_captured_unsupported_request_reply_decodes_to_notcontrol_notsupported() {
    let o = observed(&captured_reply());
    assert_eq!(o.control, Some(ObservedControl::NotControl(0x10)));
    assert_eq!(o.status, Some(CommandStatus::NotSupported));
}
#[test]
fn it_is_neutral_to_the_following_machine_in_every_mode() {
    let o = observed(&captured_reply());
    let modes = [
        ControlMode::Erg { target_w: 200 },
        ControlMode::Free,
        ControlMode::Resistance { pct: 20.0 },
    ];
    for mode in modes {
        let mut m = Machine::new();
        m.set_link(true, 0);
        m.set_desired(Some(mode.grid_key().expect("in range")), 0);
        let before = m.components();
        m.observe(&o, 2 * SEC);
        assert_eq!(m.components(), before, "{mode:?}");
        assert_eq!(
            m.verdict(),
            TrainerFollowing::Checking,
            "{mode:?}: R5 would disagree"
        );
    }
}
