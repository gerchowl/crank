// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  onTestFinished,
  test,
  vi,
} from "vitest";
import { App, type ViewRegistry } from "./App";
import { navigate } from "./router";
import { installDevices } from "./views/devicesBackend";
import { HANDOFF_MS, STATUS_AFTER_MS } from "./shell/StartScreen";
import { reloadTheme } from "./theme";

type Listener = () => void;
/** A controllable matchMedia: dark/reduced are mutable, change listeners fire on `setDark`. */
function stubMedia(opts: { dark: boolean; reduced?: boolean }) {
  const listeners = new Set<Listener>();
  const state = { dark: opts.dark, reduced: opts.reduced ?? false };
  vi.stubGlobal("matchMedia", (q: string) => ({
    get matches() {
      return q.includes("prefers-color-scheme: dark")
        ? state.dark
        : q.includes("prefers-reduced-motion")
          ? state.reduced
          : false;
    },
    media: q,
    addEventListener: (_: string, l: Listener) => listeners.add(l),
    removeEventListener: (_: string, l: Listener) => listeners.delete(l),
  }));
  return {
    setDark(v: boolean) {
      state.dark = v;
      act(() => listeners.forEach((l) => l()));
    },
  };
}

const theme = () => document.documentElement.dataset.theme;
/**
 * The shell is ready when the start screen's hand-off has focused the destination. `findByRole("main")` alone
 * resolves at the DOM commit, before React's passive effects have run: the destination's route subscription
 * (`useSyncExternalStore`) and the hand-off focus. A test acting at that point races both (#274).
 */
const ready = async () => {
  const main = await screen.findByRole("main");
  await waitFor(() => expect(document.activeElement).toBe(main));
  return main;
};

beforeEach(() => {
  localStorage.clear();
  reloadTheme();
  navigate("today");
  stubMedia({ dark: true });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("rail", () => {
  test("is visible on places and hidden under ride and onboarding", async () => {
    render(<App />);
    await ready();
    expect(screen.getByRole("navigation", { name: "Places" })).toBeTruthy();
    for (const label of ["Today", "Workouts", "History", "Settings"])
      expect(
        screen.getByRole("button", { name: new RegExp(label) }),
      ).toBeTruthy();
    expect(
      screen
        .getByRole("button", { name: /Today/ })
        .getAttribute("aria-current"),
    ).toBe("page");

    for (const view of ["ride", "onboarding"] as const) {
      act(() => navigate(view));
      expect(screen.queryByRole("navigation", { name: "Places" })).toBeNull();
    }
    act(() => navigate("history"));
    expect(screen.getByRole("navigation", { name: "Places" })).toBeTruthy();
  });

  test("Devices is reachable under Settings", async () => {
    installDevices();
    const user = userEvent.setup();
    render(<App />);
    await ready();
    await user.click(screen.getByRole("button", { name: /Settings/ }));
    await user.click(screen.getByRole("button", { name: "Devices" }));
    expect(screen.getByRole("heading", { name: "Devices" })).toBeTruthy();
  });
});

describe("theme", () => {
  test("auto follows prefers-color-scheme live; a manual choice overrides and persists", async () => {
    const media = stubMedia({ dark: false });
    const user = userEvent.setup();
    render(<App />);
    await ready();
    expect(theme()).toBe("light");
    media.setDark(true);
    expect(theme()).toBe("dark");

    await user.click(screen.getByRole("button", { name: /Settings/ }));
    await user.click(screen.getByRole("radio", { name: "Light" }));
    expect(theme()).toBe("light");
    media.setDark(true);
    expect(theme()).toBe("light"); // the OS no longer matters
    expect(
      JSON.parse(localStorage.getItem("crank.theme.v1") ?? "null"),
    ).toEqual({ v: 1, pref: "light" });
  });

  test.each([
    "{",
    '"dark"',
    '{"v":2,"pref":"dark"}',
    '{"v":1,"pref":"neon"}',
    "null",
  ])(
    "an invalid saved value (%s) falls back to auto and says so",
    async (raw) => {
      localStorage.setItem("crank.theme.v1", raw);
      reloadTheme();
      render(<App />);
      await ready();
      act(() => navigate("settings", "appearance"));
      expect(theme()).toBe("dark"); // system is dark; not a crash
      expect(screen.getByRole("status").textContent).toMatch(/invalid/);
      expect(
        (screen.getByRole("radio", { name: /Auto/ }) as HTMLInputElement)
          .checked,
      ).toBe(true);
    },
  );

  test("a saved choice is applied on launch", async () => {
    localStorage.setItem(
      "crank.theme.v1",
      JSON.stringify({ v: 1, pref: "light" }),
    );
    reloadTheme();
    render(<App />);
    expect(theme()).toBe("light");
  });
});

describe("error boundary", () => {
  test("catches a thrown view, shows role=alert, Back to Today returns", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    const views: ViewRegistry = {
      today: () => <h1>Today view</h1>,
      workouts: () => {
        throw new Error("boom");
      },
      history: () => null,
      settings: () => null,
      ride: () => null,
      onboarding: () => null,
      resume: () => null,
    };
    const user = userEvent.setup();
    render(<App views={views} />);
    await ready();
    await user.click(screen.getByRole("button", { name: /Workouts/ }));
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toMatch(/boom/);
    await user.click(screen.getByRole("button", { name: "Back to Today" }));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByRole("heading", { name: "Today view" })).toBeTruthy();
    err.mockRestore();
  });
});

