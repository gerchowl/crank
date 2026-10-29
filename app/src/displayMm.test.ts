import { afterEach, beforeEach, expect, test, vi } from "vitest";

type Handler<T> = (event: { payload: T }) => void;
const hooks = vi.hoisted(() => ({
  moved: [] as (() => void)[],
  scaled: [] as (() => void)[],
  focused: [] as Handler<boolean>[],
  offs: [] as ReturnType<typeof vi.fn>[],
  /** Hold the registration promises until released. */
  gate: null as Promise<void> | null,
}));
vi.mock("@tauri-apps/api/window", () => {
  const register = <T>(into: T[], handler: T) => {
    into.push(handler);
    const off = vi.fn();
    hooks.offs.push(off);
    return (hooks.gate ?? Promise.resolve()).then(() => off);
  };
  return {
    getCurrentWindow: () => ({
      onMoved: (h: () => void) => register(hooks.moved, h),
      onScaleChanged: (h: () => void) => register(hooks.scaled, h),
      onFocusChanged: (h: Handler<boolean>) => register(hooks.focused, h),
    }),
  };
});
vi.mock("./bindings", () => ({ commands: { displayWidthMm: vi.fn() } }));
const { watchMonitorChange } = await import("./displayMm");

beforeEach(() => {
  vi.useFakeTimers();
  hooks.moved.length = hooks.scaled.length = hooks.focused.length = 0;
  hooks.offs.length = 0;
  hooks.gate = null;
});
afterEach(() => vi.useRealTimers());

const SETTLE = 200;

test("a move, a scale change and a focus gain each ask for a refresh once settled", async () => {
  const notify = vi.fn();
  watchMonitorChange(notify);
  await vi.advanceTimersByTimeAsync(0);
  hooks.moved[0]!();
  await vi.advanceTimersByTimeAsync(SETTLE);
  expect(notify).toHaveBeenCalledTimes(1);
  hooks.scaled[0]!();
  await vi.advanceTimersByTimeAsync(SETTLE);
  expect(notify).toHaveBeenCalledTimes(2);
  hooks.focused[0]!({ payload: true });
  await vi.advanceTimersByTimeAsync(SETTLE);
  expect(notify).toHaveBeenCalledTimes(3);
});

test("losing focus does not refresh", async () => {
  const notify = vi.fn();
  watchMonitorChange(notify);
  await vi.advanceTimersByTimeAsync(0);
  hooks.focused[0]!({ payload: false });
  await vi.advanceTimersByTimeAsync(SETTLE * 2);
  expect(notify).not.toHaveBeenCalled();
});

test("a burst of events is one refresh", async () => {
  const notify = vi.fn();
  watchMonitorChange(notify);
  await vi.advanceTimersByTimeAsync(0);
  for (let i = 0; i < 20; i++) {
    hooks.moved[0]!();
    await vi.advanceTimersByTimeAsync(50);
  }
  hooks.scaled[0]!();
  await vi.advanceTimersByTimeAsync(SETTLE);
  expect(notify).toHaveBeenCalledTimes(1);
});

test("unsubscribing removes all three listeners and cancels a pending refresh", async () => {
  const notify = vi.fn();
  const stop = watchMonitorChange(notify);
  await vi.advanceTimersByTimeAsync(0);
  hooks.moved[0]!();
  stop();
  await vi.advanceTimersByTimeAsync(SETTLE * 2);
  expect(notify).not.toHaveBeenCalled();
  expect(hooks.offs).toHaveLength(3);
  for (const off of hooks.offs) expect(off).toHaveBeenCalledTimes(1);
});

test("a listener whose registration resolves after the unsubscribe is removed at once", async () => {
  let open = () => {};
  hooks.gate = new Promise<void>((resolve) => (open = resolve));
  const stop = watchMonitorChange(vi.fn());
  stop(); // before Tauri has answered any registration
  open();
  await vi.advanceTimersByTimeAsync(0);
  expect(hooks.offs).toHaveLength(3);
  for (const off of hooks.offs) expect(off).toHaveBeenCalledTimes(1);
});
