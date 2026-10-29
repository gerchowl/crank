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

/** Trailing delay that collapses the stream of move events of a drag into one refresh. */
const SETTLE_MS = 200;

/**
 * Calls `notify` once the window has stopped moving. The window event is `onMoved` (Tauri's `tauri://move`): it
 * fires for every move, including the one that takes the window onto another monitor (by drag, by keyboard or
 * by a maximise), which is the only way the display under it changes; `onScaleChanged` would miss a move
 * between two monitors with the same scale factor. Returns the unsubscribe function; outside Tauri
 * (tests, a plain browser) it subscribes to nothing.
 */
export function watchMonitorChange(notify: () => void): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;
  let unlisten: (() => void) | undefined;
  try {
    void getCurrentWindow()
      .onMoved(() => {
        clearTimeout(timer);
        timer = setTimeout(notify, SETTLE_MS);
      })
      .then((off) => (stopped ? off() : (unlisten = off)))
      .catch(() => undefined);
  } catch {
    // not running inside Tauri
  }
  return () => {
    stopped = true;
    clearTimeout(timer);
    unlisten?.();
  };
}
