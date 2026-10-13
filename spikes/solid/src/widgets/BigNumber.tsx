import { createMemo, createSignal, Show } from "solid-js";
import type { Filter } from "@neo/shared";
import { ftp, metricInfo } from "../backend";
import { formatValue, zoneIdx } from "../lib/format";
import { createSample } from "../lib/primitives";
import { ride } from "../state/ride";
import { FILTER_PRESETS, filterLabel, type WidgetProps, zoneLabel, zonesOf } from "./common";
import { DeltaChip, ZoneChip } from "./chips";

/** P01 — big number, optional zone chip and Δ-to-target chip; tap cycles smoothing. */
export function BigNumber(props: WidgetProps) {
  const metric = () => props.widget.metrics[0] ?? "power_3s";
  const targetMetric = () => props.widget.metrics[1];
  const [smoothing, setSmoothing] = createSignal<number | null>(null);
  const filter = (): Filter[] | undefined => {
    const i = smoothing();
    return i === null ? props.widget.filter : FILTER_PRESETS[i]?.filter;
  };
  const sample = createSample(() => ({ metric: metric(), rateHz: props.widget.rateHz, filter: filter() }));
  const target = createSample(() => {
    const m = targetMetric();
    return m ? { metric: m, rateHz: Math.min(props.widget.rateHz, 2) } : null;
  });
  const info = () => metricInfo(metric());
  const value = () => sample()?.v ?? null;

  const zone = createMemo(() => {
    const zs = zonesOf(metric());
    const v = value();
    if (!props.widget.colorByZone || !zs || v === null) return null;
    const i = zoneIdx(zs, v);
    return { idx: i, label: zoneLabel(zs[i], i) };
  });

  const cycle = () => {
    if (props.editing) return;
    setSmoothing((i) => (i === null ? 0 : (i + 1) % FILTER_PRESETS.length));
  };

  // countdown tiles show "block x/n" and the next block
  const isCountdown = () => metric() === "block_remaining";
  const blockInfo = createMemo(() => {
    const cur = ride.current();
    if (!isCountdown() || !cur) return null;
    const i = ride.status.blockIndex;
    const n = cur.compiled.blocks.length;
    const next = cur.compiled.blocks[i + 1];
    return {
      label: `block ${Math.min(i + 1, n)}/${n}`,
      next: next ? `next: ${formatValue("elapsed", next.durS)} @ ${Math.round(next.powerFrom)} W (${Math.round((next.powerFrom / ftp) * 100)}%)` : "last block",
    };
  });

  return (
    <div class="bignum" onClick={cycle} title={`Tap to cycle smoothing (now: ${filterLabel(filter())})`}>
      <div class="w-label">
        <span>{isCountdown() ? "Interval" : info().label}</span>
        <Show when={blockInfo()}>{(b) => <span class="muted">{b().label}</span>}</Show>
        <Show when={smoothing() !== null}>
          <span class="pill">{filterLabel(filter())}</span>
        </Show>
      </div>
      <div class="bignum-row">
        <span class="bignum-value" data-metric={metric()}>
          {formatValue(metric(), value())}
        </span>
        <span class="bignum-unit">{isCountdown() ? "" : info().unit}</span>
        <Show when={zone()}>{(z) => <ZoneChip idx={z().idx} label={z().label} />}</Show>
      </div>
      <Show when={targetMetric() && target()?.v != null}>
        <div class="bignum-target">
          <span class="muted">target</span> {formatValue(targetMetric() ?? metric(), target()?.v ?? null)} {metricInfo(targetMetric() ?? metric()).unit}
          <Show when={value() !== null}>
            <DeltaChip delta={(value() ?? 0) - (target()?.v ?? 0)} ref={target()?.v ?? 0} unit={info().unit} />
          </Show>
        </div>
      </Show>
      <Show when={blockInfo()}>{(b) => <div class="bignum-target muted">{b().next}</div>}</Show>
    </div>
  );
}
