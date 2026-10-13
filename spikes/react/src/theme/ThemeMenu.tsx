import { useState } from "react";
import { themePrefs, useThemePrefs, type ThemeMode, type ThemeName } from "./theme";

const THEMES: ThemeName[] = ["graphite", "ocean", "ember"];
const MODES: ThemeMode[] = ["dark", "light", "auto"];

export function ThemeMenu() {
  const p = useThemePrefs();
  const [open, setOpen] = useState(false);
  return (
    <div className="popover-anchor">
      <button type="button" className="btn" aria-expanded={open} onClick={() => { setOpen(!open); }} title="Theme" aria-label="Theme">
        ◑<span className="lbl"> Theme</span>
      </button>
      {open && (
        <div className="popover" role="dialog" aria-label="Theme">
          <fieldset>
            <legend>Mode</legend>
            <div className="seg">
              {MODES.map((m) => (
                <button type="button" key={m} className={m === p.mode ? "on" : ""} onClick={() => { themePrefs.set({ ...p, mode: m }); }}>
                  {m}
                </button>
              ))}
            </div>
          </fieldset>
          <fieldset>
            <legend>Preset</legend>
            <div className="seg">
              {THEMES.map((t) => (
                <button type="button" key={t} className={t === p.theme ? "on" : ""} onClick={() => { themePrefs.set({ ...p, theme: t }); }}>
                  {t}
                </button>
              ))}
            </div>
          </fieldset>
          <label className="check">
            <input type="checkbox" checked={p.classicZones} onChange={(e) => { themePrefs.set({ ...p, classicZones: e.target.checked }); }} />
            Classic rainbow zones <span className="muted small">(not colour-blind safe)</span>
          </label>
          <div className="zone-preview" aria-hidden>
            {[1, 2, 3, 4, 5, 6, 7].map((z) => (
              <span key={z} style={{ background: `var(--zone-${String(z)})` }}>Z{z}</span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
