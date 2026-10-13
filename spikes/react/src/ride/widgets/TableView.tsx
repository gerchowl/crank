export function TableView({ head, rows }: { head: string[]; rows: (string | number)[][] }) {
  return (
    <div className="table-view">
      <table>
        <thead>
          <tr>{head.map((h) => <th key={h}>{h}</th>)}</tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i}>{r.map((c, j) => <td key={j}>{c}</td>)}</tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function TableToggle({ on, onChange }: { on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button type="button" className={`icon-btn ${on ? "on" : ""}`} title="Table view" aria-pressed={on} onClick={() => { onChange(!on); }}>
      ▦
    </button>
  );
}
