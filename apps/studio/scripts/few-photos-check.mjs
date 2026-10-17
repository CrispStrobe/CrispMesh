// The "New run" form with few photos, in headless Chrome with the browser engine: below 25 photos per turn it says
// the ordinary mesh will be rough and switches the neural surface on at fast once; set back to Off it stays off
// (also after other few photos); with 25 or more photos neither happens. Also reads the photo counts "Take photos"
// offers (a fake camera). Nothing is reconstructed.
//
//   npx vite --port 1431 &
//   STUDIO_PHOTOS=.../bunny/rgb node scripts/few-photos-check.mjs
//
// Environment:
//   STUDIO_URL     where the app is served (default http://127.0.0.1:1431/)
//   STUDIO_PHOTOS  a folder of one turn's photos in capture order, at least 60 (sorted by the number in the name)
//   STUDIO_CHANNEL browser channel (default chrome)
//   STUDIO_OUT     folder for screenshots (optional)

import { readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { chromium } from "playwright";

const base = process.env.STUDIO_URL ?? "http://127.0.0.1:1431/";
const folder = process.env.STUDIO_PHOTOS;
const out = process.env.STUDIO_OUT ?? "";
const number = (name) => Number(/(\d+)(?!.*\d)/.exec(name)?.[1] ?? 0);
const all = folder
  ? readdirSync(folder)
      .filter((name) => /\.(png|jpe?g)$/i.test(name))
      .sort((a, b) => number(a) - number(b) || a.localeCompare(b))
      .map((name) => join(folder, name))
  : [];
if (all.length < 60) {
  console.error("STUDIO_PHOTOS (a folder with at least 60 photos of one turn) is needed.");
  process.exit(2);
}
const every = (step, offset, count) => all.filter((_, index) => index % step === offset).slice(0, count);
const nineteen = every(4, 0, 19);
const twenty = every(3, 1, 20);
const many = every(2, 0, 37);

const launch = ["--enable-unsafe-webgpu", "--enable-features=WebGPU", "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"];
if (process.platform === "darwin") launch.push("--use-angle=metal");
const browser = await chromium.launch({ channel: process.env.STUDIO_CHANNEL ?? "chrome", headless: true, args: launch });
const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, permissions: ["camera"] });
const problems = [];
const HINT = "Fewer than 25 photos per turn: the ordinary mesh will be rough; the neural surface (fast) copes with about 20.";
const SWITCHED = "It was switched on at fast for these photos";

async function newForm() {
  const page = await context.newPage();
  page.on("pageerror", (error) => problems.push(`page error: ${error.message}`));
  page.on("console", (message) => message.type() === "error" && problems.push(`console: ${message.text()}`));
  await page.goto(base + "#/");
  const card = page.locator("section.card", { has: page.getByRole("heading", { name: /This browser|Reconstruct in this browser/ }) });
  await card.getByRole("link", { name: /Start in this browser|Use this browser|Open runs/ }).click();
  await page.getByRole("heading", { name: "Runs", level: 1 }).waitFor();
  await page.getByRole("link", { name: /New run|Start the first run/ }).first().click();
  await page.locator("fieldset.methods").waitFor({ timeout: 60_000 });
  await page.locator(".segmented button", { hasText: "Turntable photos" }).click();
  return page;
}

async function state(page, label, shotName) {
  await page.waitForTimeout(300);
  const methods = page.locator("fieldset.methods");
  const notice = methods.locator(".notice.warn");
  const hint = (await notice.count()) > 0 ? (await notice.first().innerText()).replace(/\s+/g, " ").trim() : "";
  const surface = methods.locator("#f-method-surface");
  const quality = await surface.evaluate((select) => select.options[select.selectedIndex]?.text ?? "");
  const picked = ((await page.locator(".picked").first().innerText().catch(() => "")) || "").replace(/\s+/g, " ");
  console.log(`${label}: picked "${picked.slice(0, 60)}" | surface "${quality}" | hint "${hint}"`);
  if (out !== "") await methods.screenshot({ path: resolve(out, shotName) });
  return { hint, quality, switched: hint.includes(SWITCHED) };
}

const expect = (condition, message) => {
  if (!condition) problems.push(message);
};

try {
  // 1: 19 photos -> hint, surface switched on at fast.
  let page = await newForm();
  const surfaceSelect = page.locator("#f-method-surface");
  await page.locator("#f-photos-files").setInputFiles(nineteen);
  let seen = await state(page, "19 photos", "1-nineteen.png");
  expect(seen.hint.startsWith(HINT), "19 photos: no hint");
  expect(/fast/i.test(seen.quality) && seen.switched, "19 photos: the surface was not switched on at fast");

  // 2: set Off -> stays off, also after 20 other photos; the hint stays.
  await surfaceSelect.selectOption("");
  seen = await state(page, "set Off", "2a-off.png");
  expect(/off/i.test(seen.quality) && !seen.switched, "set Off: not off, or still says it was switched on");
  await page.locator("#f-photos-files").setInputFiles(twenty);
  seen = await state(page, "20 other photos", "2b-twenty.png");
  expect(seen.hint.startsWith(HINT), "20 photos: no hint");
  expect(/off/i.test(seen.quality) && !seen.switched, "20 photos after Off: the surface was switched on again");
  await page.close();

  // 3: fresh form, 37 photos -> no hint, surface off.
  page = await newForm();
  await page.locator("#f-photos-files").setInputFiles(many);
  seen = await state(page, "37 photos", "3-many.png");
  expect(seen.hint === "", "37 photos: a hint is shown");
  expect(/off/i.test(seen.quality), "37 photos: the surface is not off");

  // 4: Take photos offers 20 with the neural surface.
  await page.getByRole("button", { name: "Take photos" }).click();
  const dialog = page.locator("dialog.capture");
  await dialog.waitFor();
  const options = await dialog.getByLabel("Photos for one turn").locator("option").allInnerTexts();
  console.log("Take photos offers:", options.join(" | "));
  expect(options.includes("20 photos (18° steps), with the neural surface"), "Take photos: no 20-photo choice with the neural surface");
  if (out !== "") await page.screenshot({ path: resolve(out, "4-capture.png") });
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await page.close();
} catch (problem) {
  problems.push(String(problem?.message ?? problem).split("\n")[0]);
} finally {
  await Promise.race([browser.close(), new Promise((done) => setTimeout(done, 20_000))]);
}
if (problems.length > 0) {
  console.error("Problems:\n  " + [...new Set(problems)].join("\n  "));
  process.exitCode = 1;
} else console.log("All as intended.");
process.exit();
