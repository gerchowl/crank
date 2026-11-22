// The window's close request (Tauri `onCloseRequested`), as the workout editor uses it to ask about unsaved changes
// before the window closes (DESIGN §5.7, #110). The close button and Cmd+Q both arrive here: the Quit menu item asks
// each window to close (app/src-tauri lifecycle.rs `quit`). A listener that calls `preventDefault` keeps the window
// open; `destroy` then closes it once the rider has answered. Tests replace `windowClose` (as `typingClock`).
import { getCurrentWindow } from "@tauri-apps/api/window";

export interface CloseRequest {
  preventDefault(): void;
}
export const windowClose = {
  /** Listen for the close request; resolves to the removal of the listener. */
  listen: (handler: (e: CloseRequest) => void): Promise<() => void> =>
    getCurrentWindow().onCloseRequested(handler),
  /** Close the window without asking again. */
  destroy: (): Promise<void> => getCurrentWindow().destroy(),
};
