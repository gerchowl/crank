//! ADR-0002 §4: exact round trips on the wire grid and bounded panic checks.
use crank_fec::{ControlPage as C, MsgType, Page, decode, encode};
use proptest::prelude::*;

fn round_trip(control: C, expected: Page) {
    let bytes = encode(&control).expect("wire-grid control");
    let decoded = decode(&bytes).expect("encoded frame");
    assert_eq!(decoded.msg_type, MsgType::Acknowledged);
    assert_eq!(decoded.channel, 5);
    assert_eq!(decoded.page, expected);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn basic_resistance(units in 0u16..=200) {
        let pct = f32::from(units) / 2.0;
        round_trip(C::BasicResistance { pct }, Page::BasicResistance { pct: Some(pct) });
    }
    #[test]
    fn target_power(units in any::<u16>()) {
        let watts = f32::from(units) / 4.0;
        round_trip(C::TargetPower { watts }, Page::TargetPower { watts });
    }
    #[test]
    fn wind(cw in 0u8..=186, speed in -127i16..=127, draft in 0u8..=100) {
        let cw_kg_per_m = f32::from(cw) / 100.0;
        let speed_kmh = f32::from(speed);
        let drafting = f32::from(draft) / 100.0;
        round_trip(C::Wind { cw_kg_per_m, speed_kmh, drafting }, Page::Wind {
            cw_kg_per_m: Some(cw_kg_per_m), speed_kmh: Some(speed_kmh), drafting: Some(drafting),
        });
    }
    #[test]
    fn track(grade in -20_000i16..=20_000, rolling in any::<u8>()) {
        let grade_pct = f32::from(grade) / 100.0;
        let crr = f32::from(rolling) / 20_000.0;
        round_trip(C::Track { grade_pct, crr }, Page::Track { grade_pct: Some(grade_pct), crr });
    }
    #[test]
    fn user_config(rider in 0u16..=65534, bike in 0u16..=4094, wheel in 0u16..=2540) {
        let rider_kg = f32::from(rider) / 100.0;
        let bike_kg = f32::from(bike) / 20.0;
        let wheel_m = f32::from(wheel) / 1000.0;
        round_trip(C::UserConfig { rider_kg, bike_kg, wheel_m }, Page::UserConfig {
            rider_kg: Some(rider_kg), bike_kg: Some(bike_kg), wheel_m: Some(wheel_m), gear_ratio: None,
        });
    }
    #[test]
    fn request(page in any::<u8>(), times in 0u8..=127) {
        round_trip(C::Request { page, times }, Page::Request {
            page, times, acknowledged: true, command_type: 1,
        });
    }
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = decode(&bytes);
    }
    #[test]
    fn arbitrary_valid_envelopes_never_panic(page in any::<[u8; 8]>()) {
        let mut bytes = [0xa4, 9, 0x4e, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        bytes[4..12].copy_from_slice(&page);
        bytes[12] = bytes[..12].iter().fold(0, |xor, byte| xor ^ byte);
        prop_assert!(decode(&bytes).is_ok());
    }
}
