import { For, Show } from "solid-js";
import type { RideStatus } from "@neo/shared";
import { clock } from "../lib/format";
import { createSample } from "../lib/primitives";
import { layoutState } from "../state/layout";
import { ride, type Speed } from "../state/ride";
import { router } from "../state/router";
import { ThemeMenu } from "../ui/ThemeMenu";
import { StatusBadge, type StatusKind } from "../widgets/chips";

const CONN: Record<RideStatus["trainer"], StatusKind> = { connected: "good", connecting: "warning", lost: "critical" };
const SPEEDS: Speed[] = [1, 10, 60];

export function TopBar(props: { onHelp: () => void }) {
  const elapsed = createSample(() => ({ metric: "elapsed", rateHz: 2 }));
  const remaining = createSample(() => ({ metric: "remaining", rateHz: 1 }));
  const s = ride.status;
  return (
    <header class="topbar">
      <div class="tb-group">
        <button class="ghost" title="Leave ride (review)" onClick={() => router.go("review")}>
          ◀
        </button>
        <span class="tb-time">
          <b>{clock(elapsed()?.v ?? s.t)}</b>
          <span class="muted"> / −{clock(remaining()?.v ?? 0)}</span>
        </span>
        <span class="tb-mode">{s.mode.toUpperCase()}</span>
        <StatusBadge kind={CONN[s.trainer]} label={`NEO ${s.trainer === "connected" ? "" : s.trainer}`} />
        <StatusBadge kind={CONN[s.hrSensor]} label={`HR ${s.hrSensor === "connected" ? "" : s.hrSensor}`} />
      </div>

      <div class="tb-group">
        <button class="big" title="−5 % (key −)" onClick={() => ride.controls.adjust(-5)}>
          −5%
        </button>
        <span class="tb-offset" classList={{ on: s.manualOffsetPct !== 0 }}>
          {s.manualOffsetPct > 0 ? "+" : ""}
          {s.manualOffsetPct}%
        </span>
        <button class="big" title="+5 % (key +)" onClick={() => ride.controls.adjust(5)}>
          +5%
        </button>
        <button class="big" title="Skip block (S)" onClick={() => ride.controls.skip()}>
          ⏭ Skip
        </button>
        <button class="big primary" title="Pause / resume (space)" onClick={() => ride.controls.togglePause()}>
          {s.state === "riding" ? "⏸ Pause" : s.state === "idle" ? "▶ Start" : s.state === "finished" ? "✓ Done" : "▶ Resume"}
        </button>
        <span class="seg" role="group" aria-label="Simulation speed">
          <For each={SPEEDS}>
            {(sp) => (
              <button classList={{ on: s.speed === sp }} onClick={() => ride.controls.setSpeed(sp)} title={`${sp}× (key ${SPEEDS.indexOf(sp) + 1})`}>
                {sp}×
              </button>
            )}
          </For>
        </span>
      </div>

      <div class="tb-group">
        <nav class="page-dots" aria-label="Pages">
          <For each={layoutState.layout.pages}>
            {(p, i) => (
              <button classList={{ on: layoutState.pageIdx() === i() }} onClick={() => layoutState.actions.goPage(i())} title={p.name}>
                <span class="dot" />
                {p.name}
              </button>
            )}
          </For>
        </nav>
        <label class="toggle" title="Switch page automatically by phase">
          <input type="checkbox" checked={layoutState.autoSwitch()} onChange={(e) => layoutState.actions.setAutoSwitch(e.currentTarget.checked)} />
          auto
        </label>
        <button class="ghost" classList={{ on: layoutState.editing() }} title="Edit layout (E)" onClick={() => layoutState.actions.setEditing(!layoutState.editing())}>
          ✎ Layout
        </button>
        <ThemeMenu />
        <button class="ghost" title="Shortcuts (?)" onClick={() => props.onHelp()}>
          ?
        </button>
        <Show when={s.state === "finished"}>
          <button class="primary" onClick={() => router.go("review")}>
            Review →
          </button>
        </Show>
      </div>
    </header>
  );
}
