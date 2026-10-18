// Plays the demo recording at every replay speed in headless Chromium and reports, for each,
// which surface is on screen at the end (label, triangle count, what the viewer holds) and a
// fingerprint of the rendered picture. The end state must be the same at every speed.
//
//   npm run build && npm run preview &
//   node scripts/replay-speeds.mjs [out-dir-for-screenshots]

import { createHash } from "node:crypto";
import { chromium } from "playwright";

const base = process.env.STUDIO_URL ?? "http://127.0.0.1:1431/";
const out = process.argv[2] ?? "";
const browser = await chromium.launch();
const results = [];
for (const [speed, stored] of [["1×", 1], ["4×", 4], ["16×", 16], ["Instant", 0]]) {
  const page = await browser.newPage({ locale: "en-US", viewport: { width: 1280, height: 900 } });
  // The replay starts at the remembered speed: set it before the page loads, so the whole
  // run is played at that speed from its first event.
  await page.addInitScript((value) => localStorage.setItem("crispmesh.studio.v1", JSON.stringify({ replaySpeed: value })), stored);
  await page.goto(base + "#/replay");
  await page.getByRole("group", { name: "Playback speed" }).waitFor();
  await page.locator(".run-meta .badge.status-complete").waitFor({ timeout: 300000 });
  await page.waitForTimeout(2500);
  const label = (await page.locator("#mesh-heading + .sub").innerText()).trim();
  const pressed = (await page.locator(".mesh-strip [aria-pressed='true'], .steps [aria-pressed='true']").allInnerTexts()).join(" ").replace(/\s+/g, " ");
  const canvas = page.locator(".viewer canvas");
  const picture = await canvas.screenshot();
  const fingerprint = createHash("sha256").update(picture).digest("hex").slice(0, 12);
  if (out !== "") await canvas.screenshot({ path: `${out}/replay-${speed.replace("×", "x")}.png` });
  results.push({ speed, label, pressed, fingerprint });
  console.log(`${speed.padEnd(8)} ${label} | strip: ${pressed} | picture ${fingerprint}`);
  await page.close();
}
await browser.close();
const distinct = new Set(results.map((row) => `${row.label}|${row.fingerprint}`));
if (distinct.size !== 1) {
  console.error("The end state differs between speeds.");
  process.exitCode = 1;
} else console.log("Same surface and the same picture at every speed.");
