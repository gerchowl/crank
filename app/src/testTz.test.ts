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

/** Runs `fn` with `TZ` set to `prior` (absent when undefined) and puts the real value back, whatever `fn` does. */
async function withPrior(
  prior: string | undefined,
  fn: () => void | Promise<void>,
): Promise<void> {
  const had = Object.prototype.hasOwnProperty.call(process.env, "TZ");
  const saved = process.env.TZ;
  if (prior === undefined) delete process.env.TZ;
  else process.env.TZ = prior;
  try {
    await fn();
  } finally {
    if (had) process.env.TZ = saved;
    else delete process.env.TZ;
  }
}
const hasTz = () => Object.prototype.hasOwnProperty.call(process.env, "TZ");

// each of the next tests sets the zone it starts from, so none depends on the hooks or on the order of the tests
test("withTimeZone restores the prior zone when its body throws, and the error propagates", async () => {
  await withPrior("America/Chicago", async () => {
    await expect(
      withTimeZone("Asia/Kolkata", () => {
        expect(hours()).toBe(5);
        throw new Error("body failed");
      }),
    ).rejects.toThrow("body failed");
    expect(process.env.TZ).toBe("America/Chicago");
  });
});

test("withTimeZone restores an absent TZ by deleting it, never by setting the string 'undefined'", async () => {
  await withPrior(undefined, async () => {
    await withTimeZone("Asia/Tokyo", () => {
      expect(process.env.TZ).toBe("Asia/Tokyo");
    });
    expect(hasTz()).toBe(false);
    expect(process.env.TZ).toBeUndefined();
  });
});

test("withTimeZone restores an absent TZ when its body throws", async () => {
  await withPrior(undefined, async () => {
    await expect(
      withTimeZone("Asia/Tokyo", () => {
        throw new Error("body failed");
      }),
    ).rejects.toThrow("body failed");
    expect(hasTz()).toBe(false);
  });
});

test("setTimeZone's undo removes a zone that was never set", async () => {
  await withPrior(undefined, () => {
    const undo = setTimeZone("Asia/Tokyo");
    undo();
    expect(hasTz()).toBe(false);
  });
});
