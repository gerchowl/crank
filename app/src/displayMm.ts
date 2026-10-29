// The physical width of the display the window is on, from the shell (#286, DESIGN §6.2). A webview cannot read
// it, so the `display_width_mm` command answers (CoreGraphics on macOS; unknown elsewhere, see its doc).
import { getCurrentWindow } from "@tauri-apps/api/window";
import { commands } from "./bindings";

/** The display's physical width in mm; null when unknown, not a positive finite number, or outside the shell. */
export async function readDisplayWidthMm(): Promise<number | null> {
  try {
    const mm = await commands.displayWidthMm();
    return typeof mm === "number" && Number.isFinite(mm) && mm > 0 ? mm : null;
  } catch {
    return null;
  }
}

/** Trailing delay that collapses a burst of events (a drag, a focus change after a settings change) into one refresh. */
const SETTLE_MS = 200;

/**
 * Calls `notify` once the window may be on another display, after a short settle. Three window events, each for a
 * different way the display under it changes:
 * - `onMoved` (`tauri://move`): every move, including the one onto another monitor (drag, keyboard, maximise).
 * - `onScaleChanged` (`tauri://scale-change`): the window's display changed its scale factor (a resolution or
 *   scaling change, a move between displays of different scale).
 * - `onFocusChanged`, on gaining focus: an arrangement, resolution or mirroring change made in System Settings
 *   (or by plugging a display in) while the window did not have focus, with no move and no scale change. The
 *   rider comes back to the app by focusing it, which re-reads.
 *
 * Not covered, by choice: a display change while the window keeps the focus and neither moves nor changes
 * scale (hot-plugging a mirrored display under a focused window, a resolution change by an external tool). A
 * native display-reconfiguration callback (`CGDisplayRegisterReconfigurationCallback`) would catch it but needs
 * an unsafe C callback and event plumbing for a case the next move or focus fixes; it is on the #44 HIL list.
 *
 * Returns the unsubscribe function; outside Tauri (tests, a plain browser) it subscribes to nothing.
 */
export function watchMonitorChange(notify: () => void): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;
  const unlisten: (() => void)[] = [];
  const settle = () => {
    clearTimeout(timer);
    timer = setTimeout(notify, SETTLE_MS);
  };
  const keep = (off: () => void) => (stopped ? off() : unlisten.push(off));
  try {
    const win = getCurrentWindow();
    for (const listening of [
      win.onMoved(settle),
      win.onScaleChanged(settle),
      win.onFocusChanged(({ payload: focused }) => {
        if (focused) settle();
      }),
    ])
      listening.then(keep).catch(() => undefined);
  } catch {
    // not running inside Tauri
  }
  return () => {
    stopped = true;
    clearTimeout(timer);
    for (const off of unlisten.splice(0)) off();
  };
}
