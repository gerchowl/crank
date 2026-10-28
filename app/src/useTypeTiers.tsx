// Hook, CSS tokens and the too-small banner for the ride screen's type tiers (DESIGN §6.2, #91).
// The ride screen (#96) mounts <RideTiers> around its content; descendants use `var(--t1)` … `var(--t4)`.
import {
  useSyncExternalStore,
  type CSSProperties,
  type ReactNode,
} from "react";
import { useMonitorWidthMmOverride, useRideDistanceM } from "./rideSettings";
import { computeTiers, type Monitor, type Tiers } from "./typeTiers";

const subscribe = (notify: () => void) => {
  window.addEventListener("resize", notify);
  return () => window.removeEventListener("resize", notify);
};
// A string snapshot keeps useSyncExternalStore referentially stable.
const snapshot = () =>
  [
    window.innerWidth,
    window.innerHeight,
    window.screen.width,
    window.devicePixelRatio,
  ].join("x");

/** The webview exposes no physical size; `widthMm` stays null until the override setting supplies one. */
const monitorOf = (screenW: number, dpr: number): Monitor => ({
  widthPx: screenW * dpr,
  scaleFactor: dpr,
  widthMm: null,
});

/** Live tiers for the current window, monitor, distance and override. */
export function useTypeTiers(): Tiers {
  const snap = useSyncExternalStore(subscribe, snapshot);
  const distanceM = useRideDistanceM();
  const widthMmOverride = useMonitorWidthMmOverride();
  const [w, h, screenW, dpr] = snap.split("x").map(Number) as [
    number,
    number,
    number,
    number,
  ];
  return computeTiers({
    monitor: monitorOf(screenW, dpr),
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
      Window too small to read targets
    </div>
  );
}

/** Applies the tier tokens to its subtree and shows the too-small banner and the unknown-size notice. */
export function RideTiers({ children }: { children: ReactNode }) {
  const tiers = useTypeTiers();
  return (
    <div style={tiers.tokens as CSSProperties}>
      {tiers.tooSmallForT2 && <TooSmallBanner />}
      {tiers.notice && (
        <p role="status" style={{ fontSize: "var(--t3)" }}>
          {tiers.notice}
        </p>
      )}
      {children}
    </div>
  );
}
