//! Executable invariants for the ADR-0002 §3/§4.4/§4.4.1 abstraction.
use crank_devices::*;
use futures_util::{future::BoxFuture, stream::BoxStream};

fn sim(grade_pct: f32, crr: f32, wind: Wind) -> ControlMode {
    ControlMode::Sim {
        grade_pct,
        crr,
        wind,
    }
}
fn encode_error(mode: ControlMode) {
    assert!(
        matches!(
            mode.grid_key(),
            Err(DeviceError::Protocol {
                kind: ProtocolErrorKind::Encode,
                ..
            })
        ),
        "{mode:?}"
    );
}
#[test]
fn erg_entire_integer_domain_is_checked_and_exact() {
    for target_w in 0..=u16::MAX {
        let mode = ControlMode::Erg { target_w };
        if target_w <= 16383 {
            assert_eq!(
                mode.grid_key().unwrap(),
                vec![ObservedControl::Erg {
                    quarter_w: target_w * 4
                }]
            );
        } else {
            encode_error(mode);
        }
    }
}
#[test]
fn resistance_rounding_and_every_wire_value() {
    for half_pct in 0..=200u8 {
        assert_eq!(
            ControlMode::Resistance {
                pct: f32::from(half_pct) / 2.0
            }
            .grid_key()
            .unwrap(),
            vec![ObservedControl::Resistance { half_pct }]
        );
    }
    for (pct, half_pct) in [(0.249, 0), (0.25, 1), (0.251, 1), (99.75, 200)] {
        assert_eq!(
            ControlMode::Resistance { pct }.grid_key().unwrap(),
            vec![ObservedControl::Resistance { half_pct }]
        );
    }
}
#[test]
fn sim_projection_has_two_independent_signed_components() {
    assert_eq!(
        sim(
            -3.125,
            0.004,
            Wind {
                cw_kg_per_m: 0.515,
                speed_kmh: -12.5,
                drafting: 0.755
            }
        )
        .grid_key()
        .unwrap(),
        vec![
            ObservedControl::Wind {
                cw_centi: 51,
                speed_kmh: -12,
                drafting_centi: 75
            },
            ObservedControl::Track {
                grade_centi_pct: -312,
                crr_units: 80
            }
        ]
    );
    // f32 0.515 and 0.755 are below the decimal midpoint. These expected
    // integers deliberately reflect the API's f32 values, as crank-fec does.
    for (grade_pct, grade_centi_pct) in [(-200.0, -20000), (0.0, 0), (200.0, 20000)] {
        let key = sim(
            grade_pct,
            0.0127,
            Wind {
                cw_kg_per_m: 1.86,
                speed_kmh: -127.0,
                drafting: 1.0,
            },
        )
        .grid_key()
        .unwrap();
        assert_eq!(
            key,
            vec![
                ObservedControl::Wind {
                    cw_centi: 186,
                    speed_kmh: -127,
                    drafting_centi: 100
                },
                ObservedControl::Track {
                    grade_centi_pct,
                    crr_units: 254
                }
            ]
        );
    }
    assert_eq!(
        sim(
            0.0,
            0.0,
            Wind {
                cw_kg_per_m: 0.0,
                speed_kmh: 127.0,
                drafting: 0.0
            }
        )
        .grid_key()
        .unwrap()[0],
        ObservedControl::Wind {
            cw_centi: 0,
            speed_kmh: 127,
            drafting_centi: 0
        }
    );
}
#[test]
fn free_is_explicit_flat_road_sim_with_adr_defaults() {
    assert_eq!(
        ControlMode::Free.grid_key().unwrap(),
        vec![
            ObservedControl::Wind {
                cw_centi: 51,
                speed_kmh: 0,
                drafting_centi: 100
            },
            ObservedControl::Track {
                grade_centi_pct: 0,
                crr_units: 80
            },
        ]
    );
    assert_eq!(
        ControlMode::Free.grid_key(),
        sim(0.0, 0.004, Wind::default()).grid_key()
    );
}
#[test]
fn invalid_fields_are_rejected_before_rounding_without_clamping() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        encode_error(ControlMode::Resistance { pct: bad });
        encode_error(sim(bad, 0.004, Wind::default()));
        encode_error(sim(0.0, bad, Wind::default()));
        encode_error(sim(
            0.0,
            0.004,
            Wind {
                cw_kg_per_m: bad,
                ..Wind::default()
            },
        ));
        encode_error(sim(
            0.0,
            0.004,
            Wind {
                speed_kmh: bad,
                ..Wind::default()
            },
        ));
        encode_error(sim(
            0.0,
            0.004,
            Wind {
                drafting: bad,
                ..Wind::default()
            },
        ));
    }
    for bad in [-0.001, 100.001] {
        encode_error(ControlMode::Resistance { pct: bad });
    }
    for bad in [-200.001, 200.001] {
        encode_error(sim(bad, 0.004, Wind::default()));
    }
    for bad in [-0.00001, 0.01276] {
        encode_error(sim(0.0, bad, Wind::default()));
    }
    for bad in [-0.001, 1.861] {
        encode_error(sim(
            0.0,
            0.004,
            Wind {
                cw_kg_per_m: bad,
                ..Wind::default()
            },
        ));
    }
    for bad in [-127.001, 127.001] {
        encode_error(sim(
            0.0,
            0.004,
            Wind {
                speed_kmh: bad,
                ..Wind::default()
            },
        ));
    }
    for bad in [-0.001, 1.001] {
        encode_error(sim(
            0.0,
            0.004,
            Wind {
                drafting: bad,
                ..Wind::default()
            },
        ));
    }
}
#[test]
fn all_status_bytes_are_preserved_and_target_flags_decode() {
    let known = [
        CommandStatus::Pass,
        CommandStatus::Fail,
        CommandStatus::NotSupported,
        CommandStatus::Rejected,
        CommandStatus::Pending,
    ];
    for value in 0..=u8::MAX {
        let expected = known
            .get(usize::from(value))
            .copied()
            .unwrap_or(CommandStatus::Other(value));
        assert_eq!(CommandStatus::from(value), expected);
    }
    for (raw, expected) in [
        (0, TargetLimit::AtTarget),
        (1, TargetLimit::SpeedTooLow),
        (2, TargetLimit::SpeedTooHigh),
        (3, TargetLimit::Undetermined),
        (255, TargetLimit::Undetermined),
    ] {
        assert_eq!(TargetLimit::from(raw), expected);
    }
}
struct Factory;
impl TrainerDriver for Factory {
    fn kind(&self) -> DriverKind {
        DriverKind::Sim
    }
    fn connect(
        &self,
        _: &DeviceRef,
        _: RiderConfig,
    ) -> BoxFuture<'static, Result<TrainerLink, DeviceError>> {
        Box::pin(async { Err(DeviceError::AdapterUnavailable) })
    }
}
impl HeartRateDriver for Factory {
    fn connect(&self, _: &DeviceRef) -> BoxFuture<'static, Result<HeartRateLink, DeviceError>> {
        Box::pin(async { Err(DeviceError::AdapterUnavailable) })
    }
}
impl Discovery for Factory {
    fn scan(&self) -> BoxStream<'static, DiscoveredDevice> {
        Box::pin(futures_util::stream::empty())
    }
}
#[test]
fn factories_are_object_safe_and_connection_futures_own_their_inputs() {
    let trainer: Box<dyn TrainerDriver> = Box::new(Factory);
    let hr: Box<dyn HeartRateDriver> = Box::new(Factory);
    let discovery: Box<dyn Discovery> = Box::new(Factory);
    let rider = RiderConfig {
        rider_kg: 75.0,
        bike_kg: 10.0,
        wheel_m: 0.7,
    };
    let (trainer_future, hr_future) = {
        let device = DeviceRef("host-local-id".into());
        (trainer.connect(&device, rider), hr.connect(&device))
    };
    assert_eq!(trainer.kind(), DriverKind::Sim);
    drop((trainer, hr, discovery, trainer_future, hr_future));
}

