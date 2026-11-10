import { describe, expect, test } from "vitest";
import type { LinkStatus, StreamRideState, StreamStatus } from "../bindings";
import { status as makeStatus } from "../stream/testBackend";
import { clock, lossView } from "./Ride";
import {
  BAND_LINES,
  BANNER_ORDER,
  banners,
  layout,
  LINE_MAX,
  trainerNow,
  type BannerInput,
} from "./banners";

const calm: BannerInput["loss"] = {
  trainer: false,
  reconnecting: false,
  attempt: null,
  lostForS: null,
  long: false,
  reEntry: false,
  gapS: null,
  softStartW: null,
  hr: false,
  erg: false,
};
const input = (
  over: Partial<BannerInput> = {},
  loss: Partial<BannerInput["loss"]> = {},
): BannerInput => ({
  unresponsive: false,
  userPaused: false,
  free: false,
  commanded: null,
  clock,
  ...over,
  loss: { ...calm, ...loss },
});
/** The band as the rider reads it: each shown banner's id and the text of its lines. */
const band = (i: BannerInput) =>
  layout(banners(i)).map(({ banner, lines }) => [
    banner.id,
    ...lines.map((l) => (l.time ? `${l.text} ${l.time}` : l.text)),
  ]);
const lost = { trainer: true, reconnecting: true, attempt: 2, lostForS: 12 };

/**
 * Every combination of states the screen can derive: a grid over the status fields and links `lossView` reads (each at
 * its longest value where length matters), times the engine answering or not and a planned or free ride.
 */
function* everyState(): Generator<BannerInput> {
  const states: StreamRideState[] = [
    { kind: "riding" },
    { kind: "paused", reason: "user" },
    { kind: "paused", reason: "trainer_lost" },
    { kind: "paused", reason: "stopped_pedalling" },
  ];
  const links: (LinkStatus | undefined)[] = [
    "ready",
    "lost",
    "connecting",
    undefined,
  ];
  for (const state of states)
    for (const userPaused of [false, true])
      for (const lostSince of [null, 3599 - 59 * 60 - 59, 3599 - 5]) {
        // A loss stamp comes with the trainer-lost pause, which outranks every other reason (engine.rs `Session::lose`
        // sets `pauses.lost` with `lost_at`; engine.rs `Pauses::reason`): no other state carries it.
        if (
          lostSince !== null &&
          !(state.kind === "paused" && state.reason === "trainer_lost")
        )
          continue;
        for (const trainer of links)
          for (const hr of ["ready", "lost"] as const)
            for (const ergReleased of [false, true])
              for (const softStartTargetW of [null, 1999])
                for (const lastGapS of [null, 4, 3599])
                  for (const unresponsive of [false, true])
                    for (const free of [false, true]) {
                      const st: StreamStatus = makeStatus({
                        state,
                        sessionS: 3599,
                        userPaused,
                        trainerLostSinceS: lostSince,
                        reconnectAttempt: lostSince === null ? null : 999,
                        ergReleased,
                        softStartTargetW,
                        lastGapS,
                        reconnectedAtS: lastGapS === null ? null : 3595,
                        commanded: { kind: "resistance", pct: 100 },
                      });
                      yield {
                        unresponsive,
                        free,
                        userPaused,
                        commanded: st.commanded,
                        clock,
                        loss: lossView(st, { trainer, hr }),
                      };
                    }
      }
}

