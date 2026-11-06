//! Where devices come from: the host's BLE central, or the deterministic simulator
//! (`--sim`, ADR-0002 §3), so the UI and e2e run without hardware.
use crate::dto::DeviceRole;
use crank_ble::{BleCentral, CaptureDir, Pairing, RideGate};
use crank_devices::{
    Clock, DeviceError, DeviceInfo, DeviceRef, DiscoveredDevice, Discovery, DriverKind,
    HeartRateDriver, TrainerDriver,
    sim::{SimConfig, SimRig},
};
use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream, stream::BoxStream};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::OnceCell;

/// Which devices a [`crate::Core`] drives.
#[derive(Clone, Debug)]
pub enum DeviceMode {
    /// Real peripherals over the host's Bluetooth.
    Ble {
        /// Directory for a raw `.fecap` capture of each trainer connection (#411); none
        /// when `None`.
        capture_dir: Option<PathBuf>,
    },
    /// The simulator: one seeded rider with a trainer and a strap.
    Sim(SimConfig),
}

/// What the core needs from a BLE central. [`BleCentral`] is the production
/// implementation; tests inject a recording one.
pub(crate) trait Radio: Send + Sync + 'static {
    /// Post-filtered discovery.
    fn scan(&self) -> BoxStream<'static, DiscoveredDevice>;
    /// Trainer driver with reconnect.
    fn trainer(&self, clock: Arc<dyn Clock>, gate: RideGate) -> Arc<dyn TrainerDriver>;
    /// Strap driver with reconnect.
    fn heart(&self, clock: Arc<dyn Clock>, gate: RideGate) -> Arc<dyn HeartRateDriver>;
    /// A paired device the radio should be able to find again by name and verify.
    fn remember(&self, pairing: Pairing);
    /// A pairing was forgotten.
    fn forget(&self, platform_id: &str);
}
impl Radio for BleCentral {
    fn scan(&self) -> BoxStream<'static, DiscoveredDevice> {
        self.discovery().scan()
    }
    fn trainer(&self, clock: Arc<dyn Clock>, gate: RideGate) -> Arc<dyn TrainerDriver> {
        Arc::new(self.trainer_driver(clock, gate))
    }
    fn heart(&self, clock: Arc<dyn Clock>, gate: RideGate) -> Arc<dyn HeartRateDriver> {
        Arc::new(self.hr_driver(clock, gate))
    }
    fn remember(&self, pairing: Pairing) {
        BleCentral::remember(self, pairing);
    }
    fn forget(&self, platform_id: &str) {
        BleCentral::forget(self, platform_id);
    }
}
/// A radio that can be told where to write trainer captures.
pub(crate) trait Capturing: Radio + Sized {
    /// The same radio, writing a capture of each trainer connection into `dir`.
    fn capturing(self, dir: PathBuf) -> Self;
}
impl Capturing for BleCentral {
    fn capturing(self, dir: PathBuf) -> Self {
        self.with_capture_dir(CaptureDir::new(dir))
    }
}
/// The production radio factory's body: wait for `radio`, then point it at `capture_dir`
/// when one is configured. Generic so a test can run it with a radio that needs no adapter.
pub(crate) async fn acquire<R: Capturing>(
    radio: impl Future<Output = Result<R, DeviceError>>,
    capture_dir: Option<PathBuf>,
) -> Result<R, DeviceError> {
    let radio = radio.await?;
    Ok(match capture_dir {
        Some(dir) => radio.capturing(dir),
        None => radio,
    })
}
/// The production [`RadioFactory`], generic over the radio so a test can build it with one
/// that needs no adapter. `make` creates the radio on first use; the factory then points it
/// at the capture directory it is given, through [`acquire`].
pub(crate) fn radio_factory<R, F, Fut>(make: F) -> RadioFactory
where
    R: Capturing + 'static,
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<R, DeviceError>> + Send + 'static,
{
    Arc::new(move |capture_dir| {
        let radio = make();
        async move {
            let radio: Arc<dyn Radio> = Arc::new(acquire(radio, capture_dir).await?);
            Ok(radio)
        }
        .boxed()
    })
}
/// Acquires the radio on first use.
/// It is given the directory for trainer captures ([`DeviceMode::Ble`]), so a test can see
/// that the configured one arrives.
pub(crate) type RadioFactory = Arc<
    dyn Fn(Option<PathBuf>) -> BoxFuture<'static, Result<Arc<dyn Radio>, DeviceError>>
        + Send
        + Sync,
>;

