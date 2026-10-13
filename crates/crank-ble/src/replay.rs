//! Test-only `.fecap` replay through the real FE-C notification mapping.
use crate::{Clock, FecTrainer, FecTransport, Notifications};
use crank_devices::{
    DeviceError, DeviceInfo, DeviceRef, DriverKind, MonoTime, ProtocolErrorKind, RiderConfig,
    TrainerDriver, TrainerLink,
};
use futures_util::{future::BoxFuture, stream};
use std::sync::Arc;
/// Synthetic transport replay. Timing uses only the injected clock (ADR-0001 §5); no hardware claims.
/// Real #44 check-3 captures can replace the same input string.
pub struct ReplayTrainer(FecTrainer);
struct Replay(Vec<(MonoTime, Vec<u8>)>);
impl ReplayTrainer {
    /// Parse the existing crank-fec capture format without combining payloads.
    /// Captures must include the connection’s page-54 reply; TX records are
    /// validated but not replayed as engine commands. Arrival ties retain order.
    /// # Errors
    /// Rejects malformed records, unknown directions or decreasing timestamps.
    pub fn new(capture: &str, clock: Arc<dyn Clock>) -> Result<Self, DeviceError> {
        let mut records = Vec::new();
        let mut previous = 0;
        for line in capture.lines() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let mut fields = line.split_whitespace();
            let error = || crate::protocol("invalid fecap record", ProtocolErrorKind::Framing);
            let at: u64 = fields
                .next()
                .ok_or_else(error)?
                .parse()
                .map_err(|_| error())?;
            if at < previous || clock.now().0.checked_add(at).is_none() {
                return Err(error());
            }
            previous = at;
            let rx = match fields.next() {
                Some("rx") => true,
                Some("tx") => false,
                _ => return Err(error()),
            };
            let bytes = fields
                .map(|b| u8::from_str_radix(b, 16).map_err(|_| error()))
                .collect::<Result<Vec<_>, _>>()?;
            if rx {
                records.push((MonoTime(at), bytes));
            }
        }
        Ok(Self(FecTrainer::new(Arc::new(Replay(records)), clock)))
    }
}
impl TrainerDriver for ReplayTrainer {
    fn kind(&self) -> DriverKind {
        DriverKind::FeC
    }
    fn connect(
        &self,
        dev: &DeviceRef,
        rider: RiderConfig,
    ) -> BoxFuture<'static, Result<TrainerLink, DeviceError>> {
        self.0.connect(dev, rider)
    }
}
impl FecTransport for Replay {
    fn connect(
        &self,
        _: DeviceRef,
        clock: Arc<dyn Clock>,
    ) -> BoxFuture<'static, Result<(DeviceInfo, Notifications), DeviceError>> {
        let records = self.0.clone();
        Box::pin(async move {
            let start = clock.now().0;
            if records
                .last()
                .is_some_and(|(at, _)| start.checked_add(at.0).is_none())
            {
                return Err(crate::protocol(
                    "capture timestamp overflow",
                    ProtocolErrorKind::Framing,
                ));
            }
            let records = records.into_iter();
            let notifications: Notifications = Box::pin(stream::unfold(
                (records, clock),
                move |(mut records, clock)| async move {
                    let (elapsed, bytes) = records.next()?;
                    let at = MonoTime(start + elapsed.0);
                    clock.sleep_until(at).await;
                    Some((Ok((at, bytes)), (records, clock)))
                },
            ));
            Ok((DeviceInfo::default(), notifications))
        })
    }
    fn write(&self, _: [u8; 13]) -> BoxFuture<'static, Result<(), DeviceError>> {
        Box::pin(async { Ok(()) })
    }
    fn disconnect(&self) -> BoxFuture<'static, Result<(), DeviceError>> {
        Box::pin(async { Ok(()) })
    }
}
