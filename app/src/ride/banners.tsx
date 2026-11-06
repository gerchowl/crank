// The state banners of the ride screen (DESIGN §5.2 States, §6.1, §6.5, §18.2, §18.3): the band at the top of the traces
// column holds three T3+ lines of at most 26 characters (§6.1, re-derived for A in #467). Every word comes from the
// status, the links and the stream (`lossView`), never from a timer of this screen.
//
// The rule (#537 review): every active state's action instruction is always visible; status and context text (the
// attempt, "your ride is saved", the elapsed time) is dropped first when the band is full. Each banner therefore names
// its title, its action lines and its status lines, and `layout` fits them: titles with their actions first, a banner
// whose title and first action do not both fit folds them into one compact line (`compact`), and only the room left
// takes the titles of action-free states and status lines, most urgent first.
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
 * ride through, and each is also on screen elsewhere (the key row's state line and Resume, the HR readout, the title).
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

/** One line: its text, an optional duration in the number face, a test hook, and whether End… sits on it. */
export interface Line {
  text: string;
  /** A duration after the text, `data-testid="lost-for"` (context: the compact line drops it). */
  time?: string;
  testid?: string;
  /** The End… action is on this line (§5.2: a trainer lost ≥ 30 s). */
  end?: boolean;
}

export interface Banner {
  id: BannerId;
  title: Line;
  /** What the rider is to do: shown whenever the state holds. */
  actions: Line[];
  /** Context, shown only in the room the actions leave. */
  status: Line[];
  /** The title and the first action in one line, for a full band. */
  compact?: string;
  /** Read between the lines (screen readers, the element's text), not drawn. */
  sep?: string;
  alert?: boolean;
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
      title: text("Engine not answering"),
      actions: [text("End still ends the ride")],
      status: [text("the values are not live")],
      compact: "Engine silent: End works",
    });
  if (loss.trainer) {
    const what = loss.reconnecting
      ? loss.hr
        ? "NEO and HR lost"
        : "NEO lost"
      : loss.hr
        ? "No NEO data, HR lost"
        : "No data from the NEO";
    const short = loss.hr
      ? "NEO+HR lost"
      : loss.reconnecting
        ? "NEO lost"
        : "NEO silent";
    out.push({
      id: "loss-trainer",
      title: {
        text: what,
        time: loss.lostForS === null ? undefined : clock(loss.lostForS),
      },
      // ≥ 30 s adds what to try, with End… (§5.2 "Trainer lost ≥ 30 s"); the plain choice stays.
      actions: [
        text("keep pedalling or stop"),
        ...(loss.long
          ? [
              {
                text: "power-cycle · close Zwift",
                testid: "loss-help",
                end: true,
              },
            ]
          : []),
      ],
      status: [
        text(
          loss.reconnecting
            ? loss.attempt === null
              ? "reconnecting"
              : `reconnecting (attempt ${loss.attempt})`
            : "your ride is saved",
        ),
      ],
      compact: `${short}: pedal or stop`,
    });
  }
  if (loss.reEntry)
    out.push({
      id: "re-entry",
      title: text(`↻ Back after ${clock(loss.gapS)}`),
      actions: [],
      status: [],
    });
  if (loss.softStartW !== null)
    out.push({
      id: "soft-start",
      title: text("Reconnected"),
      actions: [],
      status: [text(`easing back in to ${Math.round(loss.softStartW)} W`)],
    });
  if (loss.erg)
    out.push({
      id: "loss-erg",
      sep: " — ",
      title: text("ERG released"),
      actions: [text("spin up to 60 rpm")],
      status: [text("the block timer runs on")],
      compact: "ERG off: spin up to 60 rpm",
    });
  if (i.userPaused) {
    // The rider's own pause: its action is the key row's Resume, so these lines are context.
    const trainer = trainerNow(i.commanded);
    out.push({
      id: "paused",
      title: text("Paused"),
      actions: [],
      status: [
        text("Resume continues the plan"),
        ...(trainer ? [text(trainer)] : []),
      ],
    });
  }
  // A lost trainer's banner already names the strap ("NEO and HR lost").
  if (loss.hr && !loss.trainer)
    out.push({
      id: "loss-hr",
      title: text("HR lost"),
      actions: [],
      status: [text("the ride goes on"), text("heart rate shows —")],
    });
  if (i.free)
    out.push({
      id: "free-ride",
      title: text("Free ride"),
      actions: [],
      status: [text("no targets, no plan")],
    });
  return out;
}

/** A banner as the band shows it: its lines, the first drawn bold. */
export interface Shown {
  banner: Banner;
  lines: Line[];
}

/**
 * Fits the banners into `room` lines. Every banner with an action gets its title and all its actions; while they need
 * more than the room, the least urgent banner that has a compact line folds its title into its first action. The room
 * left goes, most urgent first, to the titles of action-free banners, then to status lines. Throws if the actions alone
 * do not fit: the copy must then change (banners.test.ts walks every reachable combination of states).
 */
export function layout(list: readonly Banner[], room = BAND_LINES): Shown[] {
  const folded = new Set<Banner>();
  const need = (b: Banner) =>
    b.actions.length === 0
      ? 0
      : folded.has(b)
        ? b.actions.length
        : b.actions.length + 1;
  const total = () => list.reduce((n, b) => n + need(b), 0);
  for (let k = list.length - 1; k >= 0 && total() > room; k--) {
    const b = list[k]!;
    if (b.compact !== undefined && b.actions.length > 0) folded.add(b);
  }
  if (total() > room)
    throw new Error(
      `banner actions need ${total()} lines: ${list.map((b) => b.id).join(", ")}`,
    );
  let left = room - total();
  const shown = list.map((b) => {
    if (need(b) > 0) return true;
    if (left === 0) return false;
    left -= 1;
    return true;
  });
  const extra = list.map((b, k) => {
    if (!shown[k]) return 0;
    const more = Math.min(left, b.status.length);
    left -= more;
    return more;
  });
  return list.flatMap((b, k) => {
    if (!shown[k]) return [];
    const head: Line[] = folded.has(b)
      ? [
          { text: b.compact ?? b.title.text, end: b.actions[0]?.end },
          ...b.actions.slice(1),
        ]
      : [b.title, ...b.actions];
    return [{ banner: b, lines: [...head, ...b.status.slice(0, extra[k])] }];
  });
}

/** The band: each shown banner with its lines. */
export function BannerBand({ list, end }: { list: Banner[]; end: ReactNode }) {
  return layout(list).map(({ banner: b, lines }) => (
    <div
      key={b.id}
      role={b.alert ? "alert" : "status"}
      className="banner"
      data-testid={b.id}
      data-state={b.id}
    >
      {lines.map((l, j) => (
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
          {l.end && end}
        </p>
      ))}
    </div>
  ));
}
