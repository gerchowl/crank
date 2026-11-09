// @vitest-environment jsdom
import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { WorkoutBlock } from "../bindings";
import { Timeline } from "./Timeline";

// jsdom has no layout: give `useFit` a 1000 px strip (a label's box is its % width × 10) and 7 px per character, so
// a label sheds as it would in the browser.
beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  Object.defineProperty(document, "fonts", {
    configurable: true,
    value: { ready: Promise.resolve() },
  });
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
    font: "",
    measureText: (s: string) => ({ width: s.length * 7 }),
  } as unknown as CanvasRenderingContext2D);
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(
    function (this: HTMLElement) {
      return parseFloat(this.style.width || "0") * 10;
    },
  );
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const block = (
  phase: WorkoutBlock["phase"],
  startS: number,
  durS: number,
  w: number,
): WorkoutBlock => ({
  path: [],
  iteration: [],
  nodeId: `${phase}-${startS}`,
  startS,
  durS,
  powerFrom: w,
  powerTo: w,
  phase,
  durRangeS: null,
});

// DESIGN §18.3 "Strip labels" (#581): a block too short for its watts draws a leader and keeps its label for a screen reader.
test("a 30 s block in an hour is a leader with its full label in the accessible text", () => {
  const { container } = render(
    <Timeline
      blocks={[block("work", 0, 30, 400), block("work", 30, 3570, 250)]}
      totalS={3600}
      workoutS={0}
    />,
  );
  const [short, long] = [...container.querySelectorAll(".timeline-label")];
  expect(short?.querySelector(".timeline-leader")).not.toBeNull();
  expect(short?.querySelector(".sr-only")?.textContent).toBe("Work 400 W");
  // The leader is all it draws: no visible text beside the accessible label.
  expect(
    short?.textContent?.replace(
      short.querySelector(".sr-only")?.textContent ?? "",
      "",
    ),
  ).toBe("");
  // A block wide enough keeps its text and draws no leader.
  expect(long?.textContent).toBe("Work 250 W");
  expect(long?.querySelector(".timeline-leader")).toBeNull();
});
