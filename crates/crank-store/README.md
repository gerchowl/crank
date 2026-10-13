# crank-store

Implements ADR-0001 §5–6 and ADR-0002 §5. `Store::open_owner(data_dir)`
acquires the advisory `ride.lock` before starting the sole writer thread. That
thread applies forward-only migrations, enables WAL, `synchronous=FULL` and
foreign keys, then closes unfinished rides as aborted. The guard stays with the
writer until shutdown. `Store::open_read_only` neither migrates nor recovers.
Both modes use a pool of two read-only connections; callers never issue SQL.

Start a ride with a fixed metric set. Supply each sequential **session second**
with all metrics, explicitly using `None` for missing samples and workout time
while paused. Every five accepted seconds, all open chunks are upserted in one
transaction. Every sixty seconds, the committed buffers are cleared. Finishing
flushes the final partial chunks and closes the ride in the same transaction.
Rejected inputs or failed transactions leave the buffered record unchanged.
Dropping a live owner leaves the ride unfinished for recovery at the next owner
open; up to four buffered seconds are not yet durable.

Chunk values are little-endian f32; NaN represents missing. The estimated mask
uses the least significant bit for the chunk's first session second, with unused
high bits zero. Reads reject malformed payloads and infinite values. Timestamps
are UTC Unix seconds; an aborted ride's end timestamp is the recovery time.

Each JSON column has its own sealed newtype and requires an object containing an
integer `v`. The two workout columns also validate through the actual domain
`Workout` type from #76 and expose that typed value; #76 landed before v1's
initial release. Other owner types will replace their envelopes through forward
migrations, tracked in
[#156](https://github.com/gerchowl/crank/issues/156). Metric distribution types on
dev are not ride-summary/lap-stat schemas, so those columns also use envelopes.
The initial migration includes the scalar `athlete_profile` table required by
ADR-0001 §6; the M2 training-profile/wish contract is separate and out of scope.

The acceptance suite includes all frozen catalog metrics plus `workout_s` for a
two-hour bit-exact round trip, a real child killed with SIGKILL after an explicit
pipe handshake, concurrent lock denial and read-only access, injected transaction
failure, and invalid persisted JSON for every column. Fixture maintenance is
documented in [tests/fixtures/README.md](tests/fixtures/README.md).
