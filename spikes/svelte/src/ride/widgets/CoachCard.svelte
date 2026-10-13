<script lang="ts">
  import type { Band, CoachNudge, Widget } from "@neo/shared";
  import { ride } from "../../lib/ride.svelte";
  import { clock } from "../../lib/format";

  // Coach card: newest nudge, its band (strong/lean/uncertain, icon + label), status and the
  // big accept/decline buttons (keys Y / N, handled globally on the ride screen).
  let { widget }: { widget: Widget } = $props();
  const compact = $derived(widget.h < 2);

  const n = $derived<CoachNudge | undefined>(ride.pending ?? ride.latestNudge);

  const VERDICT: Record<CoachNudge["verdict"], string> = { ease_off: "Ease off", hold: "Hold", push: "Push", replan: "Re-plan" };
  const BAND_STATUS: Record<Band, string> = { strong: "good", lean: "warning", uncertain: "serious" };
  const STATUS_TEXT: Record<CoachNudge["status"], string> = {
    pending: "waiting for you",
    auto_applied: "applied automatically",
    accepted: "accepted",
    declined: "declined",
    expired: "expired",
  };
</script>

<div class="coach" class:pending={n?.status === "pending"}>
  <div class="head">
    <span class="title">Coach</span>
    {#if n}
      <span class="status {BAND_STATUS[n.band]}">{n.band} · p {n.p.toFixed(2)}</span>
      <span class="muted">{clock(n.t)}</span>
    {/if}
  </div>
  {#if n}
    <div class="verdict">
      {VERDICT[n.verdict]}{#if n.proposedPctChange !== undefined}<span class="pct">{n.proposedPctChange > 0 ? "+" : ""}{n.proposedPctChange}%</span>{/if}
    </div>
    {#if !compact}<p class="msg">{n.message}</p>{/if}
    {#if n.status === "pending"}
      <div class="actions">
        <button type="button" class="primary" onclick={() => {
          ride.respond(true);
        }}><kbd>Y</kbd> Accept</button>
        <button type="button" onclick={() => {
          ride.respond(false);
        }}><kbd>N</kbd> Decline</button>
      </div>
    {:else}
      <p class="secondary state">{STATUS_TEXT[n.status]}</p>
    {/if}
  {:else}
    <p class="muted">No nudges yet. The coach watches HR vs your usual response and W'bal.</p>
  {/if}
</div>

<style>
  .coach {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0.6rem 0.8rem;
    overflow: hidden;
    border-left: 4px solid transparent;
  }
  .coach.pending {
    border-left-color: var(--accent);
  }
  .head {
    display: flex;
    gap: 0.6rem;
    align-items: center;
    font-size: 0.75rem;
  }
  .title {
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-secondary);
    flex: 1;
  }
  .verdict {
    font-size: 1.6rem;
    font-weight: 650;
  }
  .pct {
    margin-left: 0.5rem;
    font-size: 0.8em;
    color: var(--ink-secondary);
  }
  .msg {
    margin: 0;
    font-size: 0.95rem;
    line-height: 1.35;
    color: var(--ink-secondary);
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    margin-top: auto;
  }
  .actions button {
    flex: 1;
    min-height: 52px;
    font-size: 1.05rem;
  }
  .state {
    margin: auto 0 0;
    font-size: 0.85rem;
  }
</style>