describe("the banner band (DESIGN §5.2, §6.5, §18.3)", () => {
  test("every reachable combination: each active state's actions show, in three lines of at most 26 characters", () => {
    let combos = 0;
    for (const i of everyState()) {
      const list = banners(i);
      const shown = layout(list);
      const lines = shown.flatMap((s) => s.lines);
      expect(lines.length).toBeLessThanOrEqual(BAND_LINES);
      const texts = lines.map((l) => (l.time ? `${l.text} ${l.time}` : l.text));
      expect(texts.filter((t) => t.length > LINE_MAX)).toEqual([]);
      for (const b of list) {
        if (b.actions.length === 0) continue;
        const on = shown.find((s) => s.banner === b);
        expect(on, `${b.id} is shown`).toBeDefined();
        // Each action is on screen as its own line, or the first folded with the title into the compact line.
        const folded = on!.lines[0]!.text === b.compact;
        const want = folded ? b.actions.slice(1) : b.actions;
        for (const a of want) expect(on!.lines).toContainEqual(a);
        if (b.actions.some((a) => a.end))
          expect(on!.lines.some((l) => l.end)).toBe(true);
      }
      combos++;
    }
    expect(combos).toBe(4608);
  });

  test("the compact lines keep the action's meaning", () => {
    const compact = (i: BannerInput) =>
      Object.fromEntries(banners(i).map((b) => [b.id, b.compact]));
    expect(
      compact(input({ unresponsive: true }, { ...lost, erg: true })),
    ).toEqual({
      "engine-unresponsive": "Engine silent: End works",
      "loss-trainer": "NEO lost: pedal or stop",
      "loss-erg": "ERG off: spin up to 45 rpm",
    });
  });

  test("the engine not answering with a lost trainer: both actions show (End still ends the ride, keep pedalling)", () => {
    expect(band(input({ unresponsive: true }, lost))).toEqual([
      [
        "engine-unresponsive",
        "Engine not answering",
        "End still ends the ride",
      ],
      ["loss-trainer", "NEO lost: pedal or stop"],
    ]);
  });

  test("lost for 30 s or more adds what to try and End… to the plain choice; status goes first", () => {
    expect(band(input({}, { ...lost, long: true, lostForS: 42 }))).toEqual([
      [
        "loss-trainer",
        "NEO lost 0:42",
        "keep pedalling or stop",
        "power-cycle · close Zwift",
      ],
    ]);
    expect(
      band(input({ unresponsive: true }, { ...lost, long: true })),
    ).toEqual([
      ["engine-unresponsive", "Engine silent: End works"],
      ["loss-trainer", "NEO lost: pedal or stop", "power-cycle · close Zwift"],
    ]);
  });

  test("a lost trainer alone keeps the rider's choice and the reconnect state", () => {
    expect(band(input({}, lost))).toEqual([
      [
        "loss-trainer",
        "NEO lost 0:12",
        "keep pedalling or stop",
        "reconnecting (attempt 2)",
      ],
    ]);
  });

  test("HR lost while paused: both show, the pause first", () => {
    expect(
      band(
        input(
          { userPaused: true, commanded: { kind: "resistance", pct: 0 } },
          { hr: true },
        ),
      ),
    ).toEqual([
      ["paused", "Paused", "Resume continues the plan"],
      ["loss-hr", "HR lost"],
    ]);
  });

  test("the order is one list", () => {
    const all = banners(
      input(
        { unresponsive: true, userPaused: true, free: true },
        {
          ...lost,
          reEntry: true,
          gapS: 200,
          softStartW: 250,
          erg: true,
          hr: true,
        },
      ),
    );
    expect(all.map((b) => b.id)).toEqual(
      BANNER_ORDER.filter((id) => all.some((b) => b.id === id)),
    );
  });

  test("the paused banner says what the engine commands the trainer, not a fixed word", () => {
    const paused = (commanded: BannerInput["commanded"]) =>
      banners(input({ userPaused: true, commanded }))
        .find((b) => b.id === "paused")
        ?.status.map((l) => l.text);
    expect(paused({ kind: "resistance", pct: 0 })).toContain(
      "trainer resistance 0 %",
    );
    expect(paused({ kind: "erg", watts: 120 })).toContain(
      "trainer in ERG at 120 W",
    );
    expect(paused(null)).toEqual(["Resume continues the plan"]);
    expect(trainerNow({ kind: "resistance", pct: 0 })).toBe(
      "trainer resistance 0 %",
    );
  });
});
