// @vitest-environment jsdom
// Settings → Athlete model (ADR-0009 §2.1.3, §2.8 "App", #716): edit, history and revert, clear, proposals, on the
// mock of the core's athlete commands (athleteBackend.ts, which models each of their refusals).
import { clearMocks } from "@tauri-apps/api/mocks";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, test } from "vitest";
import { forgetAthleteModel } from "../athleteHr";
import { commands, type AthleteParam } from "../bindings";
import { NO_HR_REFS } from "./Athlete";
import { estimate, install, type AthleteState } from "./athleteBackend";
import { Settings } from "./Settings";

afterEach(() => {
  cleanup();
  clearMocks();
  forgetAthleteModel();
});

const NOW = Math.floor(Date.now() / 1000);
const DAY = 86_400;

async function open(over: Partial<AthleteState> = {}) {
  const b = install({ now: NOW, ...over });
  render(<Settings sub="athlete" />);
  await screen.findByRole("list", { name: "Parameters" });
  return b;
}
const row = (label: string) => screen.getByRole("listitem", { name: label });
const value = (param: AthleteParam) =>
  screen.getByTestId(`athlete-${param}`).textContent;
const source = (param: AthleteParam) =>
  screen.getByTestId(`athlete-${param}-source`).textContent;
const sets = (b: { calls: { cmd: string; args: unknown }[] }) =>
  b.calls.filter((c) => c.cmd === "athlete_set").map((c) => c.args);

