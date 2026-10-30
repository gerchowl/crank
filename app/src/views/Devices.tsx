// Devices (DESIGN §5.9, #92 MVP subset): scan, pair and forget the trainer and the strap. It renders what the core
// reports (`devices_scan`, `device_pair`, `device_forget`, `device_links`) and branches on `ApiError.code`, never on
// `message` (ADR-0001 §4.4). Out of this subset, each with its issue: live W/rpm/bpm while pairing (#267),
// "Release trainer" (the core has no release command, #314) and a distinct "another app holds the NEO" signal (#314).
import { useCallback, useEffect, useRef, useState } from "react";
import { commands } from "../bindings";
import type {
  ApiError,
  DeviceLinkInfo,
  DeviceRole,
  ScannedDevice,
} from "../bindings";
import { useCommand } from "../stream";

const ROLE_LABEL: Record<DeviceRole, string> = {
  trainer: "Trainer",
  heart_rate: "Heart-rate strap",
};
const ROLES: readonly DeviceRole[] = ["trainer", "heart_rate"];

/** What the view tells the rider about a failed command. */
interface Notice {
  /** Stable across renders, for the list key: two notices may share a title. */
  id: number;
  title: string;
  body: string;
  /** The notice offers "Try again" (it re-runs the scan). */
  retry?: boolean;
}

/** The macOS path to the Bluetooth grant (ADR-0002 §8): the one manual step. */
const PERMISSION_STEPS =
  "Open System Settings → Privacy & Security → Bluetooth → crank, allow it, then try again.";

let noticeSeq = 0;

/** `not_found` has no case: the only command that can answer it is forget, and the view treats that as success. */
function describe(
  error: ApiError,
  during: "load" | "scan" | "pair" | "forget",
  what: string,
): Notice {
  return { id: ++noticeSeq, ...describeError(error, during, what) };
}

function describeError(
  error: ApiError,
  during: "load" | "scan" | "pair" | "forget",
  what: string,
): Omit<Notice, "id"> {
  switch (error.code) {
    case "permission_denied":
      return {
        title: "Bluetooth access is denied",
        body: PERMISSION_STEPS,
        retry: true,
      };
    case "device_unavailable":
      // A scan only fails this way when the adapter is missing or off. A failed connect says nothing more than
      // that: the core has no distinct signal for a trainer held by another app (#314), so the hint is hedged.
      return during === "scan"
        ? {
            title: "Bluetooth off",
            body: "Turn Bluetooth on in Control Center or System Settings → Bluetooth, then try again.",
            retry: true,
          }
        : {
            title: `Could not connect to ${what}`,
            body: "It may be out of range or asleep, or another app may be connected to it: a trainer takes one connection at a time. Close other apps that use it, then try again.",
          };
    case "conflict":
      return {
        title: "A ride is loaded",
        body: "End the ride before changing devices: it holds the Bluetooth connection.",
      };
    case "storage":
      // The pairings live in the local database; the core reads it first on scan and forget, and writes it last on pair.
      // A pair can fail after the row was written (core.rs:1064-1077), so the wording does not say what was stored:
      // the view re-reads the pairings, and the list shows the truth.
      return during === "pair"
        ? {
            title: `Could not save the pairing of ${what}`,
            body: `It connected, but crank had a problem with the pairing store. Check the list below for what is paired. ${error.message}`,
          }
        : {
            title:
              during === "load"
                ? `Could not load ${what}`
                : `Could not ${during} ${what}: the pairing store failed`,
            body: error.message,
            retry: during === "scan",
          };
    default:
      return {
        title: `Could not ${during} ${what}`,
        body: error.message,
      };
  }
}

function Banner({ notice, onRetry }: { notice: Notice; onRetry: () => void }) {
  return (
    <div className="device-notice" role="alert">
      <strong>{notice.title}</strong>
      <p>{notice.body}</p>
      {notice.retry && (
        <button type="button" className="primary" onClick={onRetry}>
          Try again
        </button>
      )}
    </div>
  );
}

function pairedLine(d: NonNullable<DeviceLinkInfo["device"]>): string {
  return [d.name ?? "Unnamed device", d.model, d.serial && `S/N ${d.serial}`]
    .filter(Boolean)
    .join(" · ");
}

