import { For, Show } from "solid-js";
import type { CompiledWorkout } from "@neo/shared";
import { backend } from "../backend";
import { clock } from "../lib/format";
import { StatusBadge } from "../widgets/chips";
import { zoneLabel } from "../widgets/common";
import { editor } from "./state";

/** Summary from compileWorkout: duration (range), TSS, IF, time-in-zone bars, validation issues. */
export function Summary(props: { compiled: CompiledWorkout }) {
  const maxZ = () => Math.max(1, ...props.compiled.timeInZoneS);
  const ranged = () => props.compiled.totalRangeS[0] !== props.compiled.totalRangeS[1];
  return (
    <div class="summary card">
      <h2>Summary</h2>
      <div class="stat-row">
        <div class="stat">
          <span class="stat-label">Duration</span>
          <span class="stat-value">{clock(props.compiled.totalS)}</span>
          <Show when={ranged()}>
            <span class="muted small">
              {clock(props.compiled.totalRangeS[0])}–{clock(props.compiled.totalRangeS[1])}
            </span>
          </Show>
        </div>
        <div class="stat">
          <span class="stat-label">TSS</span>
          <span class="stat-value">{Math.round(props.compiled.tss)}</span>
        </div>
        <div class="stat">
          <span class="stat-label">IF</span>
          <span class="stat-value">{props.compiled.if.toFixed(2)}</span>
        </div>
        <div class="stat">
          <span class="stat-label">Blocks</span>
          <span class="stat-value">{props.compiled.blocks.length}</span>
        </div>
      </div>
      <div class="tiz" role="table" aria-label="Time in zone">
        <For each={props.compiled.timeInZoneS}>
          {(s, i) => (
            <div class="tiz-row" role="row">
              <span class="tiz-label" role="cell">
                {zoneLabel(backend.zones.power[i()], i())}
              </span>
              <span class="tiz-bar" role="cell">
                <span style={{ width: `${(s / maxZ()) * 100}%`, background: `var(--zone-${i() + 1})` }} />
              </span>
              <span class="tiz-val" role="cell">
                {clock(s)}
              </span>
            </div>
          )}
        </For>
      </div>
      <div class="issues">
        <Show when={props.compiled.issues.length} fallback={<StatusBadge kind="good" label="No validation issues" />}>
          <For each={props.compiled.issues}>
            {(iss) => (
              <button class="issue" onClick={() => iss.nodeId && editor.actions.selectOnly([iss.nodeId])}>
                <StatusBadge kind={iss.severity === "error" ? "critical" : "warning"} label={iss.severity} /> {iss.message}
              </button>
            )}
          </For>
        </Show>
      </div>
    </div>
  );
}
