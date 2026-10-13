import type uPlot from "uplot";
import type { MmpCurve } from "@neo/shared";

export function toMmpData(c: MmpCurve): uPlot.AlignedData {
  return [c.durationsS, c.today.map((v) => (v > 0 ? v : null)), c.best90d, c.allTime];
}

