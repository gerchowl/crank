// @vitest-environment jsdom
import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, test } from "vitest";
import { Devices } from "./Devices";
import { both, installDevices, strap, trainer } from "./devicesBackend";

afterEach(() => {
  cleanup();
  clearMocks();
});

const scan = async (user: ReturnType<typeof userEvent.setup>) =>
  user.click(await screen.findByRole("button", { name: "Scan" }));

describe("Devices", () => {
  test("scan lists the candidates with their role", async () => {
    installDevices({ nearby: [trainer, strap] });
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    const list = await screen.findByRole("list", { name: "Nearby devices" });
    expect(list.textContent).toContain("Tacx Neo 55624");
    expect(list.textContent).toContain("Trainer");
    expect(list.textContent).toContain("HRM-Dual");
    expect(list.textContent).toContain("Heart-rate strap");
  });

  test("an empty scan says so", async () => {
    installDevices();
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    expect(await screen.findByText(/No trainer or strap found/)).toBeTruthy();
  });

  test("pair shows the paired state; forget removes it", async () => {
    const st = installDevices({ nearby: [trainer] });
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    await user.click(
      await screen.findByRole("button", {
        name: "Pair Tacx Neo 55624 as trainer",
      }),
    );
    expect(await screen.findByText(/S\/N 55624/)).toBeTruthy();
    expect(st.calls.find((c) => c.cmd === "device_pair")?.args).toEqual({
      id: "neo-1",
      role: "trainer",
    });
    expect(screen.getByText(/· Paired/)).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Forget trainer" }));
    await waitFor(() =>
      expect(screen.getAllByText("Not paired").length).toBe(2),
    );
    expect(st.paired.trainer).toBeUndefined();
  });

  test("starts from the core's pairings", async () => {
    installDevices({
      paired: {
        heart_rate: {
          role: "heart_rate",
          id: "hrm-1",
          name: "HRM-Dual",
          serial: null,
          model: null,
        },
      },
    });
    render(<Devices />);
    expect(await screen.findByText("HRM-Dual")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Forget heart-rate strap" }),
    ).toBeTruthy();
  });

  test("permission denied: the macOS path and Try again re-scans", async () => {
    const st = installDevices({
      nearby: [trainer],
      adapter: { code: "permission_denied", message: "denied" },
    });
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(
      "Open System Settings → Privacy & Security → Bluetooth → crank",
    );
    st.adapter = null;
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Tacx Neo 55624")).toBeTruthy();
    expect(st.calls.filter((c) => c.cmd === "devices_scan").length).toBe(2);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  test("adapter unavailable shows Bluetooth off", async () => {
    installDevices({
      adapter: { code: "device_unavailable", message: "Bluetooth is off" },
    });
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Bluetooth off",
    );
  });

  test("a failed connect names the device and hints at another app", async () => {
    installDevices({
      nearby: [trainer],
      connect: { code: "device_unavailable", message: "connect failed" },
    });
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    await user.click(
      await screen.findByRole("button", {
        name: "Pair Tacx Neo 55624 as trainer",
      }),
    );
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Could not connect to Tacx Neo 55624");
    expect(alert.textContent).toContain("another app");
    expect(alert.textContent).not.toContain("Bluetooth off");
    expect(screen.getAllByText("Not paired").length).toBe(2);
  });

  test("a loaded ride is a conflict the rider can act on", async () => {
    installDevices({ rideLoaded: true });
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    expect((await screen.findByRole("alert")).textContent).toContain(
      "End the ride",
    );
  });

  test("a role the device cannot act in is not offered (the core refuses it)", async () => {
    installDevices({ nearby: [strap] });
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    await screen.findByText("HRM-Dual");
    expect(screen.queryByRole("button", { name: /as trainer/ })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Pair HRM-Dual as heart-rate strap" }),
    ).toBeTruthy();
  });

  test("buttons are at least 48 px", () => {
    const css = readFileSync("src/shell.css", "utf8");
    expect(css).toMatch(/\.devices button \{[^}]*min-height:\s*48px/);
  });

  test("a failed re-read keeps the last good list; an initial failure is shown", async () => {
    const st = installDevices({
      nearby: [trainer],
      paired: {
        trainer: {
          role: "trainer",
          id: "neo-1",
          name: "Tacx Neo 55624",
          serial: null,
          model: null,
        },
      },
    });
    const user = userEvent.setup();
    render(<Devices />);
    expect(await screen.findByText("Tacx Neo 55624")).toBeTruthy();
    st.linksError = { code: "storage", message: "disk" };
    await user.click(screen.getByRole("button", { name: "Forget trainer" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Could not load the paired devices",
    );
    // The core forgot it, but the read failed: the last good list is still on screen, not "Loading…".
    expect(screen.queryByText("Loading…")).toBeNull();
    expect(screen.getByText("Tacx Neo 55624")).toBeTruthy();
  });

  test("a failed first load is shown, not left on Loading", async () => {
    installDevices({ linksError: { code: "storage", message: "disk" } });
    render(<Devices />);
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Could not load the paired devices",
    );
    expect(screen.queryByText("Loading…")).toBeNull();
  });

  test("a double click pairs once; a double click scans once", async () => {
    let open!: () => void;
    const st = installDevices({
      nearby: [trainer],
      gate: new Promise<void>((r) => (open = r)),
    });
    const user = userEvent.setup();
    render(<Devices />);
    await user.dblClick(await screen.findByRole("button", { name: "Scan" }));
    open();
    await screen.findByText("Tacx Neo 55624");
    expect(st.calls.filter((c) => c.cmd === "devices_scan").length).toBe(1);

    let openPair!: () => void;
    st.gate = new Promise<void>((r) => (openPair = r));
    await user.dblClick(
      screen.getByRole("button", { name: "Pair Tacx Neo 55624 as trainer" }),
    );
    openPair();
    await screen.findByText(/S\/N 55624/);
    expect(st.calls.filter((c) => c.cmd === "device_pair").length).toBe(1);
  });

  test("results are announced politely", async () => {
    installDevices();
    const user = userEvent.setup();
    const { container } = render(<Devices />);
    await scan(user);
    const live = container.querySelector('[aria-live="polite"]');
    expect(live?.textContent).toContain("No trainer or strap found");
  });

  test("a dual-role device shows Pairing… only on the role in flight", async () => {
    let open!: () => void;
    const st = installDevices({ nearby: [both] });
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    await screen.findByText("Dual Thing");
    st.gate = new Promise<void>((r) => (open = r));
    await user.click(
      await screen.findByRole("button", { name: "Pair Dual Thing as trainer" }),
    );
    await waitFor(() => expect(screen.getAllByText("Pairing…").length).toBe(1));
    expect(
      screen.getByRole("button", { name: "Pair Dual Thing as trainer" })
        .textContent,
    ).toBe("Pairing…");
    expect(
      screen.getByRole("button", {
        name: "Pair Dual Thing as heart-rate strap",
      }).textContent,
    ).toBe("Pair as strap");
    open();
    await screen.findByText(/S\/N 55624/);
    expect(st.paired.trainer?.id).toBe("both-1");
  });

  test("the radio core fails an unfit role at connect; the sim refuses it up front", async () => {
    // Not offered by the UI, but the contract both cores keep: no reliance on the sim's early refusal.
    const radio = installDevices({ nearby: [strap] });
    const { commands } = await import("../bindings");
    const r = await commands.devicePair("hrm-1", "trainer");
    expect(r.status === "error" && r.error).toMatchObject({
      code: "device_unavailable",
    });
    radio.core = "sim";
    const s = await commands.devicePair("hrm-1", "trainer");
    expect(s.status === "error" && s.error).toMatchObject({
      code: "invalid_input",
    });
  });
});
