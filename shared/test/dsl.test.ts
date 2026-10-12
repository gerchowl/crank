import { describe, expect, it } from "vitest";
import type { WorkoutNode } from "../src";
import { parseDsl, printDsl, Rng, SNIPPET_DSL, DEMO_DSL, withoutIds } from "../src";

const ok = (text: string): WorkoutNode[] => {
  const r = parseDsl(text, 280);
  if ("error" in r) throw new Error(`${r.error.message} @${r.error.offset}`);
  return r.nodes;
};
const err = (text: string) => {
  const r = parseDsl(text, 280);
  if (!("error" in r)) throw new Error("expected an error");
  return r.error;
};

describe("parseDsl", () => {
  it("parses the reference example (nested repeats, progression, skip-last, hr recovery)", () => {
    const nodes = ok("10m ramp 50-75%; 3x[+5w, !last](4x(30s 120%, 15s 50%), 3m 50%); hr<120 2m..4m 40%; 10m 55%");
    expect(withoutIds(nodes)).toEqual([
      { type: "step", end: { kind: "time", s: 600 }, target: { kind: "ramp", from: 50, to: 75 } },
      {
        type: "repeat",
        count: 3,
        progression: { wattsPerRep: 5 },
        skipLastRest: true,
        children: [
          {
            type: "repeat",
            count: 4,
            children: [
              { type: "step", end: { kind: "time", s: 30 }, target: { kind: "pct_ftp", value: 120 } },
              { type: "step", end: { kind: "time", s: 15 }, target: { kind: "pct_ftp", value: 50 } },
            ],
          },
          { type: "step", end: { kind: "time", s: 180 }, target: { kind: "pct_ftp", value: 50 } },
        ],
      },
      { type: "step", end: { kind: "hr_recovery", targetBpm: 120, minS: 120, maxS: 240, onUnreachable: "lower_power" }, target: { kind: "pct_ftp", value: 40 } },
      { type: "step", end: { kind: "time", s: 600 }, target: { kind: "pct_ftp", value: 55 } },
    ]);
  });

  it("parses every target / end / option form", () => {
    const nodes = ok(`"A \\"q\\"" 1h5m30s 250w 85-95rpm
      10kj 52.5-60.5%
      lap free
      20m 140bpm
      hr<115 0s..5m 35% !coach
      hr<115 1m..2m 35% !cap
      2x[-3%, +10w](1m 100% 90rpm, 30s 40%)`);
    expect(nodes).toHaveLength(7);
    const s0 = nodes[0] as Extract<WorkoutNode, { type: "step" }>;
    expect(s0.label).toBe('A "q"');
    expect(s0.end).toEqual({ kind: "time", s: 3930 });
    expect(s0.target).toEqual({ kind: "watts", value: 250 });
    expect(s0.cadence).toEqual({ lo: 85, hi: 95 });
    expect((nodes[1] as { end: unknown }).end).toEqual({ kind: "kj", kj: 10 });
    expect((nodes[1] as { target: unknown }).target).toEqual({ kind: "range", lo: 52.5, hi: 60.5 });
    expect((nodes[2] as { end: unknown }).end).toEqual({ kind: "lap" });
    expect((nodes[3] as { target: unknown }).target).toEqual({ kind: "hr", bpm: 140 });
    expect((nodes[4] as { end: { onUnreachable: string } }).end.onUnreachable).toBe("ask_coach");
    expect((nodes[5] as { end: { onUnreachable: string } }).end.onUnreachable).toBe("cap");
    expect((nodes[6] as { progression: unknown }).progression).toEqual({ pctPerRep: -3, wattsPerRep: 10 });
  });

  it("accepts comments, newlines and mixed separators", () => {
    const a = ok("# warm up\n10m 50%\n\n3x(1m 120%;1m 50%) # sets\n");
    const b = ok("10m 50%, 3x(1m 120%, 1m 50%)");
    expect(withoutIds(a)).toEqual(withoutIds(b));
  });

  it("assigns fresh unique ids", () => {
    const n = ok("2x(1m 100%, 1m 50%)");
    const ids = [n[0]!.id, ...(n[0] as { children: WorkoutNode[] }).children.map((c) => c.id)];
    expect(new Set(ids).size).toBe(3);
    expect(ok("1m 50%")[0]!.id).not.toBe(ok("1m 50%")[0]!.id);
  });

  it("parses an empty document", () => {
    expect(ok("  \n# nothing\n")).toEqual([]);
  });
});

