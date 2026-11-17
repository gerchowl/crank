// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { markSeen, parseSeen, seenBuild } from "./build";

beforeEach(() => localStorage.clear());
afterEach(() => vi.restoreAllMocks());

describe("the seen record", () => {
  test("round-trips a build string", () => {
    expect(seenBuild()).toBeNull();
    markSeen("0.2.0-rc.1 · 7516045-dirty");
    expect(seenBuild()).toBe("0.2.0-rc.1 · 7516045-dirty");
  });

  test("accepts exactly {v:1, build:<non-empty string>}", () => {
    expect(parseSeen('{"v":1,"build":"0.1.0 · dev"}')).toBe("0.1.0 · dev");
    for (const raw of [
      "",
      "not json",
      "null",
      '["0.1.0 · dev"]',
      '{"v":2,"build":"0.1.0 · dev"}',
      '{"v":1,"build":""}',
      '{"v":1,"build":7}',
      '{"v":1}',
      '{"v":1,"build":"0.1.0 · dev","extra":true}',
    ])
      expect(parseSeen(raw), raw).toBeNull();
  });

  test("unreadable or unwritable storage never throws: the motion just plays in full again", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota");
    });
    expect(() => markSeen("0.1.0 · dev")).not.toThrow();
    expect(seenBuild()).toBeNull();
  });
});