describe("keys dialog", () => {
  test("? opens it, Tab is trapped, Escape closes it and is consumed, focus returns", async () => {
    const user = userEvent.setup();
    const behind = vi.fn();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && behind();
    document.addEventListener("keydown", onKey);
    onTestFinished(() => document.removeEventListener("keydown", onKey));
    render(<App />);
    await ready();
    const today = screen.getByRole("button", { name: /Today/ });
    today.focus();

    await user.keyboard("?");
    const dialog = screen.getByRole("dialog", { name: "Keyboard" });
    const close = screen.getByRole("button", { name: "Close" });
    expect(document.activeElement).toBe(close);
    await user.tab();
    expect(dialog.contains(document.activeElement)).toBe(true);
    await user.tab({ shift: true });
    expect(dialog.contains(document.activeElement)).toBe(true);

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(behind).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(today);
  });

  test("? typed into a field does not open it", async () => {
    render(<App />);
    await ready();
    const field = document.body.appendChild(document.createElement("input"));
    field.focus();
    fireEvent.keyDown(field, { key: "?" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("start screen hand-off", () => {
  test("the destination is focused at once, the screen is inert, then removed after the fade", async () => {
    vi.useFakeTimers();
    render(<App />);
    await act(() => vi.advanceTimersByTimeAsync(0));
    const main = screen.getByRole("main");
    expect(document.activeElement).toBe(main);
    const start = screen.getByLabelText("crank is starting");
    expect(start.hasAttribute("inert")).toBe(true);
    await act(() => vi.advanceTimersByTimeAsync(HANDOFF_MS - 1));
    expect(screen.queryByLabelText("crank is starting")).not.toBeNull();
    await act(() => vi.advanceTimersByTimeAsync(1));
    expect(screen.queryByLabelText("crank is starting")).toBeNull();
  });

  test("prefers-reduced-motion removes it at once", async () => {
    stubMedia({ dark: true, reduced: true });
    vi.useFakeTimers();
    render(<App />);
    await act(() => vi.advanceTimersByTimeAsync(0));
    expect(screen.queryByLabelText("crank is starting")).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole("main"));
  });

  test("a slow boot shows the status line at 1.2 s, a fast one never does; ready lands within the budget", async () => {
    vi.useFakeTimers();
    let resolve!: () => void;
    const slow = () => new Promise<void>((r) => (resolve = r));
    render(<App waitForCore={slow} />);
    await act(() => vi.advanceTimersByTimeAsync(STATUS_AFTER_MS - 1));
    expect(screen.queryByRole("status")).toBeNull();
    await act(() => vi.advanceTimersByTimeAsync(1));
    expect(screen.getByRole("status").textContent).toMatch(/Starting crank/);
    await act(async () => resolve());
    expect(document.activeElement).toBe(screen.getByRole("main"));
    expect(screen.queryByRole("status")).toBeNull();
  });

  test("a failed boot shows role=alert with Try again, which starts a fresh attempt", async () => {
    let calls = 0;
    const wait = () =>
      ++calls === 1
        ? Promise.reject(new Error("its saved state could not be opened"))
        : Promise.resolve();
    const user = userEvent.setup();
    render(<App waitForCore={wait} />);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(
      /can’t start.*saved state could not be opened/,
    );
    expect(screen.queryByRole("main")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(screen.getByRole("main")).toBeTruthy());
    expect(calls).toBe(2);
  });
});

describe("a11y", () => {
  test("Skip to main moves focus to main", async () => {
    const user = userEvent.setup();
    render(<App />);
    await ready();
    await user.click(screen.getByRole("link", { name: "Skip to main" }));
    expect(document.activeElement).toBe(screen.getByRole("main"));
  });
});
