// Settings (DESIGN §5.9) in direction A (§18, #527): on the 12-column grid (§18.2), the section list (3 columns) and
// the open section in one panel (9 columns). A section is a heading and its rows; a row is a label and its control, so a
// later setting is one more row, and a later section one more entry in `SECTIONS`. Devices is the pairing section (#92);
// Appearance is the theme choice (THEMES.md): the manual choice overrides Auto. Controls are 32 px (§18.4).
// Athlete model shows the heart-rate references the ride's HR zone reads (§18.3 "Heart-rate zone", #586) and, with
// neither known, the one-line hint; its estimates and manual overrides are #113.
import { Bluetooth, HeartPulse, SunMoon, type LucideIcon } from "lucide-react";
import { useAthleteHr } from "../athleteHr";
import { Num } from "../Num";
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

type Section = "devices" | "athlete" | "appearance";
const SECTIONS: readonly [Section, string, LucideIcon][] = [
  ["devices", "Devices", Bluetooth],
  ["athlete", "Athlete model", HeartPulse],
  ["appearance", "Appearance", SunMoon],
];
const icon = { size: 16, strokeWidth: 1.5, "aria-hidden": true } as const;

/** The ride's HR-zone hint (§18.3 "Heart-rate zone"): one line, shown while neither reference is known. */
export const NO_HR_REFS =
  "No max or threshold heart rate yet, so the ride shows no heart-rate zone.";

function Athlete() {
  const { hrMaxBpm, lthrBpm } = useAthleteHr();
  const rows: [string, string, number | null][] = [
    ["hrmax", "Max heart rate", hrMaxBpm],
    ["lthr", "Threshold heart rate (LTHR)", lthrBpm],
  ];
  return (
    <section className="settings-section" aria-labelledby="athlete-h">
      <h2 id="athlete-h">Athlete model</h2>
      <div className="setting-rows">
        {rows.map(([id, label, bpm]) => (
          <div key={id} className="setting-row">
            <span className="setting-label">{label}</span>
            <span data-testid={id}>
              {bpm === null ? "Not set" : <Num>{`${bpm} bpm`}</Num>}
            </span>
          </div>
        ))}
      </div>
      {hrMaxBpm === null && lthrBpm === null && (
        <p className="settings-note">{NO_HR_REFS}</p>
      )}
    </section>
  );
}

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
  const section: Section =
    sub === "devices" || sub === "athlete" ? sub : "appearance";
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
          {section === "devices" ? (
            <Devices />
          ) : section === "athlete" ? (
            <Athlete />
          ) : (
            <Appearance />
          )}
        </div>
      </div>
    </div>
  );
}
