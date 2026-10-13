import { For } from "solid-js";
import { Portal } from "solid-js/web";

export function HelpOverlay(props: { title: string; shortcuts: [string, string][]; onClose: () => void }) {
  return (
    <Portal>
      <div class="overlay" role="dialog" aria-modal="true" aria-label={props.title} onClick={() => props.onClose()}>
        <div class="overlay-card" onClick={(e) => e.stopPropagation()}>
          <h2>{props.title}</h2>
          <dl class="shortcuts">
            <For each={props.shortcuts}>
              {([k, d]) => (
                <>
                  <dt>
                    <kbd>{k}</kbd>
                  </dt>
                  <dd>{d}</dd>
                </>
              )}
            </For>
          </dl>
          <button class="primary" onClick={() => props.onClose()}>
            Close (Esc)
          </button>
        </div>
      </div>
    </Portal>
  );
}
