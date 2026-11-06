// The state banners of the ride screen (DESIGN §5.2 States, §6.1, §6.5, §18.2, §18.3): the band at the top of the traces
// column holds three T3+ lines. Every state that holds gets at least its bold line, in BANNER_ORDER; the lines left go to
// the most urgent first, each banner's lines being listed most important first (the action before the detail). Lines are
// at most 26 characters (§6.1, re-derived for A in #467), so none is clipped at 1024. Every word comes from the status,
// the links and the stream (`lossView`), never from a timer of this screen.
import type { ReactNode } from "react";
import type { StreamCommanded } from "../bindings";

/** Lines in the band (§18.2: one bold line and two short lines, 3 × 48 px). */
export const BAND_LINES = 3;
/** Characters per banner line (§6.1). */
export const LINE_MAX = 26;

/**
 * Most urgent first (§5.2, §6.5). The engine not answering comes first: every other value on screen may be stale.
 * A lost trainer pauses the ride and needs the rider's action; the reconnect rows follow it. ERG released asks the rider
 * to spin up. The rider's own pause, a lost strap (the ride goes on) and a free ride are facts the rider chose or can
 * ride through, and each is also on screen elsewhere (the key row's state line, the HR readout, the title).
 */
export const BANNER_ORDER = [
  "engine-unresponsive",
  "loss-trainer",
  "re-entry",
  "soft-start",
  "loss-erg",
  "paused",
  "loss-hr",
  "free-ride",
] as const;
export type BannerId = (typeof BANNER_ORDER)[number];

/** One line: its text, an optional number in the number face, and a test hook. */
export interface Line {
  text: string;
  /** A duration after the text, `data-testid="lost-for"`. */
  time?: string;
  testid?: string;
}

export interface Banner {
  id: BannerId;
  /** Most important first; the first is the bold line. */
  lines: Line[];
  /** Read between the lines (screen readers, the element's text), not drawn. */
  sep?: string;
  alert?: boolean;
  /** The line that carries the End… action (§5.2: a trainer lost ≥ 30 s); the banner always gets that many lines. */
  end?: number;
}

/** The trainer as the engine commands it (`commanded`, #324), for the pause banner and the key row. */
export function trainerNow(
  c: StreamCommanded | null | undefined,
): string | null {
  if (!c) return null;
  switch (c.kind) {
    case "erg":
      return `trainer in ERG at ${c.watts} W`;
    case "resistance":
      return c.pct === null
        ? "trainer in resistance mode"
        : `trainer resistance ${c.pct} %`;
    case "free":
      return "trainer on a flat road";
    case "sim":
      return "trainer in sim mode";
  }
}

/** What the banners are built from: `lossView` and the status, nothing derived here. */
export interface BannerInput {
  unresponsive: boolean;
  loss: {
    trainer: boolean;
    reconnecting: boolean;
    attempt: number | null;
    lostForS: number | null;
    long: boolean;
    reEntry: boolean;
    gapS: number | null;
    softStartW: number | null;
    hr: boolean;
    erg: boolean;
  };
  userPaused: boolean;
  free: boolean;
  commanded: StreamCommanded | null | undefined;
  clock: (s: number | null) => string;
}

const text = (t: string): Line => ({ text: t });

/** Every banner that holds, in BANNER_ORDER. */
export function banners(i: BannerInput): Banner[] {
  const { loss, clock } = i;
  const out: Banner[] = [];
  if (i.unresponsive)
    out.push({
      id: "engine-unresponsive",
      alert: true,
      lines: [
        "Engine not answering",
        "End still ends the ride",
        "the values are not live",
      ].map(text),
    });
  if (loss.trainer) {
    const what = loss.reconnecting
      ? loss.hr
        ? "NEO and HR lost"
        : "NEO lost"
      : loss.hr
        ? "No NEO data, HR lost"
        : "No data from the NEO";
    const state = loss.reconnecting
      ? loss.attempt === null
        ? "reconnecting"
        : `reconnecting (attempt ${loss.attempt})`
      : "your ride is saved";
    out.push({
      id: "loss-trainer",
      end: loss.long ? 1 : undefined,
      lines: [
        {
          text: what,
          time: loss.lostForS === null ? undefined : clock(loss.lostForS),
        },
        // ≥ 30 s: what to try, and End… on the bold line, replace the plain choice (§5.2 "Trainer lost ≥ 30 s").
        loss.long
          ? { text: "power-cycle · close Zwift", testid: "loss-help" }
          : text("keep pedalling or stop"),
        text(state),
      ],
    });
  }
  if (loss.reEntry)
    out.push({
      id: "re-entry",
      lines: [text(`↻ Back after ${clock(loss.gapS)}`)],
    });
  if (loss.softStartW !== null)
    out.push({
      id: "soft-start",
      lines: [
        text("Reconnected"),
        text(`easing back in to ${Math.round(loss.softStartW)} W`),
      ],
    });
  if (loss.erg)
    out.push({
      id: "loss-erg",
      sep: " — ",
      lines: [
        "ERG released",
        "spin up to 60 rpm",
        "the block timer runs on",
      ].map(text),
    });
  if (i.userPaused) {
    const trainer = trainerNow(i.commanded);
    out.push({
      id: "paused",
      lines: [
        "Paused",
        "Resume continues the plan",
        ...(trainer ? [trainer] : []),
      ].map(text),
    });
  }
  // A lost trainer's banner already names the strap ("NEO and HR lost").
  if (loss.hr && !loss.trainer)
    out.push({
      id: "loss-hr",
      lines: ["HR lost", "the ride goes on", "heart rate shows —"].map(text),
    });
  if (i.free)
    out.push({
      id: "free-ride",
      lines: ["Free ride", "no targets, no plan"].map(text),
    });
  return out;
}

/**
 * Lines per banner within the band: in order, each banner gets its bold line (and the line with End…, so the action is
 * never dropped) while room is left, then the lines left go to them in order. A banner that gets no room is not shown;
 * the order puts last the states that are also on screen elsewhere.
 */
export function allocate(list: readonly Banner[], room = BAND_LINES): number[] {
  let left = room;
  const n = list.map((b) => {
    const need = (b.end ?? 0) + 1;
    if (need > left) return 0;
    left -= need;
    return need;
  });
  list.forEach((b, k) => {
    if (n[k] === 0) return;
    const more = Math.min(left, b.lines.length - n[k]!);
    n[k]! += more;
    left -= more;
  });
  return n;
}

/** The band: each banner with its allotted lines. */
export function BannerBand({ list, end }: { list: Banner[]; end: ReactNode }) {
  const n = allocate(list);
  return list.map((b, k) =>
    n[k] === 0 ? null : (
      <div
        key={b.id}
        role={b.alert ? "alert" : "status"}
        className="banner"
        data-testid={b.id}
        data-state={b.id}
      >
        {b.lines.slice(0, n[k]).map((l, j) => (
          <p
            key={j}
            className="banner-line"
            data-tier="t3plus"
            data-testid={l.testid}
            data-state-label={j === 0 ? "" : undefined}
          >
            {j > 0 && <span className="sr-only">{b.sep ?? " · "}</span>}
            {j === 0 ? (
              <strong className="banner-text">
                {l.text}
                {l.time !== undefined && (
                  <>
                    {" "}
                    <span className="num" data-testid="lost-for">
                      {l.time}
                    </span>
                  </>
                )}
              </strong>
            ) : (
              <span className="banner-text">{l.text}</span>
            )}
            {j === b.end && end}
          </p>
        ))}
      </div>
    ),
  );
}
