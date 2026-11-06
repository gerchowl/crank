import { describe, expect, test } from "vitest";
import { clock } from "./Ride";
import {
  allocate,
  BAND_LINES,
  BANNER_ORDER,
  banners,
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
/** The band as the rider reads it: each shown banner's id and its lines. */
const band = (i: BannerInput) => {
  const list = banners(i);
  const n = allocate(list);
  return list.flatMap((b, k) =>
    n[k] ? [[b.id, ...b.lines.slice(0, n[k]).map((l) => l.text)]] : [],
  );
};
const lost = { trainer: true, reconnecting: true, attempt: 2, lostForS: 12 };

describe("the banner band (DESIGN §5.2, §6.5, §18.3)", () => {
  test("the engine not answering does not hide a lost trainer: both show, the engine first", () => {
    expect(band(input({ unresponsive: true }, lost))).toEqual([
      [
        "engine-unresponsive",
        "Engine not answering",
        "End still ends the ride",
      ],
      ["loss-trainer", "NEO lost"],
    ]);
  });

  test("HR lost while paused: both show, the pause first, each with its action", () => {
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

  test("a lost trainer alone keeps the rider's choice and the reconnect state", () => {
    expect(band(input({}, lost))).toEqual([
      [
        "loss-trainer",
        "NEO lost",
        "keep pedalling or stop",
        "reconnecting (attempt 2)",
      ],
    ]);
  });

  test("lost for 30 s or more: End… and what to try stay even when the band is full", () => {
    const list = banners(
      input({ unresponsive: true, userPaused: true }, { ...lost, long: true }),
    );
    const n = allocate(list);
    const trainer = list.findIndex((b) => b.id === "loss-trainer");
    expect(list[trainer]!.end).toBe(1);
    expect(n[trainer]).toBe(2);
    expect(list[trainer]!.lines[1]!.text).toBe("power-cycle · close Zwift");
    // The rider's pause is the one left out: the key row's state line and Resume still say it.
    expect(n[list.findIndex((b) => b.id === "paused")]).toBe(0);
    expect(n.reduce((a, b) => a + b, 0)).toBe(BAND_LINES);
  });

  test("the order is one list, and the band never holds more than three lines", () => {
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
    expect(allocate(all).reduce((a, b) => a + b, 0)).toBeLessThanOrEqual(
      BAND_LINES,
    );
  });

  test("the paused banner says what the engine commands the trainer, not a fixed word", () => {
    const paused = (commanded: BannerInput["commanded"]) =>
      banners(input({ userPaused: true, commanded }))
        .find((b) => b.id === "paused")
        ?.lines.map((l) => l.text);
    expect(paused({ kind: "resistance", pct: 0 })).toContain(
      "trainer resistance 0 %",
    );
    expect(paused({ kind: "erg", watts: 120 })).toContain(
      "trainer in ERG at 120 W",
    );
    expect(paused(null)).toEqual(["Paused", "Resume continues the plan"]);
    expect(trainerNow({ kind: "resistance", pct: 0 })).toBe(
      "trainer resistance 0 %",
    );
  });

  test(`every line of the longest copy is at most ${LINE_MAX} characters (§6.1)`, () => {
    const longest = banners(
      input(
        {
          unresponsive: true,
          userPaused: true,
          free: true,
          commanded: { kind: "resistance", pct: 100 },
        },
        {
          trainer: true,
          reconnecting: false,
          hr: true,
          lostForS: 3599,
          long: true,
          reEntry: true,
          gapS: 3599,
          softStartW: 1999,
          erg: true,
        },
      ),
    ).concat(
      banners(
        input(
          {},
          {
            trainer: true,
            reconnecting: true,
            attempt: 999,
            lostForS: 3599,
            hr: true,
          },
        ),
      ),
    );
    const lines = longest.flatMap((b) =>
      b.lines.map((l) => (l.time ? `${l.text} ${l.time}` : l.text)),
    );
    expect(lines.filter((l) => l.length > LINE_MAX)).toEqual([]);
  });
});
