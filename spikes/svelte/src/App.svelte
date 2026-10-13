<script lang="ts">
  import { theme } from "./lib/theme.svelte";
  import { nav, isTyping, type View } from "./lib/nav.svelte";
  import { ride } from "./lib/ride.svelte";
  import RideScreen from "./ride/RideScreen.svelte";
  import Editor from "./editor/Editor.svelte";
  import Checkin from "./checkin/Checkin.svelte";
  import Review from "./review/Review.svelte";
  import HelpOverlay from "./ui/HelpOverlay.svelte";
  import ThemeMenu from "./ui/ThemeMenu.svelte";
  import { clock } from "./lib/format";

  // Root effect: theme attributes + token snapshot for canvases. Runs on any theme change.
  $effect(() => {
    theme.apply();
  });

  const RAIL: { id: View; label: string; icon: string }[] = [
    { id: "checkin", label: "Check-in", icon: "☀" },
    { id: "editor", label: "Editor", icon: "✎" },
    { id: "review", label: "Review", icon: "▤" },
  ];

  function onkeydown(e: KeyboardEvent): void {
    if (nav.view !== "ride" && e.key === "?" && !isTyping(e)) {
      nav.help = !nav.help;
      e.preventDefault();
    }
  }
</script>

<svelte:window {onkeydown} />

{#if nav.view === "ride"}
  <!-- Ride is full-screen (no rail); Exit leads to the review. -->
  <RideScreen />
{:else}
  <div class="shell">
    <nav class="rail" aria-label="Main">
      <div class="brand">neo<span class="muted">·trainer</span></div>
      {#each RAIL as r (r.id)}
        <button type="button" class="ghost" aria-current={nav.view === r.id ? "page" : undefined} onclick={() => nav.go(r.id)}>
          <span class="icon">{r.icon}</span>{r.label}
        </button>
      {/each}
      <button type="button" class="primary ride-btn" onclick={() => nav.go("ride")}>
        ▶ Ride
        {#if ride.state === "riding" || ride.state === "paused"}<span class="tabular">{clock(ride.tSec)}</span>{/if}
      </button>
      <div class="bottom">
        <ThemeMenu />
        <button type="button" class="ghost" onclick={() => (nav.help = true)}><kbd>?</kbd> keys</button>
      </div>
    </nav>
    <main>
      {#if nav.view === "editor"}
        <Editor />
      {:else if nav.view === "checkin"}
        <Checkin />
      {:else}
        <Review />
      {/if}
    </main>
  </div>
{/if}
<HelpOverlay />

<style>
  .shell {
    height: 100%;
    display: flex;
  }
  .rail {
    width: 168px;
    flex: 0 0 168px;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.8rem 0.6rem;
    background: var(--surface-1);
    border-right: 1px solid var(--border);
  }
  .brand {
    font-weight: 700;
    font-size: 1.1rem;
    margin: 0 0.4rem 1rem;
  }
  .rail button {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    text-align: left;
    justify-content: flex-start;
  }
  .rail button[aria-current="page"] {
    background: var(--accent-soft);
    border-color: var(--accent);
  }
  .icon {
    width: 1.2rem;
    text-align: center;
  }
  .ride-btn {
    margin-top: 0.8rem;
    justify-content: space-between !important;
  }
  .bottom {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  main {
    flex: 1;
    min-width: 0;
    overflow: auto;
  }
</style>
