// What the window remembers of the rides it has shown, beyond the snapshot channel (ADR-0001 §4.3): only that it has
// left a ride's screen. The ride itself, its series and how it ended are the channel's (`useUi().ride`, `.series`):
// they survive a reload and are replaced by the next load (#598). A ride screen leaves a ride once, however many
// signals say it is over (the `finished` status, `ride.ended`, the answer to End), and a reload forgets it, so a
// reloaded ride screen of an ended ride leaves it again.
const left = new Set<string>();

/** Run `go` the first time only for `rideId`. */
export function leaveOnce(rideId: string, go: () => void): void {
  if (left.has(rideId)) return;
  left.add(rideId);
  go();
}

/** Test-only: forget every ride left. */
export function resetLeft(): void {
  left.clear();
}
