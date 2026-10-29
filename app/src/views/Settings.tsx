// Settings (DESIGN §5.9). Devices is the pairing section (#92). Appearance
// is the theme choice (THEMES.md): the manual choice overrides Auto.
import { navigate } from "../router";
import {
  THEME_PREFS,
  setThemePref,
  useThemePref,
  type ThemePref,
} from "../theme";
import { Devices } from "./Devices";

const LABEL: Record<ThemePref, string> = {
  auto: "Auto (follow system)",
  dark: "Dark",
  light: "Light",
};
const SECTIONS = [
  ["devices", "Devices"],
  ["appearance", "Appearance"],
] as const;

export function Settings({ sub }: { sub?: string }) {
  const section = sub === "devices" ? "devices" : "appearance";
  const { pref, notice } = useThemePref();
  return (
    <>
      <nav className="section-nav" aria-label="Settings sections">
        {SECTIONS.map(([id, label]) => (
          <button
            key={id}
            type="button"
            aria-current={section === id ? "true" : undefined}
            onClick={() => navigate("settings", id)}
          >
            {label}
          </button>
        ))}
      </nav>
      {section === "devices" ? (
        <Devices />
      ) : (
        <div className="page">
          <h1>Appearance</h1>
          <fieldset className="theme-choice">
            <legend>Mode</legend>
            {THEME_PREFS.map((p) => (
              <label key={p}>
                <input
                  type="radio"
                  name="theme"
                  value={p}
                  checked={pref === p}
                  onChange={() => setThemePref(p)}
                />{" "}
                {LABEL[p]}
              </label>
            ))}
          </fieldset>
          {notice && (
            <p className="muted" role="status">
              {notice}
            </p>
          )}
        </div>
      )}
    </>
  );
}
