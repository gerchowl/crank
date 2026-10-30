// Hook, CSS tokens and the too-small banner for the ride screen's type tiers (DESIGN §6.2, #91).
// The ride screen (#96) mounts <RideTiers> around its content; descendants use `var(--t1)` … `var(--t4)`.
import {
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type ReactNode,
} from "react";
import { readDisplayWidthMm, watchMonitorChange } from "./displayMm";
import { createMeasuredWidth } from "./measuredWidth";
import { useMonitorWidthMmOverride, useRideDistanceM } from "./rideSettings";
import {
  TOO_SMALL_TEXT,
  computeTiers,
  type Monitor,
  type Tiers,
} from "./typeTiers";

const subscribe = (notify: () => void) => {
  window.addEventListener("resize", notify);
  // The scale factor changes without a resize when the window moves to another monitor or the zoom changes;
  // a resolution query matches only the current ratio, so it fires on any change and is re-armed each time.
  // Limitation: a non-round ratio (e.g. 1.25 under browser zoom) is formatted by the engine and may not match
  // exactly, in which case a change is still caught by the next resize.
  let mq: MediaQueryList | null = null;
  const onDpr = () => {
    notify();
    arm();
  };
  const arm = () => {
    mq?.removeEventListener("change", onDpr);
    mq =
      typeof matchMedia === "function"
        ? matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`)
        : null;
    mq?.addEventListener("change", onDpr);
  };
  arm();
  // A "Looks like" change at the same backing scale changes `screen.width` and fires neither of the above (tao
  // emits nothing when the scale is unchanged, #390). The rider makes it in System Settings, so the window should
  // regain focus afterwards, and the shell's display-may-have-changed signal (focus gain, move, scale) re-reads it.
  // That is reasoning, not verified: that Tauri's focus event fires on return from System Settings, and that
  // WKWebView's `screen.width` has updated 200 ms later, are on the #44 HIL list.
  const stopWatching = watchMonitorChange(notify);
  return () => {
    window.removeEventListener("resize", notify);
    mq?.removeEventListener("change", onDpr);
    stopWatching();
  };
};
// A string snapshot keeps useSyncExternalStore referentially stable.
const snapshot = () =>
  [
    window.innerWidth,
    window.innerHeight,
    window.screen.width,
    window.devicePixelRatio,
  ].join("x");

/**
 * The webview exposes no physical size, so `widthMm` is the shell's measurement (`display_width_mm`, #286),
 * null while it is unknown or not yet answered. The override setting is applied after it in `computeTiers`.
 */
const monitorOf = (
  screenW: number,
  dpr: number,
  widthMm: number | null,
): Monitor => ({
  widthPx: screenW * dpr,
  scaleFactor: dpr,
  widthMm,
});

/**
 * The measured physical width of the window's display, refreshed when the window may have changed display (see
 * `watchMonitorChange`). One store per mounted tree (`createMeasuredWidth`): reads are ordered by a sequence
 * number and a read that outlives the last subscriber is dropped.
 */
function useMeasuredWidthMm(): number | null {
  const [store] = useState(() =>
    createMeasuredWidth(readDisplayWidthMm, watchMonitorChange),
  );
  return useSyncExternalStore(store.subscribe, store.get);
}

/** Live tiers for the current window, monitor, distance and override. */
export function useTypeTiers(): Tiers {
  const snap = useSyncExternalStore(subscribe, snapshot);
  const distanceM = useRideDistanceM();
  const widthMmOverride = useMonitorWidthMmOverride();
  const measuredMm = useMeasuredWidthMm();
  const [w, h, screenW, dpr] = snap.split("x").map(Number) as [
    number,
    number,
    number,
    number,
  ];
  return computeTiers({
    monitor: monitorOf(screenW, dpr, measuredMm),
    widthMmOverride,
    distanceM,
    window: { width: w, height: h },
  });
}

/** "Window too small to read targets", sized at T3+ (#91). */
export function TooSmallBanner() {
  return (
    <div
      role="alert"
      className="too-small-banner"
      style={{ fontSize: "var(--t3p)" }}
    >
      {TOO_SMALL_TEXT}
    </div>
  );
}

/** Height reserved for the status-bar slot: one T3+ line, the taller of the banner (T3+) and the notice (T4). */
const SLOT_HEIGHT = "calc(var(--t3p) * 1.5)";

/**
 * Applies the tier tokens to its subtree. The too-small banner and the unknown-size notice go in a status-bar
 * slot below the content that is always rendered at a fixed height (DESIGN §6.2, §10 #10), so the content's
 * position does not depend on the warning. The parent must give this component a defined height (e.g.
 * `height: 100%` chain from the viewport, #96): the slot is pinned to the bottom of that height.
 */
export function RideTiers({ children }: { children: ReactNode }) {
  const tiers = useTypeTiers();
  return (
    <div
      style={{
        ...(tiers.tokens as CSSProperties),
        display: "flex",
        flexDirection: "column",
        height: "100%",
      }}
    >
      <div data-slot="content" style={{ flex: "1 1 0", minHeight: 0 }}>
        {children}
      </div>
      <div
        data-slot="status-bar"
        style={{ flex: "none", height: SLOT_HEIGHT, overflow: "hidden" }}
      >
        {tiers.tooSmallForT2 && <TooSmallBanner />}
        {tiers.notice && (
          <p
            role="status"
            style={{ margin: 0, fontSize: "var(--t4)", whiteSpace: "nowrap" }}
          >
            {tiers.notice}
          </p>
        )}
      </div>
    </div>
  );
}
