import { describe, expect, test } from "vitest";
import { Draft } from "./draft";

const positive = (n: number) => (n > 0 ? null : "must be above 0");

describe("Draft (the settings-draft primitive, #90's shape)", () => {
  test("set changes the draft only; cancel returns to the saved value", () => {
    const d = new Draft<number>(1, positive);
    d.set(5);
    expect(d.read().draft).toBe(5);
    expect(d.savedValue()).toBe(1);
    d.cancel();
    expect(d.read().draft).toBe(1);
  });

  test("save validates first: a refused draft is never written and the saved value stays", async () => {
    const d = new Draft<number>(1, positive);
    const written: number[] = [];
    d.set(-2);
    expect(await d.save(async (v) => (written.push(v), null))).toBe(false);
    expect(written).toEqual([]);
    expect(d.read().error).toBe("must be above 0");
    expect(d.savedValue()).toBe(1);
  });

  test("save writes once; a refused write keeps the draft open with its reason", async () => {
    const d = new Draft<number>(1, positive);
    d.set(7);
    expect(await d.save(async () => "refused")).toBe(false);
    expect(d.read()).toEqual({ draft: 7, error: "refused", saving: false });
    expect(d.savedValue()).toBe(1);
    const written: number[] = [];
    expect(await d.save(async (v) => (written.push(v), null))).toBe(true);
    expect(written).toEqual([7]);
    expect(d.savedValue()).toBe(7);
    expect(d.read().error).toBeNull();
  });

  test("a second save while one is under way writes nothing", async () => {
    const d = new Draft<number>(1, positive);
    let release!: () => void;
    const first = d.save(
      () => new Promise<null>((r) => (release = () => r(null))),
    );
    expect(d.read().saving).toBe(true);
    expect(await d.save(async () => "second")).toBe(false);
    release();
    expect(await first).toBe(true);
    expect(d.read().saving).toBe(false);
  });

  test("each change is one new read() object; no change, the same one", () => {
    const d = new Draft<number>(1, positive);
    const a = d.read();
    expect(d.read()).toBe(a);
    d.set(2);
    expect(d.read()).not.toBe(a);
  });
});
