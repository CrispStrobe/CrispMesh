// The hand-off: this page plays the phone. It connects to another computer's engine (a desktop app with "Let other
// devices use this computer" on), takes the photos with "Take photos" and Auto (a video file as the camera), sends
// them there, starts the run there with the automatic lens and follows it until the surface is shown here.
//
//   npm run build && npm run preview &
//   STUDIO_ENGINE=http://127.0.0.1:8766 STUDIO_TOKEN=... STUDIO_VIDEO=turntable.y4m STUDIO_POSITIONS=37 node scripts/handoff-check.mjs
//
// The video is what a camera on a stand sees (see the README): each turntable position held still for about a
// second, then the turn. Chrome plays it in a loop, so the check stops Auto after STUDIO_POSITIONS photos.
//
// Environment:
//   STUDIO_URL        where the built app is served (default http://127.0.0.1:1431/)
//   STUDIO_ENGINE     the computer's engine address; STUDIO_TOKEN its token
//   STUDIO_VIDEO      the camera video (.y4m or .mjpeg)
//   STUDIO_POSITIONS  photos to keep: the turntable positions in the video
//   STUDIO_CHANNEL    browser channel (default chrome)
//   STUDIO_OUT        folder for screenshots (optional)
//   STUDIO_TIMEOUT    seconds to wait for the run (default 1800)

import { resolve } from "node:path";
import { chromium } from "playwright";

const base = process.env.STUDIO_URL ?? "http://127.0.0.1:1431/";
const engine = process.env.STUDIO_ENGINE ?? "http://127.0.0.1:8766";
const token = process.env.STUDIO_TOKEN ?? "";
const video = process.env.STUDIO_VIDEO;
const positions = Number(process.env.STUDIO_POSITIONS ?? 0);
const out = process.env.STUDIO_OUT ?? "";
const timeout = Number(process.env.STUDIO_TIMEOUT ?? 1800) * 1000;
if (!video || !(positions >= 8)) {
  console.error("STUDIO_VIDEO (a .y4m or .mjpeg file) and STUDIO_POSITIONS (at least 8) are needed.");
  process.exit(2);
}
const launch = [
  "--enable-unsafe-webgpu",
  "--enable-features=WebGPU",
  "--use-fake-device-for-media-stream",
  "--use-fake-ui-for-media-stream",
  `--use-file-for-fake-video-capture=${resolve(video)}`,
];
if (process.platform === "darwin") launch.push("--use-angle=metal");
const browser = await chromium.launch({ channel: process.env.STUDIO_CHANNEL ?? "chrome", headless: true, args: launch });
const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, permissions: ["camera"] });
const page = await context.newPage();
const problems = [];
page.on("pageerror", (error) => problems.push(`page error: ${error.message}`));
page.on("console", (message) => message.type() === "error" && problems.push(`console: ${message.text()}`));
const shot = async (name) => {
  if (out !== "") await page.screenshot({ path: resolve(out, name), type: "jpeg", quality: 70 });
};

try {
  await page.goto(base + "#/");
  const card = page.locator("section.card", { has: page.getByRole("heading", { name: /Another engine|^Engine$/ }) });
  await card.locator('input[type="url"]').fill(engine);
  await card.locator('input[type="password"]').fill(token);
  await card.getByRole("button", { name: "Connect" }).click();
  await page.getByRole("heading", { name: "Runs", level: 1 }).waitFor({ timeout: 30_000 });
  console.log("connected:", (await page.locator(".page-head .sub").innerText()).trim());
  await page.getByRole("link", { name: /New run|Start the first run/ }).first().click();
  await page.locator("#s-sizes").waitFor({ timeout: 60_000 });
  await page.locator(".segmented button", { hasText: "Turntable photos" }).click();
  await page.getByRole("button", { name: "Take photos" }).click();
  const dialog = page.locator("dialog.capture");
  await dialog.waitFor();
  await dialog.getByLabel("Photos for one turn").selectOption("48");
  const started = Date.now();
  const count = async () => Number((await dialog.locator(".lightbox-count").innerText()).split("/")[0]);
  let last = -1;
  while ((await count()) < positions) {
    const now = await count();
    if (now !== last) {
      last = now;
      if (now === 1) await shot("capture-first.jpg");
      console.log(`  ${((Date.now() - started) / 1000).toFixed(1)} s: ${now} photos | ${await dialog.locator(".capture-status").innerText()}`);
    }
    if (Date.now() - started > 600_000) throw new Error(`Auto took only ${now} photos in 10 minutes`);
    await page.waitForTimeout(200);
  }
  await dialog.locator(".capture-auto input[type=checkbox]").uncheck();
  while ((await count()) > positions) await dialog.getByRole("button", { name: "Delete last" }).click();
  console.log(`Auto took ${await count()} photos in ${((Date.now() - started) / 1000).toFixed(1)} s`);
  await shot("capture-done.jpg");
  await dialog.getByRole("button", { name: /^Use \d+ photos/ }).click();
  await dialog.waitFor({ state: "detached" });
  const sent = Date.now();
  await page.locator(".device-import [role=status]", { hasText: /photos saved in/ }).waitFor({ timeout: 300_000 });
  console.log(`sent in ${((Date.now() - sent) / 1000).toFixed(1)} s:`, await page.locator(".device-import [role=status]").innerText());
  console.log("photos field:", await page.locator("#f-photos").inputValue(), "| calibration:", await page.locator("#f-calibration").inputValue());
  await page.getByRole("button", { name: "Start run" }).click();
  await page.waitForURL(/#\/engine\/run\//, { timeout: 60_000 });
  const runStart = Date.now();
  await page.locator(".run-meta .badge.status-complete, .run-meta .badge.status-failed").waitFor({ timeout });
  const status = await page.locator(".run-meta .badge").first().innerText();
  console.log(`run ${status} after ${((Date.now() - runStart) / 1000).toFixed(1)} s | ${(await page.locator(".run-meta").innerText()).replace(/\s+/g, " ")}`);
  console.log("warnings:", (await page.locator(".notice").allInnerTexts()).map((text) => text.replace(/\s+/g, " ").slice(0, 300)).join(" | "));
  if (status !== "Complete") throw new Error("the run did not complete");
  const load = page.locator(".viewer-overlay button", { hasText: /^Load / });
  if (await load.isVisible().catch(() => false)) await load.click();
  await page.locator("#mesh-heading + .sub", { hasText: "Final surface" }).waitFor({ timeout: 300_000 });
  console.log("surface:", await page.locator("#mesh-heading + .sub").innerText());
  console.log("numbers:", (await page.locator(".metrics > div").allInnerTexts()).map((row) => row.replace(/\s+/g, " ")).filter((row) => /Registered|Input photos|Reprojection/i.test(row)).join(" | "));
  await shot("capture-run.jpg");
} catch (problem) {
  problems.push(String(problem?.message ?? problem).split("\n")[0]);
  await shot("capture-problem.jpg").catch(() => undefined);
} finally {
  await Promise.race([browser.close(), new Promise((done) => setTimeout(done, 20_000))]);
}
if (problems.length > 0) {
  console.error("Problems:\n  " + [...new Set(problems)].join("\n  "));
  process.exitCode = 1;
}
process.exit();
