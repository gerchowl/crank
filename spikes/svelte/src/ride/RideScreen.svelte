<script lang="ts">
  import { backend } from "../lib/backend";
  import { ride } from "../lib/ride.svelte";
  import { layout } from "../lib/layout.svelte";
  import { nav, isTyping } from "../lib/nav.svelte";
  import TopBar from "./TopBar.svelte";
  import WidgetGrid from "./WidgetGrid.svelte";
  import LayoutPanel from "./LayoutPanel.svelte";

  // Context-aware pages: follow the phase when it changes (not on every status heartbeat —
  // ride.phase is a derived, so this effect runs once per phase change).
  $effect(() => {
    layout.followPhase(ride.phase);
  });

  function onkeydown(e: KeyboardEvent): void {
    if (isTyping(e) || e.metaKey || e.ctrlKey || e.altKey) return;
    switch (e.key) {
      case " ":
        ride.togglePause();
        break;
      case "+":
      case "=":
        backend.ride.adjust(5);
        break;
      case "-":
      case "−":
      case "_":
        backend.ride.adjust(-5);
        break;
      case "s":
      case "S":
        backend.ride.skipBlock();
        break;
      case "y":
      case "Y":
        ride.respond(true);
        break;
      case "n":
      case "N":
        ride.respond(false);
        break;
      case "ArrowRight":
        layout.go(1);
        break;
      case "ArrowLeft":
        layout.go(-1);
        break;
      case "e":
      case "E":
        if (layout.editing) layout.cancelEdit();
        else layout.startEdit();
        break;
      case "1":
        backend.ride.setSpeed(1);
        break;
      case "2":
        backend.ride.setSpeed(10);
        break;
      case "3":
        backend.ride.setSpeed(60);
        break;
      case "?":
        nav.help = !nav.help;
        break;
      case "Escape":
        if (layout.editing) layout.cancelEdit();
        nav.help = false;
        break;
      default:
        return;
    }
    e.preventDefault();
  }
</script>

<svelte:window {onkeydown} />

<div class="ride" data-phase={ride.phase} data-block={ride.blockIndex} data-state={ride.state} data-page={layout.page?.id}>
  <TopBar />
  <div class="main">
    {#if layout.page}
      <WidgetGrid page={layout.page} />
    {/if}
    {#if layout.editing}
      <LayoutPanel />
    {/if}
  </div>
</div>

<style>
  .ride {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .main {
    flex: 1;
    min-height: 0;
    display: flex;
  }
</style>
