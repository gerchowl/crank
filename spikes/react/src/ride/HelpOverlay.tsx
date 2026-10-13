const KEYS: [string, string][] = [
  ["← / →", "previous / next page (or swipe)"],
  ["+ / −", "manual intensity ±5 %"],
  ["S", "skip block"],
  ["Space", "pause / resume"],
  ["Y / N", "accept / decline coach nudge"],
  ["A", "toggle auto page by phase"],
  ["E", "edit layout"],
  ["1 / 2 / 3", "speed 1× / 10× / 60×"],
  ["?", "this help"],
  ["Esc", "close"],
];
const EDITOR_KEYS: [string, string][] = [
  ["⌘/Ctrl Z", "undo"],
  ["⇧⌘/Ctrl Z", "redo"],
  ["⌘/Ctrl G", "wrap selection in repeat"],
  ["⌘/Ctrl D", "duplicate"],
  ["⌫ / Del", "delete selection"],
  ["⇧-click", "extend selection"],
];

export function HelpOverlay({ onClose }: { onClose: () => void }) {
  return (
    <div className="overlay" role="dialog" aria-label="Keyboard shortcuts" onClick={onClose}>
      <div className="overlay-card" onClick={(e) => { e.stopPropagation(); }}>
        <h2>Keyboard shortcuts</h2>
        <div className="help-cols">
          <dl>
            <dt className="h">Ride</dt>
            <dd />
            {KEYS.map(([k, d]) => (
              <div key={k} className="kv"><dt><kbd>{k}</kbd></dt><dd>{d}</dd></div>
            ))}
          </dl>
          <dl>
            <dt className="h">Editor</dt>
            <dd />
            {EDITOR_KEYS.map(([k, d]) => (
              <div key={k} className="kv"><dt><kbd>{k}</kbd></dt><dd>{d}</dd></div>
            ))}
          </dl>
        </div>
        <button type="button" className="btn primary" onClick={onClose}>Close</button>
      </div>
    </div>
  );
}
