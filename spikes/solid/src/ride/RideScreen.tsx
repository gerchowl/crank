import { createSignal, onMount, Show } from "solid-js";
import { createKeydown } from "../lib/primitives";
import { layoutState } from "../state/layout";
import { latestNudge, ride } from "../state/ride";
import { hashParam } from "../state/router";
import { HelpOverlay } from "../ui/HelpOverlay";
import { LayoutEditor } from "./LayoutEditor";
import { TopBar } from "./TopBar";
import { WidgetGrid } from "./WidgetGrid";

export const RIDE_SHORTCUTS: [string, string][] = [
  ["Space", "Pause / resume"],
  ["+ / −", "Manual ±5 %"],
  ["S", "Skip block"],
  ["← / →", "Previous / next page"],
  ["Y / N", "Accept / decline coach nudge"],
  ["1 / 2 / 3", "Speed 1× / 10× / 60×"],
  ["E", "Edit layout"],
  ["?", "This overlay"],
  ["Esc", "Close overlay / leave edit mode"],
];

export function RideScreen() {
  const [help, setHelp] = createSignal(false);

  onMount(() => {
    const sp = Number(hashParam("speed"));
    if (sp === 1 || sp === 10 || sp === 60) ride.controls.setSpeed(sp);
    if (ride.status.state === "idle") ride.controls.start();
  });

  createKeydown((e) => {
    const target = e.target as HTMLElement | null;
    if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT")) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const k = e.key;
    const handled = (() => {
      switch (k) {
        case " ":
          ride.controls.togglePause();
          return true;
        case "+":
        case "=":
          ride.controls.adjust(5);
          return true;
        case "-":
        case "_":
          ride.controls.adjust(-5);
          return true;
        case "s":
        case "S":
          ride.controls.skip();
          return true;
        case "ArrowRight":
          layoutState.actions.nextPage(1);
          return true;
        case "ArrowLeft":
          layoutState.actions.nextPage(-1);
          return true;
        case "y":
        case "Y":
        case "n":
        case "N": {
          const n = latestNudge();
          if (n?.status === "pending") ride.controls.respond(n.callId, k.toLowerCase() === "y");
          return true;
        }
        case "1":
          ride.controls.setSpeed(1);
          return true;
        case "2":
          ride.controls.setSpeed(10);
          return true;
        case "3":
          ride.controls.setSpeed(60);
          return true;
        case "e":
        case "E":
          layoutState.actions.setEditing(!layoutState.editing());
          return true;
        case "?":
          setHelp(!help());
          return true;
        case "Escape":
          if (help()) setHelp(false);
          else if (layoutState.editing()) layoutState.actions.setEditing(false);
          return true;
        default:
          return false;
      }
    })();
    if (handled) e.preventDefault();
  });

  // touch swipe → page change
  let swipeX: number | null = null;
  const onPointerDown = (e: PointerEvent) => {
    if (e.pointerType !== "mouse" && !layoutState.editing()) swipeX = e.clientX;
  };
  const onPointerUp = (e: PointerEvent) => {
    if (swipeX === null) return;
    const dx = e.clientX - swipeX;
    swipeX = null;
    if (Math.abs(dx) > 80) layoutState.actions.nextPage(dx < 0 ? 1 : -1);
  };

  return (
    <div class="ride" data-phase={ride.status.phase} data-state={ride.status.state} data-block={ride.status.blockIndex} data-page={layoutState.page()?.id}>
      <TopBar onHelp={() => setHelp(true)} />
      <div class="ride-body">
        <main class="ride-main" onPointerDown={onPointerDown} onPointerUp={onPointerUp}>
          <Show when={layoutState.page()} keyed>
            {(page) => <WidgetGrid page={page} editing={layoutState.editing()} />}
          </Show>
        </main>
        <Show when={layoutState.editing()}>
          <LayoutEditor />
        </Show>
      </div>
      <Show when={help()}>
        <HelpOverlay title="Ride shortcuts" shortcuts={RIDE_SHORTCUTS} onClose={() => setHelp(false)} />
      </Show>
    </div>
  );
}
