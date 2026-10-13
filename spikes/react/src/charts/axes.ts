import type uPlot from "uplot";
import type { ChartTheme } from "../theme/theme";

export function axisStyle(t: ChartTheme): Pick<uPlot.Axis, "stroke" | "grid" | "ticks" | "font"> {
  return {
    stroke: t.muted,
    grid: { stroke: t.grid, width: 1 },
    ticks: { stroke: t.grid, width: 1, size: 4 },
    font: `11px ${t.font}`,
  };
}

export function alpha(hex: string, a: number): string {
  const m = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m?.[1]) return hex;
  const n = parseInt(m[1], 16);
  return `rgba(${String((n >> 16) & 255)},${String((n >> 8) & 255)},${String(n & 255)},${String(a)})`;
}
