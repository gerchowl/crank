// @vitest-environment jsdom
import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, test } from "vitest";
import { Devices } from "./Devices";
import { installDevices, strap, trainer } from "./devicesBackend";

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
});
