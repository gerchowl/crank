// pnpm video — F · Spin-up as H.264 video in docs/design/brand/ (BRAND.md §F · Spin-up): real time at 60 fps
// for both themes, plus a 4× slow-motion cut for review in both themes. Every frame is the deterministic ?t= frame,
// so the video is exactly what the storyboard shows. Needs ffmpeg on PATH, which the devShell does not carry:
//   nix develop -c nix shell nixpkgs#ffmpeg-headless -c pnpm -C design/brand video
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
import { SPIN } from "../src/spin.ts";

const root = fileURLToPath(new URL("..", import.meta.url));
const out = fileURLToPath(new URL("../../../docs/design/brand/", import.meta.url));
const FPS = 60;
const TAIL = 400; // ms held on the settled wordmark

const server = await createServer({ root, logLevel: "warn", server: { port: 0 } });
await server.listen();
const base = server.resolvedUrls?.local[0];
if (!base) throw new Error("vite did not report a local URL");
const browser = await chromium.launch();

async function render(file: string, theme: "dark" | "light", slow: number) {
  const ff = spawn("ffmpeg", ["-y", "-loglevel", "error", "-f", "image2pipe", "-framerate", String(FPS), "-i", "-",
    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18", "-movflags", "+faststart", out + file], { stdio: ["pipe", "inherit", "inherit"] });
  const done = new Promise<void>((ok, fail) => ff.on("close", (code) => (code === 0 ? ok() : fail(new Error(`ffmpeg exited ${code}`)))));
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, reducedMotion: "no-preference" });
  const frames = Math.round(((SPIN.end + TAIL) / 1000) * FPS * slow);
  for (let i = 0; i <= frames; i++) {
    await page.goto(`${base}?concept=spin&theme=${theme}&t=${((i * 1000) / FPS / slow).toFixed(3)}`);
    await page.waitForSelector(".start[data-t]");
    if (!ff.stdin.write(await page.screenshot())) await new Promise((r) => ff.stdin.once("drain", r));
  }
  ff.stdin.end();
  await done;
  await page.close();
  console.log(`  ${file} (${frames + 1} frames)`);
}

try {
  await render("spin.1440x900.dark.mp4", "dark", 1);
  await render("spin.1440x900.light.mp4", "light", 1);
  await render("spin.slowmo-4x.1440x900.dark.mp4", "dark", 4);
  await render("spin.slowmo-4x.1440x900.light.mp4", "light", 4);
} finally {
  await browser.close();
  await server.close();
}
