// The labelled icon rail (DESIGN §16.3): icon over its word, never an icon alone.
import { navigate, PLACES, type Place, type View } from "../router";

const RAIL: Record<Place, [label: string, icon: string]> = {
  today: ["Today", "◉"],
  workouts: ["Workouts", "▤"],
  history: ["History", "◷"],
  settings: ["Settings", "⚙"],
};

export function Rail({ view }: { view: View }) {
  return (
    <nav className="rail" aria-label="Places">
      <span className="brand">crank</span>
      {PLACES.map((p) => (
        <button
          key={p}
          type="button"
          aria-current={view === p ? "page" : undefined}
          onClick={() => navigate(p)}
        >
          <span className="rail-icon" aria-hidden>
            {RAIL[p][1]}
          </span>
          <span className="rail-label">{RAIL[p][0]}</span>
        </button>
      ))}
    </nav>
  );
}
