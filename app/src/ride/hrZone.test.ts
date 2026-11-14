import { describe, expect, test } from "vitest";
import { frielZone, hrZone, pctMaxZone, zoneText } from "./hrZone";

// With a reference of 100 bpm, a bpm is a whole percent, so each case is a table edge as the source writes it.
describe("Friel's bike zones, % of LTHR", () => {
  test.each([
    [1, "Z1"],
    [80, "Z1"], // "Less than 81%"
    [81, "Z2"], // "81% to 89%"
    [89, "Z2"],
    [90, "Z3"], // "90% to 93%"
    [93, "Z3"],
    [94, "Z4"], // "94% to 99%"
    [99, "Z4"],
    [100, "Z5a"], // "100% to 102%"
    [102, "Z5a"],
    [103, "Z5b"], // "103% to 106%"
    [106, "Z5b"],
    [107, "Z5c"], // "More than 106%"
    [150, "Z5c"],
  ])("%i %% is %s", (pct, zone) => {
    expect(frielZone(pct, 100)).toBe(zone);
  });

  test("the percent is rounded half up to a whole one before the table applies", () => {
    // LTHR 170: 137 bpm is 80.59 % → 81 → Z2; 136 bpm is 80.0 % → Z1.
    expect(frielZone(136, 170)).toBe("Z1");
    expect(frielZone(137, 170)).toBe("Z2");
    // 80.5 % rounds up to 81; 80.49 % stays 80.
    expect(frielZone(80.5, 100)).toBe("Z2");
    expect(frielZone(80.49, 100)).toBe("Z1");
    // 106.49 % is 106 → Z5b; 106.5 % is 107 → Z5c.
    expect(frielZone(106.49, 100)).toBe("Z5b");
    expect(frielZone(106.5, 100)).toBe("Z5c");
  });
});

describe("five zones, % of HR max", () => {
  test.each([
    [30, "Z1"], // below 50 % stays Z1 (DESIGN §18.3)
    [50, "Z1"], // "50–60%"
    [59, "Z1"],
    [60, "Z2"], // a shared edge belongs to the higher zone
    [69, "Z2"],
    [70, "Z3"],
    [79, "Z3"],
    [80, "Z4"],
    [89, "Z4"],
    [90, "Z5"], // "90–100%"
    [100, "Z5"],
    [104, "Z5"], // above the estimate stays Z5
  ])("%i %% is %s", (pct, zone) => {
    expect(pctMaxZone(pct, 100)).toBe(zone);
  });

  test("rounded like Friel's: 59.5 % is Z2, 59.49 % is Z1", () => {
    expect(pctMaxZone(59.5, 100)).toBe("Z2");
    expect(pctMaxZone(59.49, 100)).toBe("Z1");
  });
});

describe("hrZone: which model, and no data", () => {
  test("LTHR wins over HR max", () => {
    // 150 / 160 = 93.75 % of LTHR → 94 → Z4; of HR max it would be 79 % → Z3.
    const z = hrZone(150, { lthrBpm: 160, hrMaxBpm: 190 });
    expect(z).toEqual({ label: "Z4", model: "lthr" });
    expect(z && zoneText(z)).toBe("Z4 · LTHR");
  });

  test("HR max only: the five zones", () => {
    // 150 / 190 = 78.9 % → 79 → Z3.
    const z = hrZone(150, { lthrBpm: null, hrMaxBpm: 190 });
    expect(z).toEqual({ label: "Z3", model: "hrMax" });
    expect(z && zoneText(z)).toBe("Z3 · HRmax");
  });

  test("neither reference known: no zone", () => {
    expect(hrZone(150, { lthrBpm: null, hrMaxBpm: null })).toBeNull();
  });

  test("a reference that is no heart rate counts as unknown", () => {
    for (const bad of [0, -170, Number.NaN, Number.POSITIVE_INFINITY])
      expect(hrZone(150, { lthrBpm: bad, hrMaxBpm: null })).toBeNull();
    expect(hrZone(150, { lthrBpm: 0, hrMaxBpm: 190 })?.model).toBe("hrMax");
  });

  test("no HR value: no zone, whatever the references", () => {
    const refs = { lthrBpm: 160, hrMaxBpm: 190 };
    for (const hr of [null, 0, Number.NaN]) expect(hrZone(hr, refs)).toBeNull();
  });
});
