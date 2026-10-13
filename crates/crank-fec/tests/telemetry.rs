//! ADR-0002 §4.2: hand-computed telemetry and invalid-sentinel vectors.
use crank_fec::{Page, decode};

fn frame(p: [u8; 8]) -> [u8; 13] {
    let mut b = [0xa4, 9, 0x4e, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    b[4..12].copy_from_slice(&p);
    b[12] = b[..12].iter().fold(0, |a, b| a ^ b);
    b
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.0001, "{a} != {b}");
}

#[test]
fn telemetry_fields_and_sentinels() {
    let Page::GeneralFe(p) = decode(&frame([16, 25, 255, 255, 0x39, 0x30, 150, 0xb4]))
        .expect("page16")
        .page
    else {
        panic!("general")
    };
    near(p.elapsed_s, 63.75);
    near(p.speed_mps.expect("speed"), 12.345);
    assert_eq!(
        (p.distance_m, p.heart_rate_bpm, p.state, p.lap_toggle),
        (255, Some(150), 3, true)
    );
    let Page::Trainer(p) = decode(&frame([25, 255, 90, 0x34, 0x12, 0xbc, 0x7a, 0xb2]))
        .expect("page25")
        .page
    else {
        panic!("trainer")
    };
    assert_eq!(
        (
            p.event_count,
            p.cadence_rpm,
            p.accumulated_power_w,
            p.power_w,
            p.status,
            p.target_limit,
            p.state,
            p.lap_toggle
        ),
        (255, Some(90), 4660, Some(2748), 7, 2, 3, true)
    );
    let Page::Trainer(p) = decode(&frame([25, 0, 255, 255, 255, 255, 255, 3]))
        .expect("sentinels")
        .page
    else {
        panic!("trainer")
    };
    assert_eq!(
        (
            p.cadence_rpm,
            p.power_w,
            p.accumulated_power_w,
            p.target_limit
        ),
        (None, None, 65535, 3)
    );
    let Page::GeneralFe(p) = decode(&frame([16, 25, 0, 0, 255, 255, 255, 0]))
        .expect("sentinels")
        .page
    else {
        panic!("general")
    };
    assert_eq!((p.speed_mps, p.heart_rate_bpm), (None, None));
}

#[test]
fn capabilities_status_and_identity() {
    let Page::Capabilities(p) = decode(&frame([54, 255, 255, 255, 255, 0x34, 0x12, 7]))
        .expect("caps")
        .page
    else {
        panic!("caps")
    };
    assert_eq!((p.max_resistance_n, p.modes), (Some(4660), 7));
    let Page::CommandStatus(p) = decode(&frame([71, 49, 255, 4, 0xff, 0xff, 33, 3]))
        .expect("observation")
        .page
    else {
        panic!("status")
    };
    assert_eq!(
        (p.last_page, p.sequence, p.status, p.echo),
        (49, 255, 4, [255, 255, 33, 3])
    );
    let Page::Manufacturer(p) = decode(&frame([80, 255, 255, 7, 0x34, 0x12, 0x78, 0x56]))
        .expect("manufacturer")
        .page
    else {
        panic!("manufacturer")
    };
    assert_eq!(
        (p.hardware_revision, p.manufacturer_id, p.model),
        (7, 4660, 22136)
    );
    let Page::Product(p) = decode(&frame([81, 255, 9, 7, 0x78, 0x56, 0x34, 0x12]))
        .expect("product")
        .page
    else {
        panic!("product")
    };
    assert_eq!(
        (p.software_revision, p.supplemental_revision, p.serial),
        (7, Some(9), 0x1234_5678)
    );
}

#[test]
fn user_configuration_sentinels() {
    let Page::UserConfig {
        rider_kg,
        bike_kg,
        wheel_m,
        gear_ratio,
    } = decode(&frame([55, 255, 255, 255, 255, 255, 255, 0]))
        .expect("user")
        .page
    else {
        panic!("user")
    };
    assert_eq!(
        (rider_kg, bike_kg, wheel_m, gear_ratio),
        (None, None, None, None)
    );
    let Page::UserConfig {
        wheel_m,
        gear_ratio,
        ..
    } = decode(&frame([55, 0, 0, 255, 15, 0, 70, 100]))
        .expect("user")
        .page
    else {
        panic!("user")
    };
    near(wheel_m.expect("coarse only"), 0.7);
    near(gear_ratio.expect("gear ratio"), 3.0);
}

#[test]
fn optional_controls_and_common_page_sentinels() {
    assert_eq!(
        decode(&frame([48, 255, 255, 255, 255, 255, 255, 255]))
            .expect("resistance")
            .page,
        Page::BasicResistance { pct: None }
    );
    assert_eq!(
        decode(&frame([50, 255, 255, 255, 255, 255, 255, 255]))
            .expect("wind")
            .page,
        Page::Wind {
            cw_kg_per_m: None,
            speed_kmh: None,
            drafting: None
        }
    );
    assert_eq!(
        decode(&frame([51, 255, 255, 255, 255, 255, 255, 255]))
            .expect("track")
            .page,
        Page::Track {
            grade_pct: None,
            crr: 0.01275
        }
    );
    assert_eq!(
        decode(&frame([49, 255, 255, 255, 255, 255, 255, 255]))
            .expect("power")
            .page,
        Page::TargetPower { watts: 16_383.75 }
    );
    let Page::Capabilities(p) = decode(&frame([54, 255, 255, 255, 255, 255, 255, 0xf8]))
        .expect("caps")
        .page
    else {
        panic!("capabilities")
    };
    assert_eq!((p.max_resistance_n, p.modes), (None, 0xf8));
    let Page::Product(p) = decode(&frame([81, 255, 255, 7, 255, 255, 255, 255]))
        .expect("product")
        .page
    else {
        panic!("product")
    };
    assert_eq!((p.supplemental_revision, p.serial), (None, u32::MAX));
    // Reserved wheel offsets cannot produce plausible measurements.
    for offset in 10..=14 {
        let Page::UserConfig { wheel_m, .. } = decode(&frame([55, 0, 0, 255, offset, 0, 70, 0]))
            .expect("user")
            .page
        else {
            panic!("user")
        };
        assert_eq!(wheel_m, None);
    }
}
