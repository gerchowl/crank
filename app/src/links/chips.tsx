// The link chips (ADR-0002 §5.1.4 "Failure UX", #681, part c of #471): each paired device's link as the keeper and the
// ride report it, read from its one source, the snapshot channel's `devices` (ADR-0001 §4.3.6). Quiet chips, never a
// modal error or a toast: "Trainer ↻ connecting…", "Trainer ✓ ready", "Trainer ⚠ not found" with the hedged hint "is
// another app using it?" (until #314 tells a held trainer apart), the same for HR, whose hint is "keeps looking" (the
// keeper retries it on its next advertisement while a warm view is shown, §5.1.2). A word and a glyph, never colour
// alone (DESIGN §18.7 rule 11).
//
// Retry (`links_intent(Retry)`, §5.1.2) is offered where the spec offers it: on a warm view (Today, Workouts) while a
// device is not found. It goes through the window's intent reporter (intent.ts `IntentReporter::retry`), so it keeps
// the order of the window's other intents. Settings → Devices shows the chips without Retry: it releases the links for
// pairing (`Release(Devices)`), and a Retry there would connect them again. The Ready sheet has rows of its own
// (ReadySheet.tsx `DEVICE_WORD`) and no Retry: a loaded ride holds its links and the keeper ignores Retry then
// (keeper/mod.rs `Keeper::loaded_intent` (~490)); a strap that joins a loaded ride is #450.
import { RotateCw } from "lucide-react";
import { createContext, useContext } from "react";
import type { DeviceView, DevicesView } from "../bindings";
import "./links.css";

/** A chip's state: the keeper's phase, folded to what the rider tells apart, or the ride's link while it holds it. */
export type ChipState =
  | "unpaired"
  | "cold"
  | "searching"
  | "connecting"
  | "ready"
  | "not-found"
  | "connected"
  | "lost";

export type ChipRole = "trainer" | "strap";

export interface ChipWord {
  state: ChipState;
  glyph: string;
  word: string;
  /** The line under a device that is not found, else `null`. */
  hint: string | null;
}

const WORD: Record<ChipState, [glyph: string, word: string]> = {
  unpaired: ["–", "not paired"],
  cold: ["–", "not connected"],
  searching: ["↻", "searching…"],
  connecting: ["↻", "connecting…"],
  ready: ["✓", "ready"],
  "not-found": ["⚠", "not found"],
  connected: ["✓", "connected"],
  lost: ["⚠", "lost"],
};

const HINT: Record<ChipRole, string> = {
  trainer: "is another app using it?",
  strap: "keeps looking",
};

/** The chip of one device: a pure function of its `DeviceView`, tested on its own. */
export function chipWord(role: ChipRole, d: DeviceView): ChipWord {
  const state = chipState(d);
  const [glyph, word] = WORD[state];
  return {
    state,
    glyph,
    word,
    hint: state === "not-found" ? HINT[role] : null,
  };
}

function chipState(d: DeviceView): ChipState {
  switch (d.phase) {
    case null:
      return "unpaired";
    case "cold":
      return "cold";
    case "searching":
      return "searching";
    // The handshake is the trainer's last step of connecting (page 55, then the page-54 reply).
    case "connecting":
    case "handshake":
      return "connecting";
    case "warm":
      return "ready";
    case "not_found":
      return "not-found";
    // The ride holds it: its link (`link` is set exactly when `phase` is `riding`; until the session is installed the
    // core reports the state it was handed over in, `ready`).
    case "riding":
      return d.link === "lost"
        ? "lost"
        : d.link === "connecting"
          ? "connecting"
          : "connected";
  }
}

/** Retry is offered for a device that is not found. */
export const offersRetry = (devices: DevicesView): boolean =>
  devices.trainer.phase === "not_found" || devices.strap.phase === "not_found";

/** The window's Retry (`links_intent(Retry)` through its intent reporter); `null` outside `LinkIntents`. */
export const RetryContext = createContext<(() => void) | null>(null);

/** One device's chip; `label` names the device ("Trainer", "HR") where the row does not. */
export function LinkChip({
  of: role,
  device,
  label,
}: {
  of: ChipRole;
  device: DeviceView;
  label?: string;
}) {
  const c = chipWord(role, device);
  return (
    <span
      className="link-chip"
      data-link-chip={role}
      data-state={c.state}
      data-phase={device.phase ?? "none"}
    >
      {/* A space between the name and the glyph for a reader of the text; the flex gap draws it. */}
      {label !== undefined && <span className="link-chip-name">{label}</span>}
      {label !== undefined && " "}
      <span data-state-label>{c.glyph}</span> {c.word}
    </span>
  );
}

/** The Retry button of a warm view: shown while a device is not found. */
export function RetryLinks({ devices }: { devices: DevicesView }) {
  const retry = useContext(RetryContext);
  if (retry === null || !offersRetry(devices)) return null;
  return (
    <button type="button" className="link-retry" onClick={retry}>
      <RotateCw size={16} strokeWidth={1.5} aria-hidden />
      Retry
    </button>
  );
}

/** The hints of the devices that are not found, one line each ("Trainer: is another app using it?"). */
export function LinkHints({ devices }: { devices: DevicesView }) {
  const lines = (
    [
      ["trainer", "Trainer", devices.trainer],
      ["strap", "HR", devices.strap],
    ] as const
  ).flatMap(([role, label, d]) => {
    const hint = chipWord(role, d).hint;
    return hint === null ? [] : [`${label}: ${hint}`];
  });
  return lines.map((l) => (
    <p key={l} className="link-hint">
      {l}
    </p>
  ));
}

/** Both devices' chips, their hints and Retry, as one group (Workouts, beside Ride this). */
export function LinkChips({ devices }: { devices: DevicesView }) {
  return (
    <div className="link-chips" role="group" aria-label="Device links">
      <span className="link-chip-row">
        <LinkChip of="trainer" device={devices.trainer} label="Trainer" />
        <LinkChip of="strap" device={devices.strap} label="HR" />
        <RetryLinks devices={devices} />
      </span>
      <LinkHints devices={devices} />
    </div>
  );
}
