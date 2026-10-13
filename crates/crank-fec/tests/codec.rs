//! ADR-0002 §4 acceptance: independent vectors, capture replay and properties.
use crank_fec::{ControlPage as C, FrameError as E, MsgType, Page, decode, encode};
use proptest::prelude::*;

fn frame(p: [u8; 8]) -> [u8; 13] {
    let mut b = [0xa4, 9, 0x4e, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    b[4..12].copy_from_slice(&p);
    b[12] = b[..12].iter().fold(0, |a, b| a ^ b);
    b
}
fn payload(c: C) -> [u8; 8] {
    encode(&c).expect("valid control")[4..12]
        .try_into()
        .expect("eight bytes")
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.0001, "{a} != {b}");
}

#[test]
fn replay_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut count = 0;
    for file in std::fs::read_dir(dir).expect("fixture directory") {
        let path = file.expect("fixture entry").path();
        if path.extension().is_none_or(|e| e != "fecap") {
            continue;
        }
        for line in std::fs::read_to_string(path).expect("capture").lines() {
            let line = line.split('#').next().expect("line").trim();
            if line.is_empty() {
                continue;
            }
            let b: Vec<_> = line
                .split_whitespace()
                .map(|s| u8::from_str_radix(s, 16).expect("hex byte"))
                .collect();
            let f = decode(&b).expect("valid captured frame");
            assert_eq!(f.msg_type, MsgType::Broadcast);
            count += 1;
        }
    }
    assert!(count > 0, "fixtures must not be empty");
    let f = decode(&[0xa4, 9, 0x4e, 5, 0x10, 0x19, 4, 0, 0, 0, 0xff, 0x24, 0x30])
        .expect("Tacx worked example");
    let Page::GeneralFe(p) = f.page else {
        panic!("page 16")
    };
    assert_eq!(
        (f.channel, p.equipment_type, p.distance_m, p.state, p.flags),
        (5, 25, 0, 2, 4)
    );
    assert_eq!(p.heart_rate_bpm, None);
    near(p.elapsed_s, 1.0);
    near(p.speed_mps.expect("speed"), 0.0);
}

#[test]
fn independent_unit_vectors() {
    assert_eq!(
        payload(C::BasicResistance { pct: 37.5 }),
        [48, 255, 255, 255, 255, 255, 255, 75]
    );
    assert_eq!(
        payload(C::TargetPower { watts: 200.25 }),
        [49, 255, 255, 255, 255, 255, 33, 3]
    );
    assert_eq!(
        payload(C::Wind {
            cw_kg_per_m: 0.51,
            speed_kmh: -20.0,
            drafting: 0.73
        }),
        [50, 255, 255, 255, 255, 51, 107, 73]
    );
    assert_eq!(
        payload(C::Track {
            grade_pct: 0.0,
            crr: 0.004
        }),
        [51, 255, 255, 255, 255, 0x20, 0x4e, 80]
    );
    assert_eq!(
        payload(C::Track {
            grade_pct: -5.25,
            crr: 0.005
        }),
        [51, 255, 255, 255, 255, 0x13, 0x4c, 100]
    );
    assert_eq!(
        payload(C::UserConfig {
            rider_kg: 75.0,
            bike_kg: 10.0,
            wheel_m: 0.708
        }),
        [55, 0x4c, 0x1d, 255, 0x88, 0x0c, 70, 0]
    );
    assert_eq!(
        payload(C::UserConfig {
            rider_kg: 0.0,
            bike_kg: 0.0,
            wheel_m: 0.708
        })[4..7],
        [8, 0, 70]
    );
    assert_eq!(
        payload(C::Request { page: 71, times: 3 }),
        [70, 255, 255, 255, 255, 131, 71, 1]
    );
    assert_eq!(payload(C::TargetPower { watts: 0.125 })[6..8], [1, 0]);
    assert_eq!(payload(C::BasicResistance { pct: 0.24 })[7], 0);
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
        (7, Some(9), 0x12345678)
    );
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
fn rejects_malformed_frames() {
    let b = frame([99, 1, 2, 3, 4, 5, 6, 7]);
    for n in 0..13 {
        assert_eq!(decode(&b[..n]), Err(E::Truncated));
    }
    let mut longer = b.to_vec();
    longer.push(0);
    assert_eq!(decode(&longer), Err(E::Length));
    for (index, value, error) in [
        (0, 0, E::Sync),
        (1, 8, E::Length),
        (12, b[12] ^ 1, E::Checksum),
    ] {
        let mut bad = b;
        bad[index] = value;
        assert_eq!(decode(&bad), Err(error));
    }
    let mut bad = b;
    bad[2] = 0x50;
    bad[12] ^= 0x4e ^ 0x50;
    assert_eq!(decode(&bad), Err(E::MessageType(0x50)));
    assert_eq!(
        decode(&b).expect("unknown").page,
        Page::Unknown(99, [99, 1, 2, 3, 4, 5, 6, 7])
    );
}

