//! ADR-0002 §4 part A: independent control vectors and ANT framing checks.
use crank_fec::{ControlPage as C, FrameError as E, MsgType, Page, decode, encode};

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

#[test]
fn tacx_worked_example() {
    let bytes = [0xa4, 9, 0x4e, 5, 0x10, 0x19, 4, 0, 0, 0, 0xff, 0x24, 0x30];
    let f = decode(&bytes).expect("Tacx worked example, XOR including sync");
    assert_eq!(f.msg_type, MsgType::Broadcast);
    assert_eq!(f.channel, 5);
    let Page::GeneralFe(p) = f.page else {
        panic!("general FE")
    };
    assert!((p.elapsed_s - 1.0).abs() < f32::EPSILON);
    assert_eq!(p.equipment_type, 25);
    assert_eq!(p.state, 2);
    assert_eq!(p.flags, 4);
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
        Page::Unknown {
            page: 99,
            raw: [99, 1, 2, 3, 4, 5, 6, 7]
        }
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
}

#[test]
fn rejects_oversized_fields() {
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
            crr: 0.012_751,
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
}

#[test]
fn domain_endpoint_vectors() {
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

#[test]
fn page_55_frame_and_typed_diameter_errors() {
    let c = C::UserConfig {
        rider_kg: 75.0,
        bike_kg: 10.0,
        wheel_m: 0.708,
    };
    let b = encode(&c).expect("708 mm golden frame");
    assert_eq!(
        b,
        [
            0xa4, 9, 0x4f, 5, 55, 0x4c, 0x1d, 255, 0x88, 0x0c, 70, 0, 0xbc
        ]
    );
    let decoded = decode(&b).expect("acknowledged frame");
    assert_eq!(decoded.msg_type, MsgType::Acknowledged);
    assert_eq!(decoded.channel, 5);
    assert_eq!(
        decoded.page,
        Page::UserConfig {
            rider_kg: Some(75.0),
            bike_kg: Some(10.0),
            wheel_m: Some(0.708),
            gear_ratio: None,
        }
    );
    for wheel_m in [-0.001, 2.541, f32::NAN, f32::INFINITY] {
        let c = C::UserConfig {
            rider_kg: 75.0,
            bike_kg: 10.0,
            wheel_m,
        };
        assert_eq!(encode(&c), Err(crank_fec::EncodeError { field: "wheel_m" }));
    }
}
