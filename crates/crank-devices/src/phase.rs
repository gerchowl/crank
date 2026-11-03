//! The phases a driver passes through while it connects (ADR-0002 §5.1.6).
//!
//! A connect starts in `resolve` (finding the peripheral). A driver reports each later
//! phase as it enters it, with [`report`]; reaching `Ready` is the connect returning `Ok`.
//! Whoever awaits the connect observes the reports by running it inside [`observe`]: the
//! core times and logs them (`crank_core::links`), and the HIL diagnostic records `via`.
//!
//! The sink is task-local rather than a parameter of `connect`, so the driver traits and
//! their wrappers (reconnect supervisors, test fakes) stay as they are: a report reaches
//! the sink when it is made on the task that polls the observed connect, which every
//! driver's connect path does (scans and links run on their own tasks, but no phase is
//! reported from them). A report with no observer, such as a supervisor's reconnect on its
//! own task, goes nowhere.
use std::sync::Arc;

/// How a connect found its peripheral.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// The adapter already listed it (handle reuse, ADR-0002 §5.1.2).
    Lookup,
    /// A scan had to see it first.
    Scan,
}
impl Via {
    /// The log spelling: `lookup` or `scan`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lookup => "lookup",
            Self::Scan => "scan",
        }
    }
}

/// A phase of a connect, reported as it is entered. `resolve` is the phase every connect
/// starts in and is not reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectPhase {
    /// GATT connect and service discovery; the peripheral was resolved `via`.
    Gatt(Via),
    /// Subscribe, the DIS read and the identity check.
    Identity,
    /// Trainer only: page 55 written, the page-54 reply awaited.
    Handshake,
}
impl ConnectPhase {
    /// The log spelling: `gatt`, `identity` or `handshake`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gatt(_) => "gatt",
            Self::Identity => "identity",
            Self::Handshake => "handshake",
        }
    }
}

/// Where the reports of an observed connect go.
pub type PhaseSink = Arc<dyn Fn(ConnectPhase) + Send + Sync>;

tokio::task_local! {
    static SINK: PhaseSink;
}

/// Report that the connect being polled enters `phase`. Does nothing outside [`observe`].
pub fn report(phase: ConnectPhase) {
    let _ = SINK.try_with(|sink| sink(phase));
}

/// Await `connect` with `sink` receiving its [`report`]s.
pub async fn observe<F: Future>(sink: PhaseSink, connect: F) -> F::Output {
    SINK.scope(sink, connect).await
}
