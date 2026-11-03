// The shell's hand-off to the ride core. #34 builds the `core_ready` command; #89 wires `waitForCore` to its
// generated binding (src/bindings.ts is owned by the bindings export, never hand-edited). Until then the
// default adapter resolves at once, the "core ready before the first frame" case of BRAND.md
// §Start screen, with no payload: the destination is always Today until #259 routes to Onboarding / Resume.
// A rejection is a boot failure: the start screen shows its message and offers a retry.

/** Resolves when the backend handshake is done and persisted state is loaded; rejects with the reason it is not. */
export type WaitForCore = () => Promise<void>;

/** Default adapter, replaced by the #89 binding wiring. */
export const waitForCore: WaitForCore = () => Promise.resolve();