enum Kind {
    Radio {
        cell: OnceCell<Arc<dyn Radio>>,
        factory: RadioFactory,
    },
    Sim(SimRig),
}
pub(crate) struct Backend {
    kind: Kind,
    /// Everything remembered of the paired devices: handed to the radio the moment it
    /// is created (a restarted process starts with an empty adapter memory) and kept
    /// current afterwards.
    pairings: Mutex<Vec<Pairing>>,
    /// Where the radio writes a capture of each trainer connection, if anywhere.
    capture_dir: Option<PathBuf>,
}

/// The ids the simulator's scan offers, and the role each can act in.
const SIM_TRAINER: &str = "trainer";
const SIM_STRAP: &str = "strap";

impl Backend {
    pub(crate) fn new(mode: DeviceMode, clock: Arc<dyn Clock>, pairings: Vec<Pairing>) -> Self {
        let mut capture_dir = None;
        let kind = match mode {
            DeviceMode::Ble { capture_dir: dir } => {
                capture_dir = dir;
                Kind::Radio {
                    cell: OnceCell::new(),
                    factory: radio_factory(BleCentral::new),
                }
            }
            DeviceMode::Sim(cfg) => Kind::Sim(SimRig::new(cfg, clock)),
        };
        Self {
            kind,
            pairings: Mutex::new(pairings),
            capture_dir,
        }
    }
    #[cfg(test)]
    pub(crate) fn with_radio(factory: RadioFactory, pairings: Vec<Pairing>) -> Self {
        Self {
            kind: Kind::Radio {
                cell: OnceCell::new(),
                factory,
            },
            pairings: Mutex::new(pairings),
            capture_dir: None,
        }
    }
    pub(crate) fn is_sim(&self) -> bool {
        matches!(self.kind, Kind::Sim(_))
    }
    /// The radio is acquired on first use (that is where macOS asks for Bluetooth) and
    /// is given every stored pairing before anyone uses it.
    async fn radio<'a>(
        cell: &'a OnceCell<Arc<dyn Radio>>,
        factory: &RadioFactory,
        pairings: &Mutex<Vec<Pairing>>,
        capture_dir: Option<&PathBuf>,
    ) -> Result<&'a Arc<dyn Radio>, DeviceError> {
        cell.get_or_try_init(|| async {
            let radio = factory(capture_dir.cloned()).await?;
            for p in pairings.lock().map(|p| p.clone()).unwrap_or_default() {
                radio.remember(p);
            }
            Ok(radio)
        })
        .await
    }
    /// A pairing was made or refreshed.
    pub(crate) fn remember(&self, pairing: Pairing) {
        if let Ok(mut all) = self.pairings.lock() {
            all.retain(|p| p.platform_id != pairing.platform_id);
            all.push(pairing.clone());
        }
        if let Kind::Radio { cell, .. } = &self.kind
            && let Some(radio) = cell.get()
        {
            radio.remember(pairing);
        }
    }
    /// A pairing was forgotten.
    pub(crate) fn forget(&self, platform_id: &str) {
        if let Ok(mut all) = self.pairings.lock() {
            all.retain(|p| p.platform_id != platform_id);
        }
        if let Kind::Radio { cell, .. } = &self.kind
            && let Some(radio) = cell.get()
        {
            radio.forget(platform_id);
        }
    }
    /// The simulator only has one device per role; a real radio cannot know the role of
    /// an id it has not scanned, so its connect is the check.
    pub(crate) fn accepts(&self, id: &str, role: DeviceRole) -> bool {
        match (&self.kind, role) {
            (Kind::Sim(_), DeviceRole::Trainer) => id == SIM_TRAINER,
            (Kind::Sim(_), DeviceRole::HeartRate) => id == SIM_STRAP,
            (Kind::Radio { .. }, _) => true,
        }
    }
    pub(crate) fn telemetry_source(&self) -> &'static str {
        if self.is_sim() { "sim" } else { "fec" }
    }
    pub(crate) async fn scan(&self) -> Result<BoxStream<'static, DiscoveredDevice>, DeviceError> {
        match &self.kind {
            Kind::Radio { cell, factory } => {
                Ok(
                    Self::radio(cell, factory, &self.pairings, self.capture_dir.as_ref())
                        .await?
                        .scan(),
                )
            }
            Kind::Sim(rig) => {
                let device = |id: &str, trainer, heart_rate| DiscoveredDevice {
                    device: DeviceRef(id.into()),
                    info: DeviceInfo {
                        name: Some(format!("crank sim {id}")),
                        ..DeviceInfo::default()
                    },
                    trainer,
                    heart_rate,
                };
                // a device that does not advertise is not listed (SimLatency::not_advertising)
                let seen = [
                    rig.advertises(true)
                        .then(|| device(SIM_TRAINER, Some(DriverKind::Sim), false)),
                    rig.advertises(false).then(|| device(SIM_STRAP, None, true)),
                ];
                Ok(stream::iter(seen.into_iter().flatten()).boxed())
            }
        }
    }
    pub(crate) async fn trainer(
        &self,
        clock: Arc<dyn Clock>,
        gate: RideGate,
    ) -> Result<Arc<dyn TrainerDriver>, DeviceError> {
        match &self.kind {
            Kind::Radio { cell, factory } => {
                Ok(
                    Self::radio(cell, factory, &self.pairings, self.capture_dir.as_ref())
                        .await?
                        .trainer(clock, gate),
                )
            }
            Kind::Sim(rig) => Ok(Arc::new(rig.trainer())),
        }
    }
    pub(crate) async fn heart(
        &self,
        clock: Arc<dyn Clock>,
        gate: RideGate,
    ) -> Result<Arc<dyn HeartRateDriver>, DeviceError> {
        match &self.kind {
            Kind::Radio { cell, factory } => {
                Ok(
                    Self::radio(cell, factory, &self.pairings, self.capture_dir.as_ref())
                        .await?
                        .heart(clock, gate),
                )
            }
            Kind::Sim(rig) => Ok(Arc::new(rig.heart())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crank_engine::VirtualClock;

    fn ble(capture_dir: Option<&str>) -> DeviceMode {
        DeviceMode::Ble {
            capture_dir: capture_dir.map(PathBuf::from),
        }
    }

    #[test]
    fn the_configured_capture_dir_is_the_backends() {
        let clock = || -> Arc<dyn Clock> { Arc::new(VirtualClock::new(1)) };
        let with = Backend::new(ble(Some("/cap")), clock(), Vec::new());
        assert_eq!(
            with.capture_dir.as_deref(),
            Some(std::path::Path::new("/cap"))
        );
        assert_eq!(
            Backend::new(ble(None), clock(), Vec::new()).capture_dir,
            None
        );
    }

    /// A radio that reports the capture directory it was given into `sink`.
    struct Recording(Arc<Mutex<Option<PathBuf>>>);
    impl Radio for Recording {
        fn scan(&self) -> BoxStream<'static, DiscoveredDevice> {
            stream::empty().boxed()
        }
        fn trainer(&self, _: Arc<dyn Clock>, _: RideGate) -> Arc<dyn TrainerDriver> {
            unreachable!("not used")
        }
        fn heart(&self, _: Arc<dyn Clock>, _: RideGate) -> Arc<dyn HeartRateDriver> {
            unreachable!("not used")
        }
        fn remember(&self, _: Pairing) {}
        fn forget(&self, _: &str) {}
    }
    impl Capturing for Recording {
        fn capturing(self, dir: PathBuf) -> Self {
            *self.0.lock().unwrap() = Some(dir);
            self
        }
    }

    #[tokio::test]
    async fn acquiring_the_radio_points_it_at_the_capture_dir_only_when_there_is_one() {
        let capturing = |dir| async move {
            let sink = Arc::new(Mutex::new(None));
            acquire(async { Ok(Recording(sink.clone())) }, dir)
                .await
                .unwrap();
            sink.lock().unwrap().clone()
        };
        assert_eq!(
            capturing(Some("/cap".into())).await,
            Some(PathBuf::from("/cap"))
        );
        assert_eq!(capturing(None).await, None);
    }

    #[tokio::test]
    async fn the_radio_is_acquired_with_the_backends_capture_dir() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let factory: RadioFactory = {
            let seen = seen.clone();
            Arc::new(move |dir| {
                seen.lock().unwrap().push(dir);
                async { Err(DeviceError::AdapterUnavailable) }.boxed()
            })
        };
        let mut backend = Backend::with_radio(factory, Vec::new());
        backend.capture_dir = Some("/cap".into());
        assert!(backend.scan().await.is_err());
        assert_eq!(*seen.lock().unwrap(), [Some(PathBuf::from("/cap"))]);
    }

    #[tokio::test]
    async fn the_production_factory_hands_its_capture_dir_to_the_radio() {
        let sink = Arc::new(Mutex::new(None));
        let factory = {
            let sink = sink.clone();
            radio_factory(move || {
                let sink = sink.clone();
                async move { Ok(Recording(sink)) }
            })
        };
        assert!(factory(Some("/cap".into())).await.is_ok());
        assert_eq!(*sink.lock().unwrap(), Some(PathBuf::from("/cap")));
    }
}
