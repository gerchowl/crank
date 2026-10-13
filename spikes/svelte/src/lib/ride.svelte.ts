// Ride-level state shared by the top bar, widgets and keyboard handler.
// `status` is replaced on every backend event (≥ 2 Hz heartbeat), but consumers read the
// fine-grained $derived fields below: a derived only notifies when its *value* changes, so a
// widget reading `ride.blockIndex` re-renders once per block, not twice per second.
import type { CoachNudge, CompiledWorkout, FlatBlock, Phase, RideStatus, Workout } from "@neo/shared";
import { backend } from "./backend";

const initial: RideStatus = {
  state: "idle",
  t: 0,
  speed: 10,
  mode: "erg",
  trainer: "connecting",
  hrSensor: "connecting",
  blockIndex: 0,
  phase: "warmup",
  manualOffsetPct: 0,
};

class RideStore {
  status = $state.raw<RideStatus>(initial);
  current = $state.raw<{ workout: Workout; compiled: CompiledWorkout } | null>(backend.ride.current());
  /** All nudges of this ride, by callId (latest status wins). */
  nudges = $state.raw<CoachNudge[]>([]);

  state = $derived(this.status.state);
  phase = $derived<Phase>(this.status.phase);
  blockIndex = $derived(this.status.blockIndex);
  speed = $derived(this.status.speed);
  mode = $derived(this.status.mode);
  trainer = $derived(this.status.trainer);
  hrSensor = $derived(this.status.hrSensor);
  manualOffsetPct = $derived(this.status.manualOffsetPct);
  /** Whole seconds only, so the clock text updates once per ride-second at most. */
  tSec = $derived(Math.floor(this.status.t));

  blocks = $derived<FlatBlock[]>(this.current?.compiled.blocks ?? []);
  block = $derived<FlatBlock | undefined>(this.blocks[this.blockIndex]);
  nextBlock = $derived<FlatBlock | undefined>(this.blocks[this.blockIndex + 1]);
  totalS = $derived(this.current?.compiled.totalS ?? 0);
  /** Newest pending nudge (what the coach card and Y/N act on). */
  pending = $derived(this.nudges.findLast((n) => n.status === "pending"));
  latestNudge = $derived(this.nudges.at(-1));

  constructor() {
    backend.onStatus((s) => {
      const prev = this.status;
      this.status = s;
      // ride.current() is a deep copy: refresh only on discrete changes (polling budget).
      if (prev.blockIndex !== s.blockIndex || prev.state !== s.state || (s.t < prev.t && s.t < 1)) {
        this.current = backend.ride.current();
      }
      if (s.state === "idle" && prev.state !== "idle") this.nudges = [];
    });
    backend.onNudge((n) => {
      const i = this.nudges.findIndex((x) => x.callId === n.callId);
      this.nudges = i < 0 ? [...this.nudges, n] : this.nudges.with(i, n);
      // Coach changes rewrite compiled blocks (aiChanged): refresh the profile.
      if (n.status === "auto_applied" || n.status === "accepted") this.current = backend.ride.current();
    });
  }

  load(w: Workout): void {
    backend.ride.stop();
    backend.ride.load(w);
    this.nudges = [];
    this.current = backend.ride.current();
  }

  togglePause(): void {
    if (this.state === "riding") backend.ride.pause();
    else if (this.state === "paused") backend.ride.resume();
    else if (this.state === "idle") backend.ride.start();
    else {
      // finished: restart the same workout
      const cur = this.current;
      if (cur) this.load(cur.workout);
      backend.ride.start();
    }
  }

  respond(accept: boolean): void {
    const p = this.pending;
    if (p) backend.coach.respond(p.callId, accept);
  }
}

export const ride = new RideStore();
