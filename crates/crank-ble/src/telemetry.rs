//! Notification mapping without synthesis, deduplication or following policy.
use crank_devices::{
    MonoTime, ObservationSource, ObservedControl, TargetLimit, TrainerEvent, TrainerObservation,
    TrainerSample, TrainerStatus,
};
use crank_fec::{Page, decode};

pub(crate) fn echo(page: u8, e: [u8; 4]) -> ObservedControl {
    match page {
        48 => ObservedControl::Resistance { half_pct: e[3] },
        49 => ObservedControl::Erg {
            quarter_w: u16::from_le_bytes([e[2], e[3]]),
        },
        50 => ObservedControl::Wind {
            cw_centi: e[1],
            speed_kmh: i16::from(e[2]) - 127,
            drafting_centi: e[3],
        },
        51 => ObservedControl::Track {
            grade_centi_pct: i32::from(u16::from_le_bytes([e[1], e[2]])) - 20000,
            crr_units: e[3],
        },
        other => ObservedControl::NotControl(other),
    }
}
pub(crate) fn receive(at: MonoTime, bytes: &[u8]) -> Vec<TrainerEvent> {
    let page = match decode(bytes) {
        Ok(frame) => frame.page,
        Err(e) => return vec![TrainerEvent::Fault(crate::frame_error(e))],
    };
    let mut sample = TrainerSample {
        at,
        power_w: None,
        cadence_rpm: None,
        speed_mps: None,
        distance_m: None,
        accumulated_power_w: None,
        event_count: None,
        status: TrainerStatus::default(),
        target_limit: TargetLimit::Undetermined,
    };
    match page {
        Page::CommandStatus(p) => vec![TrainerEvent::Observed(TrainerObservation {
            at,
            source: ObservationSource::Page71Echo,
            control: Some(echo(p.last_page, p.echo)),
            status: Some(p.status.into()),
            target_limit: None,
        })],
        Page::GeneralFe(p) => {
            sample.speed_mps = p.speed_mps;
            sample.distance_m = Some(f32::from(p.distance_m));
            vec![TrainerEvent::Sample(sample)]
        }
        Page::Trainer(p) => {
            sample.power_w = p.power_w;
            sample.cadence_rpm = p.cadence_rpm;
            sample.accumulated_power_w = Some(u32::from(p.accumulated_power_w));
            sample.event_count = Some(p.event_count);
            sample.status = TrainerStatus(p.status);
            sample.target_limit = p.target_limit.into();
            vec![
                TrainerEvent::Sample(sample),
                TrainerEvent::Observed(TrainerObservation {
                    at,
                    source: ObservationSource::Page25Flags,
                    control: None,
                    status: None,
                    target_limit: Some(p.target_limit.into()),
                }),
            ]
        }
        Page::Unknown { page, raw } => {
            tracing::debug!(page, ?raw, "unknown FE-C page");
            vec![]
        }
        _ => vec![],
    }
}
