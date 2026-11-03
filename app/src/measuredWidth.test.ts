import { expect, test, vi } from "vitest";
import { createMeasuredWidth } from "./measuredWidth";

/** A read the test settles by hand, in any order. */
function deferred() {
  const reads: ((v: number | null) => void)[] = [];
  const read = vi.fn(
    () => new Promise<number | null>((resolve) => reads.push(resolve)),
  );
  return { read, settle: (n: number, v: number | null) => reads[n]!(v) };
}
const noWatch = () => () => undefined;
const tick = () => new Promise((r) => setTimeout(r, 0));

test("reads on the first subscriber and tells its listener", async () => {
  const d = deferred();
  const store = createMeasuredWidth(d.read, noWatch);
  const listener = vi.fn();
  store.subscribe(listener);
  expect(d.read).toHaveBeenCalledTimes(1);
  expect(store.get()).toBeNull();
  d.settle(0, 288);
  await tick();
  expect(store.get()).toBe(288);
  expect(listener).toHaveBeenCalledTimes(1);
});

test("a slow earlier read cannot overwrite a newer one", async () => {
  const d = deferred();
  let refresh = () => {};
  const store = createMeasuredWidth(d.read, (r) => {
    refresh = r;
    return () => undefined;
  });
  store.subscribe(() => undefined);
  refresh(); // the window moved while the mount read (0) is still out: read 1
  d.settle(1, 597); // the newer read answers first
  await tick();
  expect(store.get()).toBe(597);
  d.settle(0, 288); // then the slow mount read arrives
  await tick();
  expect(store.get()).toBe(597);
});

test("an unchanged answer does not wake the listeners", async () => {
  const d = deferred();
  let refresh = () => {};
  const store = createMeasuredWidth(d.read, (r) => {
    refresh = r;
    return () => undefined;
  });
  const listener = vi.fn();
  store.subscribe(listener);
  d.settle(0, 288);
  await tick();
  refresh();
  d.settle(1, 288);
  await tick();
  expect(listener).toHaveBeenCalledTimes(1);
});

test("a read that outlives the last subscriber changes nothing", async () => {
  const d = deferred();
  const store = createMeasuredWidth(d.read, noWatch);
  const listener = vi.fn();
  const off = store.subscribe(listener);
  off(); // unmounted while the read is out
  d.settle(0, 288);
  await tick();
  expect(store.get()).toBeNull();
  expect(listener).not.toHaveBeenCalled();
});

test("the watch runs while subscribed and is stopped with the last subscriber", () => {
  const stop = vi.fn();
  const watch = vi.fn(() => stop);
  const store = createMeasuredWidth(() => Promise.resolve(null), watch);
  const a = store.subscribe(() => undefined);
  const b = store.subscribe(() => undefined);
  expect(watch).toHaveBeenCalledTimes(1);
  a();
  expect(stop).not.toHaveBeenCalled();
  b();
  expect(stop).toHaveBeenCalledTimes(1);
});

test("a second subscriber after the first left starts afresh (remount)", async () => {
  const d = deferred();
  const store = createMeasuredWidth(d.read, noWatch);
  store.subscribe(() => undefined)();
  const listener = vi.fn();
  store.subscribe(listener);
  expect(d.read).toHaveBeenCalledTimes(2);
  d.settle(0, 288); // the first, stale read
  d.settle(1, 597);
  await tick();
  expect(store.get()).toBe(597);
});
