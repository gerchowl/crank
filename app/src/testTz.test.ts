import { afterAll, beforeAll, expect, test } from "vitest";
import { setTimeZone, withTimeZone } from "./testTz";

const original = Object.prototype.hasOwnProperty.call(process.env, "TZ")
  ? process.env.TZ
  : undefined;
const hours = () => new Date(0).getHours();

// the contract the History test relies on: a pinned zone is gone once the file's hooks have run
let restore: () => void;
beforeAll(() => {
  restore = setTimeZone("UTC");
});
afterAll(() => {
  restore();
  expect(process.env.TZ).toBe(original);
});

test("the zone is pinned while the file runs", () => {
  expect(process.env.TZ).toBe("UTC");
  expect(hours()).toBe(0);
});

test("withTimeZone restores the zone when its body fails", async () => {
  await expect(
    withTimeZone("Asia/Kolkata", () => {
      expect(hours()).toBe(5);
      throw new Error("body failed");
    }),
  ).rejects.toThrow("body failed");
  expect(process.env.TZ).toBe("UTC");
  expect(hours()).toBe(0);
});

test("restoring a zone that was never set removes it", () => {
  const had = Object.prototype.hasOwnProperty.call(process.env, "TZ");
  const prior = process.env.TZ;
  delete process.env.TZ;
  try {
    const undo = setTimeZone("Asia/Tokyo");
    undo();
    expect(Object.prototype.hasOwnProperty.call(process.env, "TZ")).toBe(false);
  } finally {
    // a failed expectation must not leak the deleted zone; an absent prior is deleted, never set to "undefined"
    if (had) process.env.TZ = prior;
    else delete process.env.TZ;
  }
});

// runs after the test above: a failure there must not leave the pinned zone changed for the rest of the file
test("the pinned zone survives the previous test", () => {
  expect(process.env.TZ).toBe("UTC");
});
