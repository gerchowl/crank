import type { ReactNode } from "react";
import type { Widget } from "@neo/shared";
import type { ChartTheme } from "../../theme/theme";
import { zoneColor } from "../../theme/theme";
import type { ZoneHit } from "./metrics";

export function ZoneChip({ hit, theme }: { hit: ZoneHit | null; theme: ChartTheme }) {
  if (!hit) return null;
  const label = hit.name.split(" ")[0] ?? hit.name;
  return (
    <span className="zone-chip">
      <i style={{ background: zoneColor(theme, hit.idx, hit.count) }} />
      {label}
    </span>
  );
}

export function WidgetFrame({ title, aside, children, className }: { title: ReactNode; aside?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={`widget ${className ?? ""}`}>
      <header className="widget-head">
        <h3>{title}</h3>
        {aside}
      </header>
      <div className="widget-body">{children}</div>
    </section>
  );
}

export interface WidgetProps { widget: Widget }
