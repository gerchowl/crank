// dev helper: node scripts/peek.mjs <url> <out.png> [waitMs] [mode] [keys,comma,separated] [waitSelector]
import { chromium } from "playwright";
const [url, out, waitMs = "4000", mode = "dark", keys = "", waitSel = ""] = process.argv.slice(2);
const b = await chromium.launch();
const p = await b.newPage({ viewport: { width: 1440, height: 900 } });
const errs = [];
p.on("console", (m) => { if (m.type() === "error" || m.type() === "warning") errs.push(m.type() + ": " + m.text()); });
p.on("pageerror", (e) => errs.push("pageerror: " + e.message));
await p.addInitScript((mode) => localStorage.setItem("neo-trainer.theme.v1", JSON.stringify({ mode, preset: "ocean", classicZones: false })), mode);
await p.goto(url);
if (waitSel) await p.waitForSelector(waitSel, { timeout: 120000 });
await p.waitForTimeout(Number(waitMs));
for (const k of keys.split(",").filter(Boolean)) { await p.keyboard.press(k); await p.waitForTimeout(300); }
await p.waitForTimeout(1500);
await p.screenshot({ path: out });
console.log(errs.slice(0, 15).join("\n") || "no console errors");
await b.close();
