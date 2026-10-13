import { createMemo, For, Show } from "solid-js";
import { metricInfo } from "../backend";
import { formatValue, zoneIdx } from "../lib/format";
import { createSample } from "../lib/primitives";
import { ZoneChip } from "./chips";
import { type WidgetProps, zoneLabel, zonesOf } from "./common";

/** P03 — horizontal bar: zone bands (sequential ramp), target band shaded, marker = value. */
export function ZoneBar(props: WidgetProps) {
  const metric = () => props.widget.metrics[0] ?? "wbal_pct";
  const targetMetric = () => props.widget.metrics[1];
  const sample = createSample(() => ({ metric: metric(), rateHz: props.widget.rateHz, filter: props.widget.filter }));
  const target = createSample(() => {
    const m = targetMetric();
    return m ? { metric: m, rateHz: 1 } : null;
  });
  const info = () => metricInfo(metric());
  const value = () => sample()?.v ?? null;

  const range = createMemo((): [number, number] => {
    const zs = zonesOf(metric());
    if (info().unit === "%" && (!zs || (zs[zs.length - 1]?.max ?? 0) <= 101)) return [0, 100];
    if (zs && zs.length) {
      const last = zs[zs.length - 1];
      const hi = Math.min(last?.max ?? 100, (zs[zs.length - 2]?.max ?? 100) * 1.25);
      return [0, hi];
    }
    return [0, 100];
  });
  const pos = (v: number) => {
    const [lo, hi] = range();
    return Math.max(0, Math.min(100, ((v - lo) / (hi - lo)) * 100));
  };
  const zones = createMemo(() => {
    const zs = zonesOf(metric());
    if (!zs) return [];
    // spread the zones over the sequential ramp so the bar reads light → dark
    return zs.map((z, i) => ({ ...z, rampIdx: zs.length === 1 ? 3 : Math.round((i / (zs.length - 1)) * 6), label: zoneLabel(z, i) }));
  });
  const zone = createMemo(() => {
    const zs = zonesOf(metric());
    const v = value();
    if (!zs || v === null) return null;
    const i = zoneIdx(zs, v);
    const z = zones()[i];
    return { idx: z?.rampIdx ?? i, label: zoneLabel(zs[i], i) };
  });

  return (
    <div class="zonebar">
      <div class="w-label">
        <span>{info().label}</span>
        <span class="zonebar-value">
          {formatValue(metric(), value())} <span class="bignum-unit">{info().unit}</span>
        </span>
        <Show when={props.widget.colorByZone && zone()}>{(z) => <ZoneChip idx={z().idx} label={z().label} />}</Show>
      </div>
      <div class="zonebar-track" role="meter" aria-valuenow={value() ?? 0} aria-valuemin={range()[0]} aria-valuemax={range()[1]} aria-label={info().label}>
        <Show
          when={props.widget.colorByZone && zones().length}
          fallback={<div class="zonebar-fill" style={{ width: `${pos(value() ?? 0)}%` }} />}
        >
          {/* zone bands stay faint; the part up to the value is filled (clip-path follows the marker) */}
          <For each={zones()}>
            {(z) => (
              <div
                class="zonebar-zone"
                style={{ left: `${pos(z.min)}%`, width: `${pos(Math.min(z.max, range()[1])) - pos(z.min)}%`, background: `var(--zone-${z.rampIdx + 1})` }}
              />
            )}
          </For>
          <For each={zones()}>
            {(z) => (
              <div
                class="zonebar-zone reached"
                style={{
                  left: `${pos(z.min)}%`,
                  width: `${pos(Math.min(z.max, range()[1])) - pos(z.min)}%`,
                  background: `var(--zone-${z.rampIdx + 1})`,
                  "clip-path": `inset(0 ${Math.max(0, 100 - ((pos(value() ?? 0) - pos(z.min)) / Math.max(0.001, pos(Math.min(z.max, range()[1])) - pos(z.min))) * 100)}% 0 0)`,
                }}
              />
            )}
          </For>
        </Show>
        <Show when={target()?.v}>
          {(tv) => <div class="zonebar-target" style={{ left: `${pos(tv() * 0.95)}%`, width: `${pos(tv() * 1.05) - pos(tv() * 0.95)}%` }} />}
        </Show>
        <Show when={value() !== null}>
          <div class="zonebar-marker" style={{ left: `${pos(value() ?? 0)}%` }} />
        </Show>
      </div>
      <Show when={props.widget.colorByZone && zones().length}>
        <div class="zonebar-labels" aria-hidden="true">
          <For each={zones()}>{(z) => <span style={{ left: `${pos(z.min)}%` }}>{z.label}</span>}</For>
        </div>
      </Show>
    </div>
  );
}
