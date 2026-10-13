import { Show } from "solid-js";
import type { Band } from "@neo/shared";

/** Zone chip: colour swatch carries the zone, the label is ink (never the series colour). */
export function ZoneChip(props: { idx: number; label: string }) {
  return (
    <span class="zone-chip">
      <span class="swatch" style={{ background: `var(--zone-${props.idx + 1})` }} />
      {props.label}
    </span>
  );
}

/** Δ to target as a diverging chip with a neutral midpoint (±2 % counts as on target). */
export function DeltaChip(props: { delta: number; ref: number; unit: string }) {
  const pct = () => (props.ref ? (props.delta / props.ref) * 100 : 0);
  const side = () => (Math.abs(pct()) < 2 ? "mid" : props.delta > 0 ? "pos" : "neg");
  return (
    <span class={`delta-chip delta-${side()}`}>
      {props.delta > 0 ? "▲ +" : props.delta < 0 ? "▼ " : ""}
      {Math.round(props.delta)} {props.unit}
      <Show when={props.ref}>
        <span class="muted"> ({pct() > 0 ? "+" : ""}{Math.round(pct())}%)</span>
      </Show>
    </span>
  );
}

const BAND_ICON: Record<Band, string> = { strong: "●", lean: "◐", uncertain: "○" };

export function BandChip(props: { band: Band; p: number }) {
  return (
    <span class={`band-chip band-${props.band}`}>
      {BAND_ICON[props.band]} {props.band} <span class="muted">{Math.round(props.p * 100)}%</span>
    </span>
  );
}

export type StatusKind = "good" | "warning" | "serious" | "critical";
const STATUS_ICON: Record<StatusKind, string> = { good: "✓", warning: "!", serious: "⚠", critical: "✕" };

/** Status colours are reserved for coach/sensor state and always carry an icon + label. */
export function StatusBadge(props: { kind: StatusKind; label: string }) {
  return (
    <span class={`status status-${props.kind}`}>
      <span class="status-icon" aria-hidden="true">
        {STATUS_ICON[props.kind]}
      </span>
      {props.label}
    </span>
  );
}
