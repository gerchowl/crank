import { createMemo, createResource, createSignal, For, Match, Show, Switch } from "solid-js";
import { createStore, unwrap } from "solid-js/store";
import type { Checkin as CheckinData, Proposal } from "@neo/shared";
import { backend } from "../backend";
import { Canvas } from "../charts/Canvas";
import { layoutProfile, paintProfile, profileView } from "../charts/profile";
import { editor } from "../editor/state";
import { clock } from "../lib/format";
import { ride } from "../state/ride";
import { router } from "../state/router";
import type { ThemeTokens } from "../theme/theme";

type Score = 1 | 2 | 3 | 4 | 5;
const SCORES: Score[] = [1, 2, 3, 4, 5];
const ROWS: { key: "sleep" | "legs" | "stress" | "motivation"; label: string; lo: string; hi: string }[] = [
  { key: "sleep", label: "Sleep", lo: "awful", hi: "great" },
  { key: "legs", label: "Legs", lo: "dead", hi: "fresh" },
  { key: "stress", label: "Stress", lo: "calm", hi: "maxed" },
  { key: "motivation", label: "Motivation", lo: "none", hi: "fired up" },
];
const MINUTES = [30, 45, 60, 90];

/** V2 — check-in (≤ 10 s) → planner proposal → accept / regenerate / edit. */
export function Checkin() {
  const [form, setForm] = createStore<CheckinData>({ sleep: 3, legs: 3, stress: 3, motivation: 3, minutes: 60 });
  const [custom, setCustom] = createSignal(false);
  const [request, setRequest] = createSignal<{ c: CheckinData; n: number } | null>(null);
  let n = 0;
  const [proposal, { refetch }] = createResource(request, (r) => backend.planner.propose(r.c));

  const submit = (e?: Event) => {
    e?.preventDefault();
    setRequest({ c: structuredClone(unwrap(form)), n: ++n });
  };

  const accept = (p: Proposal) => {
    ride.controls.load(p.workout);
    router.go("ride");
  };
  const edit = (p: Proposal) => {
    editor.actions.load(p.workout);
    router.go("editor");
  };

  return (
    <div class="page checkin">
      <h1>How are you today?</h1>
      <form class="card checkin-form" onSubmit={submit}>
        <For each={ROWS}>
          {(row) => (
            <div class="ci-row" role="radiogroup" aria-label={row.label}>
              <span class="ci-label">{row.label}</span>
              <span class="ci-scale">
                <span class="muted small">{row.lo}</span>
                <For each={SCORES}>
                  {(s) => (
                    <button
                      type="button"
                      role="radio"
                      aria-checked={form[row.key] === s}
                      class="ci-dot"
                      classList={{ on: form[row.key] === s }}
                      onClick={() => setForm(row.key, s)}
                    >
                      {s}
                    </button>
                  )}
                </For>
                <span class="muted small">{row.hi}</span>
              </span>
            </div>
          )}
        </For>
        <div class="ci-row">
          <span class="ci-label">Time</span>
          <span class="ci-scale">
            <For each={MINUTES}>
              {(m) => (
                <button type="button" class="ci-chip" classList={{ on: !custom() && form.minutes === m }} onClick={() => (setCustom(false), setForm("minutes", m))}>
                  {m}′
                </button>
              )}
            </For>
            <button type="button" class="ci-chip" classList={{ on: custom() }} onClick={() => setCustom(true)}>
              custom
            </button>
            <Show when={custom()}>
              <input
                class="num"
                type="number"
                min="15"
                max="240"
                step="5"
                value={form.minutes}
                onInput={(e) => setForm("minutes", Math.max(15, Number(e.currentTarget.value) || 60))}
                aria-label="Minutes"
              />
            </Show>
          </span>
        </div>
        <label class="ci-row">
          <span class="ci-label">Note</span>
          <input
            class="text ci-note"
            placeholder="how do you feel? (optional)"
            value={form.note ?? ""}
            onInput={(e) => {
              const v = e.currentTarget.value;
              if (v) setForm("note", v);
              else setForm("note", undefined);
            }}
          />
        </label>
        <div class="ci-actions">
          <button type="submit" class="primary big" disabled={proposal.loading}>
            {proposal.loading ? "Planning…" : "Get my workout →"}
          </button>
        </div>
      </form>

      <Switch>
        <Match when={proposal.loading}>
          <div class="card proposal loading" role="status">
            <div class="skeleton" />
            The coach is planning your session…
          </div>
        </Match>
        <Match when={proposal.error !== undefined}>
          <div class="card" role="alert">
            Planner failed. <button onClick={() => void refetch()}>Retry</button>
          </div>
        </Match>
        <Match when={proposal()}>{(p) => <ProposalCard proposal={p()} onAccept={accept} onEdit={edit} onRegenerate={() => void refetch()} />}</Match>
      </Switch>
    </div>
  );
}

function ProposalCard(props: { proposal: Proposal; onAccept: (p: Proposal) => void; onEdit: (p: Proposal) => void; onRegenerate: () => void }) {
  const compiled = createMemo(() => backend.compileWorkout(props.proposal.workout));
  const draw = (ctx: CanvasRenderingContext2D, w: number, h: number, t: ThemeTokens) => {
    const blocks = compiled().blocks;
    const v = profileView(blocks, { pad: { l: 0, r: 0, t: 6, b: 0 } });
    paintProfile(ctx, layoutProfile(blocks, w, h, v), t, w, v, { ftpLine: true });
  };
  return (
    <div class="card proposal">
      <h2>Today's proposal</h2>
      <div class="proposal-head">
        <span class="proposal-name">{props.proposal.workout.name}</span>
        <span class="muted">
          {clock(compiled().totalS)} · TSS {Math.round(compiled().tss)} · IF {compiled().if.toFixed(2)}
        </span>
      </div>
      <div class="proposal-profile">
        <Canvas draw={draw} ariaLabel="proposed workout profile" />
      </div>
      <p class="rationale">
        <span class="muted">why: </span>
        {props.proposal.rationale}
      </p>
      <div class="ci-actions">
        <button class="primary big" onClick={() => props.onAccept(props.proposal)}>
          ▶ Accept & ride
        </button>
        <button class="big" onClick={() => props.onRegenerate()}>
          ↻ Regenerate
        </button>
        <button class="big" onClick={() => props.onEdit(props.proposal)}>
          ✎ Edit
        </button>
      </div>
    </div>
  );
}
