//! Deterministic single-writer tick allocation (ADR-0002 §4.4 amendment #43).
use crate::protocol;
use crank_devices::{ControlMode, DeviceError, ProtocolErrorKind};
use crank_fec::{ControlPage, encode};

pub(crate) fn pages(mode: ControlMode) -> Result<Vec<[u8; 13]>, DeviceError> {
    mode.grid_key()?;
    let controls = match mode {
        ControlMode::Erg { target_w } => vec![ControlPage::TargetPower {
            watts: f32::from(target_w),
        }],
        ControlMode::Resistance { pct } => vec![ControlPage::BasicResistance { pct }],
        ControlMode::Free => {
            return pages(ControlMode::Sim {
                grade_pct: 0.0,
                crr: 0.004,
                wind: crank_devices::Wind::default(),
            });
        }
        ControlMode::Sim {
            grade_pct,
            crr,
            wind,
        } => vec![
            ControlPage::Wind {
                cw_kg_per_m: wind.cw_kg_per_m,
                speed_kmh: wind.speed_kmh,
                drafting: wind.drafting,
            },
            ControlPage::Track { grade_pct, crr },
        ],
    };
    controls
        .iter()
        .map(|p| encode(p).map_err(|e| protocol(e, ProtocolErrorKind::Encode)))
        .collect()
}
pub(crate) fn request(page: u8) -> Result<[u8; 13], DeviceError> {
    encode(&ControlPage::Request { page, times: 1 })
        .map_err(|e| protocol(e, ProtocolErrorKind::Encode))
}
#[derive(Default)]
pub(crate) struct Pacer {
    pub mode: Option<ControlMode>,
    current: Vec<[u8; 13]>,
    sent: Vec<[u8; 13]>,
    pending: std::collections::VecDeque<[u8; 13]>,
    pair_tail: bool,
    pair_completed: bool,
    poll: Option<u64>,
    last_poll: Option<u64>,
    keep: u64,
    component: usize,
}
impl Pacer {
    pub fn set(&mut self, mode: ControlMode, tick: u64) -> Result<(), DeviceError> {
        let next = pages(mode)?;
        let same_kind = self.current.first().map(|p| p[4]) == next.first().map(|p| p[4]);
        if !same_kind {
            self.sent.clear();
            self.pair_tail = false;
        }
        let mut changed: std::collections::VecDeque<_> = next
            .iter()
            .filter(|p| !self.sent.contains(p))
            .copied()
            .collect();
        if self.pair_tail && next.len() == 2 {
            changed.retain(|p| p[4] != 51);
            changed.push_front(next[1]);
        }
        if self.poll.is_none() {
            self.poll = Some(tick + 4);
        }
        if !same_kind {
            self.keep = if next.len() == 2 {
                self.poll.unwrap_or(tick + 4).max(tick + 1) - 1
            } else {
                tick + 8
            };
            self.component = 0;
        }
        self.pending = changed;
        self.current = next;
        self.mode = Some(mode);
        Ok(())
    }
    pub fn tick(&mut self, tick: u64) -> Result<Option<[u8; 13]>, DeviceError> {
        if self.mode.is_none() {
            return Ok(None);
        }
        let due = self.poll.is_some_and(|due| tick >= due);
        let pair = self.pending.len() == 2 && !self.pair_tail;
        if due
            && ((!pair && !self.pair_tail) || self.pair_completed)
            && self.last_poll != tick.checked_sub(1)
        {
            self.last_poll = Some(tick);
            if let Some(due) = self.poll.as_mut() {
                while *due <= tick {
                    *due += 4;
                }
            }
            self.pair_completed = false;
            return Ok(Some(request(71)?));
        }
        if let Some(frame) = self.pending.pop_front() {
            if self.pair_tail {
                self.pair_completed = due;
                let mut poll = self.poll.unwrap_or(tick + 4);
                while poll <= tick + 1 {
                    poll += 4;
                }
                self.keep = poll - 1;
                self.component = 0;
            } else if self.current.len() == 1 {
                self.keep = tick + 8;
            } else if tick >= self.keep && frame[4] == self.current[self.component][4] {
                self.component ^= 1;
                self.keep += 4;
            }
            self.pair_tail = pair;
            self.sent.retain(|p| p[4] != frame[4]);
            self.sent.push(frame);
            return Ok(Some(frame));
        }
        if tick >= self.keep {
            let frame = self.current[self.component];
            if self.current.len() == 2 {
                self.component ^= 1;
                self.keep += 4;
            } else {
                self.keep += 8;
            }
            return Ok(Some(frame));
        }
        Ok(None)
    }
}
