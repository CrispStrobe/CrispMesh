// Runs the photos start of "This browser" through the app's own screens in headless Chrome
// with WebGPU: picks the photos and an included lens calibration, starts with threshold masks
// and turntable cameras, waits, and reports the stages, the sheets, the surface and the
// downloaded STL.
//
//   STUDIO_PHOTOS=/path/to/photos [STUDIO_URL=http://127.0.0.1:1431/] node scripts/browser-photos-check.mjs
//
// Uses the installed Google Chrome (channel "chrome"). STUDIO_OUT: folder for screenshots.

import { readdirSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { chromium } from "playwright";

const base = process.env.STUDIO_URL ?? "http://127.0.0.1:1431/";
const photos = process.env.STUDIO_PHOTOS;
const out = process.env.STUDIO_OUT ?? "";
if (!photos) {
  console.error("STUDIO_PHOTOS is needed: a folder of turntable photos.");
  process.exit(2);
}
const files = readdirSync(photos).filter((name) => /\.(png|jpe?g)$/i.test(name)).map((name) => join(photos, name));
const launch = ["--enable-unsafe-webgpu", "--enable-features=WebGPU"];
if (process.platform === "darwin") launch.push("--use-angle=metal");
const browser = await chromium.launch({ channel: "chrome", headless: true, args: launch });
const page = await browser.newPage({ locale: "en-US", viewport: { width: 1440, height: 900 }, acceptDownloads: true });
const problems = [];
page.on("pageerror", (error) => problems.push(`page error: ${error.message}`));
page.on("console", (message) => message.type() === "error" && problems.push(`console: ${message.text()}`));
const shot = async (name) => out !== "" && page.screenshot({ path: resolve(out, name), type: "jpeg", quality: 70 });

try {
  await page.goto(base + "#/");
  // On the first visit the isolation service worker reloads the page once.
  await page.waitForFunction(() => globalThis.crossOriginIsolated, null, { timeout: 15000 }).catch(() => undefined);
  await page.waitForLoadState("load");
  console.log("cross-origin isolated:", await page.evaluate(() => globalThis.crossOriginIsolated), "|", (await page.locator("section.card.featured [role=note]").innerText().catch(() => "(no note)")).trim());
  await page.getByRole("link", { name: /Start in this browser|Use this browser|Open runs/ }).first().click();
  await page.getByRole("heading", { name: "Runs", level: 1 }).waitFor();
  await page.locator(".page-head .sub", { hasText: "default device" }).waitFor({ timeout: 60000 }).catch(() => undefined);
  console.log("engine:", (await page.locator(".page-head .sub").innerText()).replace(/\s+/g, " "));
  await page.getByRole("link", { name: /New run|Start the first run/ }).first().click();
  await page.locator("#s-sizes").waitFor({ timeout: 60000 });
  await page.locator(".segmented button", { hasText: "Turntable photos" }).click();
  await page.locator("#f-photos-files").setInputFiles(files);
  await page.locator(".picked", { hasText: "photos" }).first().waitFor();
  console.log("photos:", await page.locator(".picked").first().innerText());
  await page.locator(".calibrations button").first().click();
  await page.locator(".picked", { hasText: "Calibration" }).waitFor();
  console.log("calibration:", await page.locator(".picked", { hasText: "Calibration" }).innerText());
  console.log("providers:", (await page.locator(".provider-option").allInnerTexts()).map((text) => text.split("\n")[0]).join(" | "));
  await shot("photos-form.jpg");
  const started = Date.now();
  await page.getByRole("button", { name: "Start run" }).click();
  await page.waitForURL(/#\/engine\/run\//, { timeout: 60000 });
  let last = "";
  const watch = setInterval(async () => {
    const line = (await page.locator(".stage-running").first().innerText().catch(() => "")).replace(/\s+/g, " ").trim();
    const head = line.split(" ").slice(0, 2).join(" ");
    if (line !== "" && head !== last) {
      last = head;
      console.log(`  ${((Date.now() - started) / 1000).toFixed(0)} s: ${line}`);
    }
  }, 3000);
  await page.locator(".run-meta .badge.status-complete, .run-meta .badge.status-failed").waitFor({ timeout: 1800000 });
  clearInterval(watch);
  const status = await page.locator(".run-meta .badge").first().innerText();
  console.log(`run ${status} after ${((Date.now() - started) / 1000).toFixed(1)} s | ${(await page.locator(".run-meta").innerText()).replace(/\s+/g, " ")}`);
  if (status !== "Complete") throw new Error("the run did not complete: " + (await page.locator(".notice").first().innerText().catch(() => "")));
  console.log("stages:", (await page.locator(".stage").allInnerTexts()).map((text) => text.replace(/\s+/g, " ")).join(" | "));
  const load = page.getByRole("button", { name: /^Load final/ });
  if ((await load.count()) > 0) await load.click();
  await page.locator("#mesh-heading + .sub", { hasText: "Final surface" }).waitFor({ timeout: 300000 });
  console.log("surface:", await page.locator("#mesh-heading + .sub").innerText());
  console.log("sheets:", (await page.locator(".sheet-label").allInnerTexts()).join(" | "));
  await shot("photos-done.jpg");
  const waiting = page.waitForEvent("download");
  await page.getByRole("button", { name: /^Download STL/ }).click();
  const download = await waiting;
  const bytes = statSync(await download.path()).size;
  console.log(`downloaded ${download.suggestedFilename()}: ${bytes} bytes = ${(bytes - 84) / 50} triangles`);
} catch (problem) {
  problems.push(String(problem?.message ?? problem).split("\n")[0]);
  await shot("photos-problem.jpg");
} finally {
  await browser.close();
}
if (problems.length > 0) {
  console.error("Problems:\n  " + [...new Set(problems)].join("\n  "));
  process.exitCode = 1;
}
