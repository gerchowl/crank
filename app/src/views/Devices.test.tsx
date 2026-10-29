// @vitest-environment jsdom
import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, test } from "vitest";
import type { PairedDevice } from "../bindings";
import { Devices } from "./Devices";
import {
  both,
  installDevices,
  simStrap,
  simTrainer,
  strap,
  trainer,
} from "./devicesBackend";

const neo: PairedDevice = {
  role: "trainer",
  id: "neo-1",
  name: "Tacx Neo 55624",
  serial: null,
  model: null,
};

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
        heart_rate: {
          role: "heart_rate",
          id: "hrm-1",
          name: "HRM-Dual",
          serial: null,
          model: null,
        },
      },
    });
    const user = userEvent.setup();
    render(<Devices />);
    expect(await screen.findByText("HRM-Dual")).toBeTruthy();
    await scan(user);
    st.linksError = { code: "storage", message: "disk" };
    await user.click(
      await screen.findByRole("button", {
        name: "Pair Tacx Neo 55624 as trainer",
      }),
    );
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Could not load the paired devices",
    );
    // The core paired it, but the read failed: the last good list is still on screen, not "Loading…".
    expect(screen.queryByText("Loading…")).toBeNull();
    expect(screen.getByText("HRM-Dual")).toBeTruthy();
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

  test("Scan, Pair and Forget are disabled while a command is in flight", async () => {
    let open!: () => void;
    const st = installDevices({ nearby: [trainer], paired: { trainer: neo } });
    const user = userEvent.setup();
    render(<Devices />);
    await screen.findByRole("button", { name: "Forget trainer" });
    // A scan in flight.
    st.gate = new Promise<void>((r) => (open = r));
    await user.click(screen.getByRole("button", { name: "Scan" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Scanning…" })).toHaveProperty(
        "disabled",
        true,
      ),
    );
    expect(
      screen.getByRole("button", { name: "Forget trainer" }),
    ).toHaveProperty("disabled", true);
    open();
    const pairBtn = () =>
      screen.findByRole("button", { name: "Pair Tacx Neo 55624 as trainer" });
    await pairBtn();
    expect(screen.getByRole("button", { name: "Scan again" })).toHaveProperty(
      "disabled",
      false,
    );
    // A pair in flight.
    st.gate = new Promise<void>((r) => (open = r));
    await user.click(await pairBtn());
    await waitFor(() => expect(screen.getByText("Pairing…")).toBeTruthy());
    expect(await pairBtn()).toHaveProperty("disabled", true);
    expect(screen.getByRole("button", { name: "Scan again" })).toHaveProperty(
      "disabled",
      true,
    );
    expect(
      screen.getByRole("button", { name: "Forget trainer" }),
    ).toHaveProperty("disabled", true);
    open();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Scan again" })).toHaveProperty(
        "disabled",
        false,
      ),
    );
    // A forget in flight.
    st.gate = new Promise<void>((r) => (open = r));
    await user.click(screen.getByRole("button", { name: "Forget trainer" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Scan again" })).toHaveProperty(
        "disabled",
        true,
      ),
    );
    expect(await pairBtn()).toHaveProperty("disabled", true);
    open();
    await waitFor(() =>
      expect(screen.getAllByText("Not paired").length).toBe(2),
    );
  });

  test("the sim pairs only its fixed ids; the mock refuses the others like the core", async () => {
    const st = installDevices({
      core: "sim",
      nearby: [simTrainer, simStrap, trainer],
    });
    const { commands } = await import("../bindings");
    for (const [id, role] of [
      ["neo-1", "trainer"],
      ["strap", "trainer"],
      ["trainer", "heart_rate"],
      ["hrm-1", "heart_rate"],
    ] as const) {
      const r = await commands.devicePair(id, role);
      expect(r.status === "error" && r.error).toMatchObject({
        code: "invalid_input",
      });
    }
    expect(st.paired).toEqual({});
    const ok = await commands.devicePair("trainer", "trainer");
    expect(ok.status).toBe("ok");
    expect(st.paired.trainer?.id).toBe("trainer");
  });

  test("a store failure on scan is shown with Try again", async () => {
    const st = installDevices({
      nearby: [trainer],
      store: { code: "storage", message: "disk is full" },
    });
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("the pairing store failed");
    expect(alert.textContent).toContain("disk is full");
    st.store = null;
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Tacx Neo 55624")).toBeTruthy();
  });

  test("a store failure on pair says nothing is paired", async () => {
    const st = installDevices({ nearby: [trainer] });
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    st.store = { code: "storage", message: "disk is full" };
    await user.click(
      await screen.findByRole("button", {
        name: "Pair Tacx Neo 55624 as trainer",
      }),
    );
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Could not save the pairing");
    expect(alert.textContent).not.toContain("nothing is paired");
    expect(st.paired).toEqual({});
    expect(screen.getAllByText("Not paired").length).toBe(2);
  });

  test("a store failure on forget keeps the pairing and says why", async () => {
    const st = installDevices({ paired: { trainer: neo } });
    const user = userEvent.setup();
    render(<Devices />);
    await screen.findByText("Tacx Neo 55624");
    st.store = { code: "storage", message: "disk is full" };
    await user.click(screen.getByRole("button", { name: "Forget trainer" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Could not forget trainer");
    expect(alert.textContent).toContain("disk is full");
    expect(st.paired.trainer).toBeDefined();
    expect(screen.getByText("Tacx Neo 55624")).toBeTruthy();
  });

  test("a forget whose refresh fails does not leave the row paired", async () => {
    const st = installDevices({ paired: { trainer: neo } });
    const user = userEvent.setup();
    render(<Devices />);
    await screen.findByText("Tacx Neo 55624");
    st.linksError = { code: "storage", message: "disk" };
    await user.click(screen.getByRole("button", { name: "Forget trainer" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Could not load the paired devices",
    );
    expect(st.paired.trainer).toBeUndefined();
    // The forget succeeded, so the row says so although the re-read failed: no second Forget to offer.
    expect(screen.queryByRole("button", { name: "Forget trainer" })).toBeNull();
    expect(screen.queryByText("Tacx Neo 55624")).toBeNull();
  });

  test("not_found on forget is success: the row is cleared, with no notice", async () => {
    const st = installDevices({ paired: { trainer: neo } });
    const user = userEvent.setup();
    render(<Devices />);
    await screen.findByText("Tacx Neo 55624");
    delete st.paired.trainer; // forgotten elsewhere since the last read
    st.linksError = { code: "storage", message: "disk" };
    await user.click(screen.getByRole("button", { name: "Forget trainer" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "Forget trainer" }),
      ).toBeNull(),
    );
    expect(st.calls.filter((c) => c.cmd === "device_forget").length).toBe(1);
    const alerts = screen.getAllByRole("alert");
    expect(alerts.length).toBe(1);
    expect(alerts[0]?.textContent).not.toContain("Nothing to forget");
  });

  test("when forget and the refresh both fail, both errors are shown", async () => {
    const st = installDevices({ paired: { trainer: neo } });
    const user = userEvent.setup();
    render(<Devices />);
    await screen.findByText("Tacx Neo 55624");
    st.store = { code: "storage", message: "disk is full" };
    st.linksError = { code: "storage", message: "disk is full" };
    await user.click(screen.getByRole("button", { name: "Forget trainer" }));
    await waitFor(() => expect(screen.getAllByRole("alert").length).toBe(2));
    const text = screen.getAllByRole("alert").map((a) => a.textContent);
    expect(text[0]).toContain("Could not forget trainer");
    expect(text[1]).toContain("Could not load the paired devices");
    // The forget did not happen, so the pairing stays on screen.
    expect(screen.getByText("Tacx Neo 55624")).toBeTruthy();
  });

  test("a pair that fails in the store after writing is re-read: the list shows the truth", async () => {
    const st = installDevices({ nearby: [trainer] });
    const user = userEvent.setup();
    render(<Devices />);
    await scan(user);
    st.storeAfterWrite = { code: "storage", message: "disk is full" };
    await user.click(
      await screen.findByRole("button", {
        name: "Pair Tacx Neo 55624 as trainer",
      }),
    );
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Could not save the pairing",
    );
    // The core wrote the row before it failed: the re-read shows it paired.
    expect(await screen.findByText(/S\/N 55624/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Forget trainer" })).toBeTruthy();
    expect(st.calls.filter((c) => c.cmd === "device_links").length).toBe(2);
  });

  test("the store is checked where the core checks it: pair after the connect, forget before not_found", async () => {
    const st = installDevices({
      nearby: [trainer],
      store: { code: "storage", message: "disk" },
      connect: { code: "device_unavailable", message: "connect failed" },
    });
    const { commands } = await import("../bindings");
    // Pair connects first (core.rs:621-650), so a connect failure wins over the store.
    const p = await commands.devicePair("neo-1", "trainer");
    expect(p.status === "error" && p.error.code).toBe("device_unavailable");
    st.connect = null;
    const q = await commands.devicePair("neo-1", "trainer");
    expect(q.status === "error" && q.error.code).toBe("storage");
    // Forget reads the store before it can find nothing (core.rs:684-696).
    const f = await commands.deviceForget("trainer");
    expect(f.status === "error" && f.error.code).toBe("storage");
    st.store = null;
    const g = await commands.deviceForget("trainer");
    expect(g.status === "error" && g.error.code).toBe("not_found");
  });
});