export function Devices() {
  const [found, setFound] = useState<ScannedDevice[] | null>(null);
  // Every failure of the command in flight, in order: a forget that fails and a refresh that fails are both shown.
  const [notices, setNotices] = useState<Notice[]>([]);
  /** The pairing in flight: device id and role, so a dual-role device shows "Pairing…" on one button only. */
  const [pairing, setPairing] = useState<string | null>(null);
  const getLinks = useCommand(commands.deviceLinks);
  const scan = useCommand(commands.devicesScan);
  const pair = useCommand(commands.devicePair);
  const forget = useCommand(commands.deviceForget);
  // `useCommand` drops its data on a failure: keep the last good list here, so a failed re-read leaves the rows.
  const [links, setLinks] = useState<DeviceLinkInfo[] | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);

  // Re-read the pairings; a failure surfaces as a notice and the last good list stays.
  const load = getLinks.run;
  const apply = useCallback((r: Awaited<ReturnType<typeof load>>) => {
    if (r.status === "ok") {
      setLinks(r.data);
      setLoadFailed(false);
    } else {
      setLoadFailed(true);
      const n = describe(r.error, "load", "the paired devices");
      setNotices((all) => [...all, n]);
    }
  }, []);
  const refresh = useCallback(async () => apply(await load()), [load, apply]);
  useEffect(() => {
    void load().then(apply);
  }, [load, apply]);

  // One command at a time: the core has one central. A ref, not the pending flags, because a second click can
  // arrive before the render that disables the buttons.
  const running = useRef(false);
  const exclusive = async (op: () => Promise<void>) => {
    if (running.current) return;
    running.current = true;
    try {
      await op();
    } finally {
      running.current = false;
    }
  };

  const runScan = () =>
    exclusive(async () => {
      setNotices([]);
      const r = await scan.run(null);
      if (r.status === "ok") setFound(r.data);
      else {
        setFound(null);
        setNotices([describe(r.error, "scan", "for devices")]);
      }
    });
  const runPair = (d: ScannedDevice, role: DeviceRole) =>
    exclusive(async () => {
      setNotices([]);
      setPairing(`${d.id}:${role}`);
      const r = await pair.run(d.id, role);
      setPairing(null);
      if (r.status === "ok") await refresh();
      else {
        setNotices([
          describe(r.error, "pair", d.name ?? ROLE_LABEL[role].toLowerCase()),
        ]);
        // `storage` after the connect may have written the pairing already: re-read, so the list is the truth.
        if (r.error.code === "storage") await refresh();
      }
    });
  const runForget = (role: DeviceRole) =>
    exclusive(async () => {
      setNotices([]);
      const r = await forget.run(role);
      // `not_found` is success: the role has no pairing, which is what the rider asked for. Both outcomes are settled
      // facts, so the row is cleared here and the refresh only reconciles: if it fails, the row must not go on
      // showing a pairing that is gone (a second Forget would answer `not_found` for nothing).
      const gone = r.status === "ok" || r.error.code === "not_found";
      if (gone)
        setLinks(
          (all) =>
            all?.map((l) => (l.role === role ? { ...l, device: null } : l)) ??
            null,
        );
      else
        setNotices([
          describe(r.error, "forget", ROLE_LABEL[role].toLowerCase()),
        ]);
      // The refresh adds its own notice after the forget's: neither replaces the other.
      await refresh();
    });

  const busy = scan.pending || pair.pending || forget.pending;
  const pairedIds = new Set(
    (links ?? []).flatMap((l) => (l.device ? [l.device.id] : [])),
  );
  return (
    <div className="page devices">
      <div className="page-head">
        <h1>Devices</h1>
        <p className="muted">
          A trainer and a heart-rate strap connect one at a time: close other
          apps that use them.
        </p>
      </div>
      {notices.map((n) => (
        <Banner key={n.id} notice={n} onRetry={() => void runScan()} />
      ))}

      <section aria-labelledby="paired-h">
        <h2 id="paired-h">Paired</h2>
        <ul className="device-list">
          {ROLES.map((role) => {
            const device = links?.find((l) => l.role === role)?.device ?? null;
            return (
              <li key={role} className="device-row">
                <div>
                  <strong>{ROLE_LABEL[role]}</strong>
                  <div className="muted">
                    {links === null
                      ? loadFailed
                        ? "Unknown"
                        : "Loading…"
                      : device
                        ? pairedLine(device)
                        : "Not paired"}
                  </div>
                </div>
                {device && (
                  <button
                    type="button"
                    disabled={busy}
                    aria-label={`Forget ${ROLE_LABEL[role].toLowerCase()}`}
                    onClick={() => void runForget(role)}
                  >
                    Forget
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      </section>

      <section aria-labelledby="scan-h">
        <h2 id="scan-h">Nearby</h2>
        <button
          type="button"
          className="primary"
          disabled={busy}
          onClick={() => void runScan()}
        >
          {scan.pending ? "Scanning…" : found ? "Scan again" : "Scan"}
        </button>
        <div aria-live="polite">
          {found?.length === 0 && (
            <p className="muted">
              No trainer or strap found. Wake it up (pedal, or put the strap on)
              and scan again.
            </p>
          )}
        </div>
        {found && found.length > 0 && (
          <ul className="device-list" aria-label="Nearby devices">
            {found.map((d) => {
              const name = d.name ?? "Unnamed device";
              const roles = ROLES.filter((r) =>
                r === "trainer" ? d.trainer : d.heartRate,
              );
              return (
                <li key={d.id} className="device-row">
                  <div>
                    <strong>{name}</strong>
                    <div className="muted">
                      {roles.map((r) => ROLE_LABEL[r]).join(" · ")}
                      {pairedIds.has(d.id) && " · Paired"}
                    </div>
                  </div>
                  <div className="device-actions">
                    {roles.map((role) => (
                      <button
                        key={role}
                        type="button"
                        disabled={busy}
                        aria-label={`Pair ${name} as ${ROLE_LABEL[role].toLowerCase()}`}
                        onClick={() => void runPair(d, role)}
                      >
                        {pairing === `${d.id}:${role}` && pair.pending
                          ? "Pairing…"
                          : `Pair as ${role === "trainer" ? "trainer" : "strap"}`}
                      </button>
                    ))}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </section>
    </div>
  );
}
