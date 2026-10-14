//! ERG policy (ADR-0002 §4.4.1, §7; DESIGN §15.2): what the engine commands and whether
//! the trainer follows it.
//!
//! * [`following`]: the trainer-following machine, a port of the executable model
//!   `docs/adr/models/trainer_following.py`.
//! * [`policy`]: [`ErgPolicy`], the [`TargetPolicy`](crate::control::TargetPolicy) the
//!   engine runs: block target and ramps, the soft start after a loss or after leaving
//!   free/SIM or a release (§7.5), the low-cadence release (§7.4) and the health-cap clamp.
//! * [`compliance`]: the settle rules and the per-second tally (§7.8).
pub mod compliance;
pub mod following;
pub mod policy;

pub use compliance::Compliance;
pub use following::{Component, ComponentState, Machine, TrainerFollowing};
pub use policy::ErgPolicy;
