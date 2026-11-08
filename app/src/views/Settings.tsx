// Settings (DESIGN §5.9) in direction A (§18, #527): on the 12-column grid (§18.2), the section list (3 columns) and
// the open section in one panel (9 columns). A section is a heading and its rows; a row is a label and its control, so a
// later setting is one more row, and a later section one more entry in `SECTIONS`. Devices is the pairing section (#92);
// Appearance is the theme choice (THEMES.md): the manual choice overrides Auto. Controls are 32 px (§18.4).
import { Bluetooth, SunMoon, type LucideIcon } from "lucide-react";
import { navigate } from "../router";
import {
  THEME_PREFS,
  setThemePref,
  useThemePref,
  type ThemePref,
} from "../theme";
import { Devices } from "./Devices";
import "./settings.css";

const LABEL: Record<ThemePref, string> = {
  auto: "Auto (follow system)",
  dark: "Dark",
  light: "Light",
};

type Section = "devices" | "appearance";
const SECTIONS: readonly [Section, string, LucideIcon][] = [
  ["devices", "Devices", Bluetooth],
  ["appearance", "Appearance", SunMoon],
];
const icon = { size: 16, strokeWidth: 1.5, "aria-hidden": true } as const;

function Appearance() {
  const { pref, notice } = useThemePref();
  return (
    <section className="settings-section" aria-labelledby="appearance-h">
      <h2 id="appearance-h">Appearance</h2>
      <div className="setting-row">
        <span id="theme-l" className="setting-label">
          Mode
        </span>
        <div
          className="theme-choice"
          role="radiogroup"
          aria-labelledby="theme-l"
        >
          {THEME_PREFS.map((p) => (
            <label key={p}>
              <input
                type="radio"
                name="theme"
                value={p}
                checked={pref === p}
                onChange={() => setThemePref(p)}
              />
              {LABEL[p]}
            </label>
          ))}
        </div>
      </div>
      {notice && (
        <p className="settings-note" role="status">
          {notice}
        </p>
      )}
    </section>
  );
}

export function Settings({ sub }: { sub?: string }) {
  const section: Section = sub === "devices" ? "devices" : "appearance";
  return (
    <div className="page settings">
      <h1>Settings</h1>
      <div className="settings-body">
        <nav className="settings-nav" aria-label="Settings sections">
          {SECTIONS.map(([id, label, Icon]) => (
            <button
              key={id}
              type="button"
              aria-current={section === id ? "true" : undefined}
              onClick={() => navigate("settings", id)}
            >
              <Icon {...icon} />
              {label}
            </button>
          ))}
        </nav>
        <div className="settings-panel">
          {section === "devices" ? <Devices /> : <Appearance />}
        </div>
      </div>
    </div>
  );
}
