import { createSignal, For, onCleanup, Show } from "solid-js";
import { type Mode, type Preset, theme } from "../theme/theme";

const MODES: Mode[] = ["dark", "light", "auto"];
const PRESETS: Preset[] = ["graphite", "ocean", "ember"];

/** Settings popover: mode, preset, classic zones. Switching is live (canvases redraw). */
export function ThemeMenu() {
  const [open, setOpen] = createSignal(false);
  let root: HTMLDivElement | undefined;
  const onDoc = (e: MouseEvent) => {
    if (root && !root.contains(e.target as Node)) setOpen(false);
  };
  document.addEventListener("mousedown", onDoc);
  onCleanup(() => document.removeEventListener("mousedown", onDoc));

  return (
    <div class="popover-host" ref={(el) => (root = el)}>
      <button class="ghost" aria-expanded={open()} onClick={() => setOpen(!open())} title="Theme (T)">
        ◐ Theme
      </button>
      <Show when={open()}>
        <div class="popover" role="dialog" aria-label="Theme settings">
          <div class="pop-row">
            <span class="pop-label">Mode</span>
            <span class="seg">
              <For each={MODES}>
                {(m) => (
                  <button classList={{ on: theme.mode() === m }} onClick={() => theme.setMode(m)}>
                    {m}
                  </button>
                )}
              </For>
            </span>
          </div>
          <div class="pop-row">
            <span class="pop-label">Preset</span>
            <span class="seg">
              <For each={PRESETS}>
                {(p) => (
                  <button classList={{ on: theme.preset() === p }} onClick={() => theme.setPreset(p)}>
                    <span class="preset-swatch" data-preset={p} />
                    {p}
                  </button>
                )}
              </For>
            </span>
          </div>
          <div class="pop-row">
            <label class="toggle">
              <input type="checkbox" checked={theme.classicZones()} onChange={(e) => theme.setClassicZones(e.currentTarget.checked)} />
              Classic rainbow zones
            </label>
            <Show when={theme.classicZones()}>
              <span class="muted small">not colour-blind safe</span>
            </Show>
          </div>
          <div class="zone-preview">
            <For each={[1, 2, 3, 4, 5, 6, 7]}>
              {(i) => (
                <span style={{ background: `var(--zone-${i})` }}>
                  <b>Z{i}</b>
                </span>
              )}
            </For>
          </div>
        </div>
      </Show>
    </div>
  );
}