// ---- #153: errors name the bad field; Display; PartialEq; status nibble ----

/// The `detail` of the encode error `mode` is rejected with.
fn rejected_detail(mode: ControlMode) -> String {
    match mode.grid_key() {
        Err(DeviceError::Protocol {
            kind: ProtocolErrorKind::Encode,
            detail,
        }) => detail,
        other => panic!("{mode:?} was not rejected: {other:?}"),
    }
}
/// Whether `detail` names `field` as a whole identifier (not as a suffix of a longer one).
fn mentions(detail: &str, field: &str) -> bool {
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    detail.match_indices(field).any(|(at, _)| {
        !detail[..at].chars().next_back().is_some_and(ident)
            && !detail[at + field.len()..].chars().next().is_some_and(ident)
    })
}
#[test]
fn crr_is_capped_at_the_largest_encodable_value() {
    // 0.0127 is wire unit 254, the largest the NEO accepts (ADR-0002 §4.3, HIL #44 Part A).
    assert_eq!(
        sim(0.0, 0.0127, Wind::default()).grid_key().unwrap()[1],
        ObservedControl::Track {
            grade_centi_pct: 0,
            crr_units: 254
        }
    );
    // 0.012725 and 0.01275 round to unit 255 (the NEO ignores it); anything wider is out of range
    // too. All fail at the mode boundary naming `crr`, not later as the BLE writer's encode error.
    for crr in [0.012_725, 0.012_74, 0.01275, 0.0128, 0.02] {
        let detail = rejected_detail(sim(0.0, crr, Wind::default()));
        assert!(
            detail.contains("crr") && detail.contains("outside its range"),
            "{crr}: {detail}"
        );
    }
}
#[test]
fn each_cause_gets_its_own_message() {
    let non_finite = rejected_detail(ControlMode::Resistance { pct: f32::NAN });
    let range = rejected_detail(ControlMode::Resistance { pct: 100.5 });
    let overflow = rejected_detail(ControlMode::Erg { target_w: 16384 });
    assert!(
        non_finite.contains("not finite") && !non_finite.contains("outside"),
        "{non_finite}"
    );
    assert!(
        range.contains("outside its range") && !range.contains("not finite"),
        "{range}"
    );
    assert!(
        overflow.contains("overflows")
            && !overflow.contains("not finite")
            && !overflow.contains("range"),
        "{overflow}"
    );
}
#[test]
fn each_rejected_field_is_named_in_its_error() {
    let wind = |f: fn(&mut Wind)| {
        let mut w = Wind::default();
        f(&mut w);
        w
    };
    let cases: [(&str, ControlMode); 7] = [
        ("target_w", ControlMode::Erg { target_w: 16384 }),
        ("pct", ControlMode::Resistance { pct: 100.5 }),
        ("grade_pct", sim(200.5, 0.004, Wind::default())),
        ("crr", sim(0.0, 0.02, Wind::default())),
        (
            "wind.cw_kg_per_m",
            sim(0.0, 0.004, wind(|w| w.cw_kg_per_m = 2.0)),
        ),
        (
            "wind.speed_kmh",
            sim(0.0, 0.004, wind(|w| w.speed_kmh = 128.0)),
        ),
        (
            "wind.drafting",
            sim(0.0, 0.004, wind(|w| w.drafting = f32::NAN)),
        ),
    ];
    for (field, mode) in cases {
        let detail = rejected_detail(mode);
        assert!(detail.contains(field), "{field}: {detail}");
        // and only that field: the other names must not appear
        for (other, _) in &cases {
            // `wind.speed_kmh` etc. legitimately contain no other listed name, but `pct` is a
            // substring of `grade_pct`, so compare whole tokens of the message, not substrings.
            if *other != field {
                assert!(
                    !mentions(&detail, other),
                    "{field} blamed {other}: {detail}"
                );
            }
        }
    }
}
#[test]
fn a_protocol_error_prints_its_kind_with_display_not_debug() {
    let e = DeviceError::Protocol {
        kind: ProtocolErrorKind::UnexpectedPage,
        detail: "page 0x42".into(),
    };
    assert_eq!(e.to_string(), "protocol unexpected page: page 0x42");
    assert_eq!(
        DeviceError::Protocol {
            kind: ProtocolErrorKind::Checksum,
            detail: "x".into()
        }
        .to_string(),
        "protocol checksum: x"
    );
}
#[test]
fn discovered_devices_compare_by_value() {
    let d = || DiscoveredDevice {
        device: DeviceRef("a".into()),
        info: DeviceInfo::default(),
        trainer: Some(DriverKind::Sim),
        heart_rate: false,
    };
    assert_eq!(d(), d());
    let mut other = d();
    other.heart_rate = true;
    assert_ne!(d(), other);
}
#[test]
fn trainer_status_keeps_only_the_four_bit_nibble() {
    for raw in 0..=u8::MAX {
        assert_eq!(TrainerStatus::from_raw(raw).raw(), raw & 0x0F);
    }
}