describe("parseDsl errors carry offsets", () => {
  const cases: [string, string, RegExp][] = [
    ["10m 50%; 3x(1m 120%, 1m 50%", "(", /Unclosed '\('/],
    ["10m 50%)", ")", /Unexpected '\)'/],
    ["10m 50%; 5q 120%", "5q", /Expected a duration/],
    ["10m", "10m", /Missing power target/],
    ["10m 50% 60%", "60%", /Duplicate power target/],
    ["10m 50% zzz", "zzz", /Unknown token 'zzz'/],
    ["3x[+5q](1m 50%)", "+5q", /Unknown repeat option/],
    ["3x 1m 50%", "1m", /Expected '\('/],
    ["0x(1m 50%)", "0x", /Repeat count/],
    ["hr<120 4m 40%", "4m", /min\.\.max/],
    ["hr<120 4m..2m 40%", "4m..2m", /Min duration/],
    ["5m 50% !cap", "!cap", /only applies to hr</],
    ["5m 900%", "900%", /implausible/],
    ["5m 75-50%", "75-50%", /ramp/],
    ['"unterminated 5m 50%', '"unterminated 5m 50%', /Unterminated/],
    ["10 50%", "10", /needs a unit/],
    ["5m 50% 2x(1m 50%)", "2x", /Unknown token|Unexpected/],
  ];
  for (const [text, tok, re] of cases) {
    it(`${JSON.stringify(text)} → ${re}`, () => {
      const e = err(text);
      expect(e.message).toMatch(re);
      expect(text.slice(e.offset, e.offset + e.length)).toBe(tok);
    });
  }

  it("points at end of input for a dangling label", () => {
    const e = err('10m 50%\n"label"');
    expect(e.offset).toBe(15);
    expect(e.length).toBe(0);
  });

  it("reports an empty repeat spanning count..paren", () => {
    const text = "5m 50%; 3x()";
    const e = err(text);
    expect(e.message).toMatch(/Empty repeat/);
    expect(text.slice(e.offset, e.offset + e.length)).toBe("3x()");
  });
});

describe("printDsl round trip", () => {
  const roundTrip = (nodes: WorkoutNode[]) => {
    const text = printDsl(nodes);
    const back = ok(text);
    expect(withoutIds(back)).toEqual(withoutIds(nodes));
    expect(printDsl(back)).toBe(text);
  };

  it("round-trips library content", () => {
    for (const s of SNIPPET_DSL) roundTrip(ok(s.text));
    roundTrip(ok(DEMO_DSL));
  });

  it("prints canonical text", () => {
    expect(printDsl(ok("10m ramp 50-75%; 3x[+5w,!last](4x(30s 120%,15s 50%),3m 50%); hr<120 2m..4m 40%; 10m 55%"))).toBe(
      "10m ramp 50-75%\n3x[+5w, !last](4x(30s 120%, 15s 50%), 3m 50%)\nhr<120 2m..4m 40%\n10m 55%",
    );
    expect(printDsl(ok("90s 100%; 3600s 50%; 3661s 50%"))).toBe("1m30s 100%\n1h 50%\n1h1m1s 50%");
  });

  it("round-trips 300 random trees", () => {
    const rng = new Rng(1234);
    const step = (): WorkoutNode => {
      const k = rng.int(0, 9);
      const end =
        k < 6
          ? { kind: "time" as const, s: rng.int(1, 7200) }
          : k === 6
            ? { kind: "kj" as const, kj: rng.int(1, 300) }
            : k === 7
              ? { kind: "lap" as const }
              : {
                  kind: "hr_recovery" as const,
                  targetBpm: rng.int(90, 150),
                  minS: rng.int(0, 120),
                  maxS: rng.int(121, 600),
                  onUnreachable: rng.pick(["lower_power", "cap", "ask_coach"] as const),
                };
      const t = rng.int(0, 5);
      const target =
        t === 0
          ? { kind: "pct_ftp" as const, value: rng.int(0, 2000) / 10 }
          : t === 1
            ? { kind: "watts" as const, value: rng.int(0, 900) }
            : t === 2
              ? (() => {
                  const lo = rng.int(30, 120);
                  return { kind: "range" as const, lo, hi: lo + rng.int(0, 30) };
                })()
              : t === 3
                ? { kind: "ramp" as const, from: rng.int(30, 150), to: rng.int(30, 150) }
                : t === 4
                  ? { kind: "hr" as const, bpm: rng.int(100, 180) }
                  : { kind: "free" as const };
      const s: WorkoutNode = { type: "step", id: "x", end, target };
      if (rng.chance(0.3)) s.label = rng.pick(["Warm-up", 'say "hi"', "back\\slash", "VO2, max; (hard)", ""]);
      if (rng.chance(0.3)) {
        const lo = rng.int(60, 100);
        s.cadence = { lo, hi: rng.chance(0.5) ? lo : lo + rng.int(1, 20) };
      }
      return s;
    };
    const tree = (depth: number): WorkoutNode[] =>
      Array.from({ length: rng.int(1, 4) }, () => {
        if (depth < 3 && rng.chance(0.35)) {
          const r: WorkoutNode = { type: "repeat", id: "r", count: rng.int(1, 12), children: tree(depth + 1) };
          if (rng.chance(0.4)) r.skipLastRest = true;
          if (rng.chance(0.4)) {
            r.progression = {};
            if (rng.chance(0.6)) r.progression.wattsPerRep = rng.int(-10, 10);
            if (rng.chance(0.6) || r.progression.wattsPerRep === undefined) r.progression.pctPerRep = rng.int(-30, 30) / 10;
          }
          if (rng.chance(0.2)) r.label = "Set";
          return r;
        }
        return step();
      });
    for (let i = 0; i < 300; i++) roundTrip(tree(0));
  });
});
