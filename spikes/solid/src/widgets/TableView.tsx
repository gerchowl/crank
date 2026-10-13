import { For } from "solid-js";

/** Accessible table view behind every analytics plot (exact values). */
export function TableView(props: { head: string[]; rows: (string | number)[][]; caption: string }) {
  return (
    <div class="table-view">
      <table>
        <caption>{props.caption}</caption>
        <thead>
          <tr>
            <For each={props.head}>{(h) => <th>{h}</th>}</For>
          </tr>
        </thead>
        <tbody>
          <For each={props.rows}>
            {(r) => (
              <tr>
                <For each={r}>{(c) => <td>{c}</td>}</For>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}
