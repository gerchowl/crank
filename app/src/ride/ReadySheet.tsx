// The Ready sheet (DESIGN §5.4, §18 direction A; #526): the ride screen before Start. It keeps the ride anatomy (§18.2),
// so no region moves when the ride starts (rule 10): the status bar names the workout and its duration, the metric column
// shows the first block's countdown and the next block, the block timeline shows the plan, and the key row holds Start ⏎
// and End Esc. The traces column, empty before the first sample, holds the sheet: its state in the banner band (one
// banner in the band's vocabulary, banners.tsx, T3+ lines of at most 26 characters) and, below it, the body at T3: one
// row per device, the mode and how the ride starts, and the reason of a refused command.
//
// Every word comes from today's data: the status (`null` until the stream has answered), the links of the loaded ride
// (`useLinks`: the stream's link frames and `device_links`), the `ride_load` answer (`RideLoaded.hrLinked`; a loaded ride
// had its trainer linked, core.rs `Core::ride_load` (~1566) fails the load without one) and the compiled plan. The device
// rows are the connect-progress region of ADR-0002 §5.1.4, in their final geometry: one fixed row per device, a T3 line
// and one hint line. What #471 plugs in once #470's `LoadProgress` exists is listed in `DEVICE_WORD`'s comment.
import { Bluetooth, Heart } from "lucide-react";
import type { ReactNode } from "react";
import type { LinkStatus, WorkoutBlock } from "../bindings";
import { Num } from "../Num";
import type { Links } from "../stream/state";
import type { Banner } from "./banners";
import { PHASE_NAME } from "./Timeline";

/** The sheet's states (#526 acceptance), most urgent first. */
export type ReadyState =
  | "error"
  | "loading"
  | "trainer-missing"
  | "connecting"
  | "strap-missing"
  | "ready";

/** The ride commands of the screen; before Start only Start and End reach the core. */
export type RideCommand = "start" | "pause" | "resume" | "skip" | "end";

/** One device row's phase, from the links of the loaded ride and the load's answer. */
export type DevicePhase = "checking" | "connected" | "connecting" | "lost";

export interface ReadyInput {
  /** The status has arrived: until then nothing on the sheet is the core's yet. */
  status: boolean;
  links: Links;
  /** `RideLoaded.hrLinked`: the strap linked at the load. */
  hrLinked: boolean;
  /** The command whose refusal is shown (until the next command), `null` without one. */
  refused: RideCommand | null;
}

export interface ReadyView {
  state: ReadyState;
  trainer: DevicePhase;
  hr: DevicePhase;
  /** The refused command, for the error state's title. */
  refused: RideCommand | null;
  /** Start may be pressed: the trainer is not known to be anything but connected (§5.4: "Start is enabled when the trainer is connected"). */
  canStart: boolean;
  /**
   * The key row's state line beside Start: what Start does or waits for, from the devices' state (a refusal does not
   * change it). The band says the state and the rows say each device, so the line never repeats either of them.
   */
  line: string;
}

/**
 * The link of a device as the sheet says it. No link reported yet means the load's own answer: the trainer linked (the
 * load fails without it), the strap as `hrLinked` says; before the status has arrived, nothing is known.
 */
function phase(
  status: boolean,
  link: LinkStatus | undefined,
  linkedAtLoad: boolean,
): DevicePhase {
  if (link === "ready") return "connected";
  if (link === "connecting") return "connecting";
  if (link === "lost") return "lost";
  if (!status) return "checking";
  return linkedAtLoad ? "connected" : "lost";
}

/** The sheet's state and its device rows: a pure function of today's data, tested on its own. */
export function readyView(i: ReadyInput): ReadyView {
  const trainer = phase(i.status, i.links.trainer, true);
  const hr = phase(i.status, i.links.hr, i.hrLinked);
  const devices: ReadyState = !i.status
    ? "loading"
    : trainer === "lost"
      ? "trainer-missing"
      : trainer === "connecting"
        ? "connecting"
        : hr !== "connected"
          ? "strap-missing"
          : "ready";
  return {
    state: i.refused !== null ? "error" : devices,
    trainer,
    hr,
    refused: i.refused,
    canStart: trainer !== "lost" && trainer !== "connecting",
    line: LINE[devices],
  };
}

/** The key row's line per devices' state (`ReadyView.line`); `error` never comes from the devices. */
const LINE: Record<ReadyState, string> = {
  error: "",
  loading: "Start is enabled once the ride has loaded",
  connecting: "Start is enabled once the NEO is connected",
  "trainer-missing": "Start needs the NEO connected",
  "strap-missing": "Press Start; the trainer waits for you to pedal",
  ready: "Press Start; the trainer waits for you to pedal",
};

