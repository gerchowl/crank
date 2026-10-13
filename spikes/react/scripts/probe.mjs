// dev helper: node scripts/probe.mjs <url> <out.png> [waitMs]
import { chromium } from "playwright";
const [url, out, wait = "4000"] = process.argv.slice(2);
const b = await chromium.launch();
const p = await b.newPage({ viewport: { width: 1440, height: 900 } });
p.on("console", (m) => { if (m.type() === "error" || m.type() === "warning") console.log("console:", m.type(), m.text()); });
p.on("pageerror", (e) => console.log("pageerror:", e.message));
await p.goto(url);
await p.waitForTimeout(Number(wait));
await p.screenshot({ path: out });
await b.close();