#[test]
fn range_checks_precede_rounding() {
    for v in [-1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for c in [
            C::BasicResistance { pct: v },
            C::TargetPower { watts: v },
            C::Wind {
                cw_kg_per_m: v,
                speed_kmh: 0.0,
                drafting: 1.0,
            },
            C::Wind {
                cw_kg_per_m: 0.5,
                speed_kmh: 0.0,
                drafting: v,
            },
            C::Track {
                grade_pct: 0.0,
                crr: v,
            },
            C::UserConfig {
                rider_kg: v,
                bike_kg: 0.0,
                wheel_m: 0.7,
            },
            C::UserConfig {
                rider_kg: 75.0,
                bike_kg: v,
                wheel_m: 0.7,
            },
            C::UserConfig {
                rider_kg: 75.0,
                bike_kg: 10.0,
                wheel_m: v,
            },
        ] {
            assert!(encode(&c).is_err(), "{c:?}");
        }
    }
    for c in [
        C::BasicResistance { pct: 100.001 },
        C::TargetPower { watts: 16383.76 },
        C::Wind {
            cw_kg_per_m: 1.861,
            speed_kmh: 0.0,
            drafting: 1.0,
        },
        C::Wind {
            cw_kg_per_m: 0.5,
            speed_kmh: 0.0,
            drafting: 1.001,
        },
        C::Track {
            grade_pct: 200.001,
            crr: 0.0,
        },
        C::Track {
            grade_pct: -200.001,
            crr: 0.0,
        },
        C::Track {
            grade_pct: 0.0,
            crr: 0.012751,
        },
        C::UserConfig {
            rider_kg: 655.35,
            bike_kg: 0.0,
            wheel_m: 0.7,
        },
        C::UserConfig {
            rider_kg: 75.0,
            bike_kg: 204.75,
            wheel_m: 0.7,
        },
        C::UserConfig {
            rider_kg: 75.0,
            bike_kg: 10.0,
            wheel_m: 2.541,
        },
        C::Request {
            page: 71,
            times: 128,
        },
    ] {
        assert!(encode(&c).is_err(), "{c:?}");
    }
    for v in [-127.01, 127.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            encode(&C::Wind {
                cw_kg_per_m: 0.5,
                speed_kmh: v,
                drafting: 1.0
            })
            .expect_err("speed range")
            .field,
            "speed_kmh"
        );
    }
    for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(
            encode(&C::Track {
                grade_pct: v,
                crr: 0.0
            })
            .is_err()
        );
    }
    assert_eq!(
        payload(C::TargetPower { watts: 16383.75 })[6..8],
        [255, 255]
    );
    assert_eq!(
        payload(C::Track {
            grade_pct: 200.0,
            crr: 0.01275
        })[5..8],
        [0x40, 0x9c, 255]
    );
    assert_eq!(
        payload(C::Track {
            grade_pct: -200.0,
            crr: 0.0
        })[5..8],
        [0, 0, 0]
    );
    assert_eq!(
        payload(C::UserConfig {
            rider_kg: 655.34,
            bike_kg: 204.7,
            wheel_m: 2.54
        }),
        [55, 254, 255, 255, 224, 255, 254, 0]
    );
}

proptest! {
    #[test]
    fn decode_never_panics(b in proptest::collection::vec(any::<u8>(),0..512)) { let _ = decode(&b); }
    #[test]
    fn arbitrary_valid_envelopes(p in any::<[u8;8]>()) { prop_assert!(decode(&frame(p)).is_ok()); }
    #[test]
    fn every_control_round_trips(res in 0u16..=200, power in any::<u16>(), cw in 0u16..=186, speed in -127i16..=127, draft in 0u16..=100, grade in 0u16..=40000, crr in 0u16..=255, rider in 0u16..=65534, bike in 0u16..=4094, wheel in 0u16..=2540, page in any::<u8>(), times in 0u8..=127) {
        let controls = [C::BasicResistance{pct:f32::from(res)*0.5},C::TargetPower{watts:f32::from(power)*0.25},C::Wind{cw_kg_per_m:f32::from(cw)/100.0,speed_kmh:f32::from(speed),drafting:f32::from(draft)/100.0},C::Track{grade_pct:f32::from(grade)/100.0-200.0,crr:f32::from(crr)/20000.0},C::UserConfig{rider_kg:f32::from(rider)/100.0,bike_kg:f32::from(bike)/20.0,wheel_m:f32::from(wheel)/1000.0},C::Request{page,times}];
        for c in controls {
            let encoded = encode(&c).expect("valid control");
            let f = decode(&encoded).expect("encoded frame");
            prop_assert_eq!(f.msg_type,MsgType::Acknowledged); prop_assert_eq!(f.channel,5);
            let again = match f.page {
                Page::BasicResistance{pct} => C::BasicResistance{pct:pct.expect("resistance")},
                Page::TargetPower{watts} => C::TargetPower{watts},
                Page::Wind{cw_kg_per_m,speed_kmh,drafting} => C::Wind{cw_kg_per_m:cw_kg_per_m.expect("cw"),speed_kmh:speed_kmh.expect("wind"),drafting:drafting.expect("drafting")},
                Page::Track{grade_pct,crr} => C::Track{grade_pct:grade_pct.expect("grade"),crr},
                Page::UserConfig{rider_kg,bike_kg,wheel_m,gear_ratio} => { prop_assert_eq!(gear_ratio,None); C::UserConfig{rider_kg:rider_kg.expect("rider"),bike_kg:bike_kg.expect("bike"),wheel_m:wheel_m.expect("wheel")} },
                Page::Request{page,times,acknowledged,command_type} => { prop_assert!(acknowledged); prop_assert_eq!(command_type,1); C::Request{page,times} },
                other => panic!("unexpected control: {other:?}"),
            };
            prop_assert_eq!(encode(&again).expect("decoded control"),encoded);
        }
    }
}
