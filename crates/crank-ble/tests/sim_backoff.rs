//! The simulator's reconnect schedule must equal the real supervisor's
//! (ADR-0002 §6), so a sim dropout is observably the same as a real one.

use crank_ble::backoff;
use crank_devices::sim::reconnect_backoff;

#[test]
fn sim_backoff_equals_the_real_supervisor_schedule() {
    for attempt in 0..=12 {
        assert_eq!(
            reconnect_backoff(attempt),
            backoff(attempt),
            "attempt {attempt}"
        );
    }
}