/** The error state's title, by the command the core refused: End on an unstarted ride unloads it (§5.4). */
const REFUSED_TITLE: Record<RideCommand, string> = {
  start: "Could not start",
  end: "Could not unload the ride",
  pause: "Not possible right now",
  resume: "Not possible right now",
  skip: "Not possible right now",
};

const text = (t: string) => ({ text: t });

/**
 * The sheet's banner for the band (§6.1: a bold line and at most two short lines of ≤ 26 characters). Only the
 * trainer-missing state asks the rider to act; a missing strap states its consequence. What Start waits for is the key
 * row's line (`ReadyView.line`), never repeated here.
 */
export function readyBanner(view: ReadyView): Banner {
  switch (view.state) {
    case "error":
      return {
        id: "ready-error",
        title: text(REFUSED_TITLE[view.refused ?? "start"]),
        actions: [],
        status: [],
      };
    case "loading":
      return {
        id: "ready-loading",
        title: text("Loading the ride…"),
        actions: [],
        status: [],
      };
    case "trainer-missing":
      return {
        id: "ready-trainer-missing",
        title: text("NEO not found"),
        actions: [text("close apps using the NEO")],
        status: [],
        compact: "NEO not found: close apps",
      };
    case "connecting":
      return {
        id: "ready-connecting",
        title: text("Connecting the NEO…"),
        actions: [],
        status: [],
      };
    case "strap-missing":
      return {
        id: "ready-strap-missing",
        title: text("HR not found"),
        actions: [],
        status: [text("this ride runs without HR")],
      };
    case "ready":
      return {
        id: "ready",
        title: text("Ready to ride"),
        actions: [],
        status: [],
      };
  }
}

/**
 * A device row's word and glyph (rule 11: never colour alone). The words for `LoadProgress`'s phases, which #471 maps
 * onto these rows once #470 reports them in `ride_status` (ADR-0002 §5.1.4): `searching` "searching…" ↻,
 * `connecting`/`handshake` as `connecting` here, `warm`/`riding` as `connected`, `not_found` "not found" ⚠ with the
 * hint line "keeps looking" (reserved and empty today: the band and the key row say what a missing device means). #471 also adds Retry (the `Retry` intent) to a row that is not found, and "Retry now" before
 * "Start without HR" on the HR row (DESIGN §5.4); no command for either exists today.
 */
const DEVICE_WORD: Record<DevicePhase, { glyph: string; word: string }> = {
  checking: { glyph: "↻", word: "checking…" },
  connected: { glyph: "✓", word: "connected" },
  connecting: { glyph: "↻", word: "connecting…" },
  lost: { glyph: "⚠", word: "not found" },
};

const icon = { size: 16, strokeWidth: 1.5, "aria-hidden": true } as const;

function Device({
  device,
  name,
  Icon,
  at,
}: {
  device: "trainer" | "hr";
  name: string;
  Icon: typeof Heart;
  at: DevicePhase;
}) {
  const w = DEVICE_WORD[at];
  return (
    <li className="ready-device" data-device={device} data-state={at}>
      <span className="ready-device-line">
        <Icon {...icon} />
        <span className="ready-device-name">{name}</span>
        <span className="ready-device-word" data-state-label>
          {w.glyph} {w.word}
        </span>
      </span>
      <span className="ready-hint" />
    </li>
  );
}

/** "starts with 3:00 warm-up": the first block, its duration in the number face. */
function Starts({
  first,
  clock,
}: {
  first: WorkoutBlock | undefined;
  clock: (s: number | null) => string;
}) {
  if (!first) return <>loading the plan…</>;
  const name = PHASE_NAME[first.phase].toLowerCase();
  if (first.durS === null) return <>starts with {name}</>;
  return (
    <>
      starts with <Num>{clock(first.durS)}</Num> {name}
    </>
  );
}

/** The sheet's body in the traces column; `refusal` is the refused command's alert, drawn in its reserved row. */
export function ReadyBody({
  view,
  free,
  first,
  clock,
  refusal,
}: {
  view: ReadyView;
  free: boolean;
  first: WorkoutBlock | undefined;
  clock: (s: number | null) => string;
  refusal: ReactNode;
}) {
  return (
    <section
      className="ready-sheet"
      data-ready={view.state}
      aria-label="Ready check"
    >
      <ul className="ready-devices" aria-label="Devices before the start">
        <Device
          device="trainer"
          name="NEO"
          Icon={Bluetooth}
          at={view.trainer}
        />
        <Device device="hr" name="HR" Icon={Heart} at={view.hr} />
      </ul>
      <p className="ready-plan" data-testid="ready-plan">
        {free ? (
          "Free ride · no targets"
        ) : (
          <>
            ERG · <Starts first={first} clock={clock} />
          </>
        )}
      </p>
      <div className="ready-refusal">{refusal}</div>
    </section>
  );
}
