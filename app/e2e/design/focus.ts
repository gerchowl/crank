// The focus walk (#458, DESIGN §18.7 rule 8, §6.7, §18.4): Tab from the document start until focus comes back to the
// first stop, read each stop with focus-audit.js, then Shift+Tab the same cycle back and use the skip link. Three rules,
// counted like the layout audit's so the warn baseline (baseline.ts) holds them:
//   focus.ring   a stop without a visible ring: no change on focus, under 2 px, off screen, or under 3 : 1 against
//                its background (focus-audit.js).
//   focus.order  a step against the reading order (below), a Shift+Tab stop that is not the forward cycle reversed,
//                and the skip link: it is the first stop, Enter moves focus to its target (`#main`), and the next Tab
//                reaches the first stop inside the target, if it has one.
//   focus.trap   focus lost mid-cycle (to `body`; once next to the first stop is the document's edge, where a window
//                hands focus to the browser), a stop that is no control (focus-audit.js `kindOf`), and a cycle that never
//                comes back to the first stop (a loop elsewhere, or no end within MAX_STOPS).
// Reading order: rows top to bottom, then left to right; a later column (right of the stop before, with no horizontal
// overlap) may start higher. So a step from A to B is in order when B is on A's row and not left of it, when B starts
// lower than A, or when B is higher but wholly right of A. Two boxes share a row when their vertical overlap is at
// least half the shorter one's height, and "left of" and "right of" allow ORDER_TOLERANCE px (sub-pixel layout, borders).
import { join } from "node:path";
import type { Page } from "@playwright/test";
import type { Counts } from "./baseline";

const PROBE = join(import.meta.dirname, "focus-audit.js");
/** More Tab stops than any screen has: a walk that has not cycled by then is trapped. */
const MAX_STOPS = 200;
/** px a same-row or next-column box may sit left of where reading order wants it. */
export const ORDER_TOLERANCE = 4;
/** Offenders kept per rule, as the layout audit keeps them. */
const TOP = 3;

interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}
interface Probe {
  previous: { sel: string; fault: string | null } | null;
  kind: "lost" | "control" | "other";
  id: number;
  sel: string;
  box?: Box;
  skipTo?: string | null;
}

declare global {
  interface Window {
    __focusStart(): void;
    __focusProbe(): Probe;
    __focusWithin(
      selector: string,
      order: number[],
    ): { on: boolean; first: number };
  }
}

/** Whether a Tab from `a` to `b` follows the reading order (header comment). */
export function inOrder(a: Box, b: Box): boolean {
  const overlap = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
  const shorter = Math.min(a.bottom - a.top, b.bottom - b.top);
  if (overlap >= shorter / 2) return b.left >= a.left - ORDER_TOLERANCE;
  if (b.top >= a.top) return true;
  return b.left >= a.right - ORDER_TOLERANCE;
}

/** Walk the page's Tab cycle and count the focus rules' violations, with the worst offenders per rule. */
export async function focusWalk(page: Page) {
  const rules: Record<string, string[]> = {};
  const add = (rule: string, msg: string) => (rules[rule] ??= []).push(msg);

  await page.addScriptTag({ path: PROBE });
  // The fixtures click to reach their state; a pointer resting on a control would hold its hover style.
  await page.mouse.move(0, 0);
  await page.evaluate(() => window.__focusStart());

  /** Press `key` until focus is back on the first stop: the stops in between, and whether the cycle closed. */
  const cycle = async (key: string, first?: Probe) => {
    const stops: Probe[] = [];
    let lost = 0;
    for (let i = 0; i < MAX_STOPS; i++) {
      await page.keyboard.press(key);
      const p = await page.evaluate(() => window.__focusProbe());
      // Forward only: Shift+Tab reads the same stops again.
      if (p.previous?.fault && key === "Tab")
        add("focus.ring", `${p.previous.sel}: ${p.previous.fault}`);
      if (p.kind === "lost") {
        lost++;
        continue;
      }
      if (first && p.id === first.id) {
        // Tab crosses the document's edge last, just before the first stop again.
        if (lost > (key === "Tab" ? 1 : 0))
          add("focus.trap", `focus lost to body before ${p.sel}`);
        return { first, stops, closed: true };
      }
      // Shift+Tab from the first stop crosses the document's edge first.
      const wrap = !stops.length && key !== "Tab" ? 1 : 0;
      if (lost > wrap) add("focus.trap", `focus lost to body before ${p.sel}`);
      lost = 0;
      if (stops.some((s) => s.id === p.id)) {
        add(
          "focus.trap",
          `focus loops at ${p.sel}, never back to the first stop`,
        );
        return { first, stops, closed: false };
      }
      if (p.kind === "other")
        add("focus.trap", `${p.sel}: a Tab stop that is no control`);
      if (first) stops.push(p);
      else first = p;
    }
    add("focus.trap", `no cycle within ${MAX_STOPS} Tab stops`);
    return { first, stops, closed: false };
  };

  const fwd = await cycle("Tab");
  const first = fwd.first;
  if (first) {
    const all = [first, ...fwd.stops];
    for (let i = 1; i < all.length; i++)
      if (!inOrder(all[i - 1].box as Box, all[i].box as Box))
        add(
          "focus.order",
          `${all[i - 1].sel} -> ${all[i].sel}: against reading order`,
        );

    if (fwd.closed) {
      const back = await cycle("Shift+Tab", first);
      const want = [...fwd.stops].reverse();
      for (let i = 0; i < Math.max(want.length, back.stops.length); i++)
        if (want[i]?.id !== back.stops[i]?.id)
          add(
            "focus.order",
            `Shift+Tab stop ${i + 1}: ${back.stops[i]?.sel ?? "none"}, not ${want[i]?.sel ?? "none"}`,
          );
    }

    // The skip link (§6.7): the first stop, onto `#main`, and the next Tab is main's first control.
    const target = first.skipTo;
    if (!target) add("focus.order", `first stop ${first.sel} is no skip link`);
    else {
      await page.evaluate(() => window.__focusStart());
      await page.keyboard.press("Tab");
      await page.keyboard.press("Enter");
      const order = fwd.stops.map((s) => s.id);
      const at = await page.evaluate(([t, o]) => window.__focusWithin(t, o), [
        target,
        order,
      ] as const);
      if (!at.on)
        add("focus.order", `skip link: Enter does not focus ${target}`);
      else {
        await page.keyboard.press("Tab");
        const p = await page.evaluate(() => window.__focusProbe());
        // A target with no control (Today's placeholder) has nothing to reach.
        if (at.first && p.id !== at.first)
          add(
            "focus.order",
            `skip link: Tab after it reaches ${p.sel}, not ${target}'s first control`,
          );
      }
    }
  }

  const counts: Counts = {};
  const worst: Record<string, string[]> = {};
  for (const [k, v] of Object.entries(rules)) {
    counts[k] = v.length;
    worst[k] = v.slice(0, TOP);
  }
  /** Tab stops in the forward cycle: a walk that reached none checked nothing. */
  const stops = first ? fwd.stops.length + 1 : 0;
  return { counts, worst, stops };
}