describe("Settings → Athlete model: what each row says", () => {
  test("a fresh store: FTP is the prior, said as an estimate; the rest not set; the HR hint shows", async () => {
    const b = await open();
    // The section and its HR hint share one read.
    expect(b.calls.filter((c) => c.cmd === "athlete_model")).toHaveLength(1);
    expect(value("ftp")).toBe("estimate: 250 W (no test yet)");
    expect(source("ftp")).toBe("not measured yet");
    expect(value("w_prime")).toBe("estimate: 20.0 kJ (no test yet)");
    expect(value("lthr")).toBe("Not set");
    expect(value("weight")).toBe("Not set");
    expect(source("weight")).toBe("not measured yet");
    // An unset value never reads 0.
    expect(screen.queryByText(/\b0 (W|bpm|kg)\b/)).toBeNull();
    expect(screen.getByText(NO_HR_REFS)).toBeTruthy();
    // There is nothing to clear on a prior or an unset value.
    for (const label of ["FTP", "Weight"])
      expect(
        within(row(label)).getByRole<HTMLButtonElement>("button", {
          name: `Clear ${label}`,
        }).disabled,
      ).toBe(true);
    expect(screen.getByText(/^None\. When a test or your rides/)).toBeTruthy();
  });

  test("a measured value: ± sd, its source in words, its day and age; over 6 weeks it is stale", async () => {
    await open({
      rows: [
        estimate(1, "ftp", 287, {
          sd: 12,
          source: "test",
          method: "ramp_v1_0.75",
          updatedAt: NOW - 3 * DAY,
        }),
        estimate(2, "weight", 72.5, { sd: 1, updatedAt: NOW - 60 * DAY }),
      ],
    });
    expect(value("ftp")).toBe("287 W ± 12");
    expect(source("ftp")).toMatch(/^ramp test, \w+ \d+ · 3 days ago$/);
    expect(source("ftp")).not.toContain("stale");
    expect(value("weight")).toBe("72.5 kg ± 1.0");
    expect(source("weight")).toMatch(/^typed in, .+ · 8 weeks ago · stale$/);
  });

  test("an HR max prior from the age is an estimate, and is no zone reference: the hint stays", async () => {
    await open({ age: 40 });
    expect(value("hr_max")).toBe("estimate: 180 bpm (from your age)");
    expect(screen.getByText(NO_HR_REFS)).toBeTruthy();
  });

  test("a measured LTHR: the hint goes", async () => {
    await open({ rows: [estimate(1, "lthr", 165)] });
    expect(value("lthr")).toBe("165 bpm ± 33");
    expect(screen.queryByText(NO_HR_REFS)).toBeNull();
  });

  test("open proposals are listed at the top (#720 raises them)", async () => {
    await open({
      proposals: [
        {
          id: 4,
          rideId: "r1",
          method: "cp_ols_v1",
          values: [
            { param: "cp", value: 265, sd: 8 },
            { param: "w_prime", value: 18500, sd: 2000 },
          ],
          createdAt: NOW,
        },
      ],
    });
    const [item] = document.querySelectorAll(".athlete-proposals li");
    expect(item.firstElementChild?.textContent).toBe(
      "Critical power (CP) 265 W ± 8, W′ (work above CP) 18.5 kJ ± 2.0",
    );
    expect(item.lastElementChild?.textContent).toMatch(/^suggested /);
  });

  test("a failed read: the notice, and Try again reads again", async () => {
    const b = install({
      now: NOW,
      store: { code: "storage", message: "database is locked" },
    });
    render(<Settings sub="athlete" />);
    expect((await screen.findByRole("alert")).textContent).toContain(
      "database is locked",
    );
    b.state.store = null;
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByRole("list", { name: "Parameters" });
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("Settings → Athlete model: Edit", () => {
  const edit = async (label: string) => {
    await userEvent.click(
      within(row(label)).getByRole("button", { name: `Edit ${label}` }),
    );
  };

  test("typing an FTP and Save sets it (sd by default) and the row reads it from the model again", async () => {
    const b = await open();
    await edit("FTP");
    await userEvent.type(
      screen.getByRole("textbox", { name: "FTP, W" }),
      "300",
    );
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(value("ftp")).toBe("300 W ± 60"));
    expect(sets(b)).toEqual([{ param: "ftp", value: 300, sd: null }]);
    expect(source("ftp")).toMatch(/^typed in, .+ · today$/);
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  test("an ± is sent in the stored unit: W′ is typed in kJ", async () => {
    const b = await open();
    await edit("W′ (work above CP)");
    await userEvent.type(
      screen.getByRole("textbox", { name: "W′ (work above CP), kJ" }),
      "22,5",
    );
    await userEvent.type(
      screen.getByRole("textbox", { name: "± kJ, optional" }),
      "2",
    );
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(value("w_prime")).toBe("22.5 kJ ± 2.0"));
    expect(sets(b)).toEqual([{ param: "w_prime", value: 22500, sd: 2000 }]);
  });

  test("the draft opens on the measured value; Cancel sends nothing and the value stays", async () => {
    const b = await open({ rows: [estimate(1, "ftp", 280)] });
    await edit("FTP");
    const box = screen.getByRole<HTMLInputElement>("textbox", {
      name: "FTP, W",
    });
    expect(box.value).toBe("280");
    await userEvent.clear(box);
    await userEvent.type(box, "310");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(sets(b)).toEqual([]);
    expect(value("ftp")).toBe("280 W ± 56");
  });

  test("something that is no number is not sent", async () => {
    const b = await open();
    await edit("Weight");
    await userEvent.type(
      screen.getByRole("textbox", { name: "Weight, kg" }),
      "abc",
    );
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByRole("alert").textContent).toContain(
      "Type the value in kg.",
    );
    expect(sets(b)).toEqual([]);
  });

  test.each([
    [
      "Threshold heart rate (LTHR), bpm",
      "230",
      "lthr: value must be within 100–220",
    ],
    [
      "Threshold heart rate (LTHR), bpm",
      "195",
      "LTHR 195 must be below HR max 190",
    ],
  ])(
    "the core's refusal stays in the open draft, and the model is unchanged (%s %s)",
    async (box, typed, why) => {
      const b = await open({ rows: [estimate(1, "hr_max", 190)] });
      await edit("Threshold heart rate (LTHR)");
      await userEvent.type(screen.getByRole("textbox", { name: box }), typed);
      await userEvent.click(screen.getByRole("button", { name: "Save" }));
      await waitFor(() =>
        expect(screen.getByRole("alert").textContent).toContain(
          `Not saved: ${why}`,
        ),
      );
      expect(screen.getByRole("textbox", { name: box })).toBeTruthy();
      expect(b.state.rows).toHaveLength(1);
      await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
      expect(value("lthr")).toBe("Not set");
    },
  );

  test("an ± above half the value is refused by the core", async () => {
    await open();
    await edit("FTP");
    await userEvent.type(
      screen.getByRole("textbox", { name: "FTP, W" }),
      "300",
    );
    await userEvent.type(
      screen.getByRole("textbox", { name: "± W, optional" }),
      "200",
    );
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain(
        "ftp: sd must be above 0 and at most 50 % of the value",
      ),
    );
  });
});

describe("Settings → Athlete model: History, Use this value, Clear", () => {
  const history = async (label: string) => {
    await userEvent.click(
      within(row(label)).getByRole("button", { name: `History of ${label}` }),
    );
    return screen.findByRole("list", { name: `${label} history` });
  };

  test("History lists every value newest first; Use this value reverts, and both lists read again", async () => {
    const b = await open({
      rows: [
        estimate(1, "ftp", 280, { updatedAt: NOW - 10 * DAY }),
        estimate(2, "ftp", 300, { updatedAt: NOW - DAY }),
      ],
    });
    const list = await history("FTP");
    const items = within(list).getAllByRole("listitem");
    expect(
      items.map((i) => i.querySelector(".athlete-history-value")?.textContent),
    ).toEqual(["300 W ± 60", "280 W ± 56"]);
    expect(within(items[0]).getByText("Current")).toBeTruthy();
    await userEvent.click(
      within(items[1]).getByRole("button", { name: "Use this value" }),
    );
    await waitFor(() => expect(value("ftp")).toBe("280 W ± 56"));
    expect(
      b.calls.filter((c) => c.cmd === "athlete_revert").map((c) => c.args),
    ).toEqual([{ estimateId: 1 }]);
    await waitFor(() =>
      expect(
        within(screen.getByRole("list", { name: "FTP history" })).getAllByRole(
          "listitem",
        ),
      ).toHaveLength(3),
    );
  });

  test("Clear of FTP goes back to the prior; of LTHR to unset; History names both and offers no revert to either", async () => {
    await open({ rows: [estimate(1, "ftp", 300), estimate(2, "lthr", 165)] });
    await userEvent.click(
      within(row("FTP")).getByRole("button", { name: "Clear FTP" }),
    );
    await waitFor(() =>
      expect(value("ftp")).toBe("estimate: 250 W (no test yet)"),
    );
    await userEvent.click(
      within(row("Threshold heart rate (LTHR)")).getByRole("button", {
        name: "Clear Threshold heart rate (LTHR)",
      }),
    );
    await waitFor(() => expect(value("lthr")).toBe("Not set"));
    const ftp = within(await history("FTP")).getAllByRole("listitem");
    expect(ftp[0].textContent).toContain("cleared to the estimate");
    expect(
      within(ftp[1]).getByRole("button", { name: "Use this value" }),
    ).toBeTruthy();
    const lthr = within(
      await history("Threshold heart rate (LTHR)"),
    ).getAllByRole("listitem");
    expect(lthr[0].textContent).toContain("Cleared");
    expect(within(lthr[0]).queryByRole("button")).toBeNull();
  });

  test("a revert the model refuses is the row's notice", async () => {
    // An LTHR of 185 from before a measured HR max of 180: going back to it would put it above HR max.
    await open({
      rows: [
        estimate(1, "lthr", 185, { updatedAt: NOW - 20 * DAY }),
        estimate(2, "lthr", 160, { updatedAt: NOW - 10 * DAY }),
        estimate(3, "hr_max", 180, { updatedAt: NOW - 5 * DAY }),
      ],
    });
    const items = within(
      await history("Threshold heart rate (LTHR)"),
    ).getAllByRole("listitem");
    await userEvent.click(
      within(items[1]).getByRole("button", { name: "Use this value" }),
    );
    await waitFor(() =>
      expect(
        within(row("Threshold heart rate (LTHR)")).getByRole("alert")
          .textContent,
      ).toContain("LTHR 185 must be below HR max 180"),
    );
    expect(value("lthr")).toBe("160 bpm ± 32");
  });

  test("an empty history says so", async () => {
    await open();
    await userEvent.click(
      within(row("Weight")).getByRole("button", { name: "History of Weight" }),
    );
    expect(await screen.findByText(/^Nothing yet/)).toBeTruthy();
  });
});

describe("the mock refuses what the core refuses (athleteBackend.ts)", () => {
  test.each([
    [
      "athlete_set unknown param",
      () => commands.athleteSet("vo2", 50, null),
      "invalid_input",
    ],
    [
      "athlete_set non-finite",
      () => commands.athleteSet("ftp", Number.NaN, null),
      "invalid_input",
    ],
    [
      "athlete_set below bounds",
      () => commands.athleteSet("ftp", 49, null),
      "invalid_input",
    ],
    [
      "athlete_set sd 0",
      () => commands.athleteSet("ftp", 250, 0),
      "invalid_input",
    ],
    [
      "athlete_history unknown",
      () => commands.athleteHistory("vo2"),
      "invalid_input",
    ],
    [
      "athlete_clear unknown",
      () => commands.athleteClear("vo2"),
      "invalid_input",
    ],
    [
      "athlete_revert unknown row",
      () => commands.athleteRevert(99),
      "not_found",
    ],
  ] as const)("%s", async (_, call, code) => {
    install({ now: NOW });
    const r = await call();
    expect(r.status).toBe("error");
    if (r.status === "error")
      expect((r.error as { code: string }).code).toBe(code);
  });

  test("a revert to a prior row or a cleared row is invalid input", async () => {
    install({ now: NOW });
    await commands.athleteClear("ftp");
    await commands.athleteClear("weight");
    for (const id of [1, 2]) {
      const r = await commands.athleteRevert(id);
      expect(r.status === "error" && (r.error as { code: string }).code).toBe(
        "invalid_input",
      );
    }
  });

  test("an HR max at or below a measured LTHR is refused; a prior HR max never refuses an LTHR", async () => {
    install({ now: NOW, age: 60, rows: [estimate(1, "lthr", 170)] });
    const r = await commands.athleteSet("hr_max", 170, null);
    expect(r.status === "error" && (r.error as { code: string }).code).toBe(
      "invalid_input",
    );
    // The age's prior is 166 bpm, below the LTHR: it is no measurement, so nothing is refused.
    expect((await commands.athleteSet("lthr", 175, null)).status).toBe("ok");
  });
});
