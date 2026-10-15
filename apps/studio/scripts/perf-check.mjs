// Loads a (large) replay bundle in headless Chromium and reports how the 3D view copes:
// what is loaded without asking, how long each surface takes to appear, and the longest
// time the page was unresponsive meanwhile.
//
//   STUDIO_URL=http://127.0.0.1:1431/ STUDIO_BUNDLE=big/ node scripts/perf-check.mjs
//
// STUDIO_BUNDLE is a bundle address as typed into the app (absolute, or relative to the app).

import { resolve } from "node:path";
import { chromium } from "playwright";

const base = process.env.STUDIO_URL ?? "http://127.0.0.1:1431/";
const bundle = process.env.STUDIO_BUNDLE ?? "demo/";
const out = process.env.STUDIO_OUT ?? "";

// STUDIO_GL=metal (macOS) or another ANGLE backend uses the real GPU; the default is software rendering, which works everywhere.
const gl = process.env.STUDIO_GL ?? "swiftshader";
const browser = await chromium.launch({ args: [`--use-angle=${gl}`, "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--enable-gpu"] });
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await context.addInitScript(() => {
  localStorage.setItem("crispmesh.studio.v1", JSON.stringify({ replaySpeed: 0 }));
  // Longest gap between 10 ms ticks = longest time the main thread could not respond.
  let last = performance.now();
  window.__stall = 0;
  setInterval(() => {
    const now = performance.now();
    window.__stall = Math.max(window.__stall, now - last - 10);
    last = now;
  }, 10);
});
const page = await context.newPage();
const failures = [];
page.on("pageerror", (error) => failures.push(error.message));

const shown = () => page.locator("#mesh-heading + .sub").innerText();
const waitFor = (label, timeout = 180_000) => page.locator("#mesh-heading + .sub", { hasText: label }).waitFor({ timeout });
const stall = async () => {
  const value = await page.evaluate(() => {
    const worst = window.__stall;
    window.__stall = 0;
    return worst;
  });
  return `${Math.round(value)} ms`;
};
const heap = () => page.evaluate(() => (performance.memory ? Math.round(performance.memory.usedJSHeapSize / 1e6) + " MB" : "n/a"));

const requested = [];
page.on("request", (request) => {
  if (request.url().endsWith(".stl")) requested.push(request.url().split("/").slice(-3).join("/"));
});

let started = Date.now();
await page.goto(`${base}#/replay?bundle=${encodeURIComponent(bundle)}`);
await page.locator(".step").first().waitFor({ timeout: 60_000 });
await page.locator("#mesh-heading + .sub", { hasText: "triangles" }).waitFor({ timeout: 180_000 });
console.log(`first surface on screen after ${Date.now() - started} ms: ${await shown()}; longest stall ${await stall()}`);
console.log("meshes downloaded without asking:", requested.join(", ") || "none");

const load = page.getByRole("button", { name: /^Load / });
if ((await load.count()) > 0) {
  console.log("final mesh waits for a click:", await load.innerText());
  started = Date.now();
  await load.click();
  await waitFor("Final surface");
  console.log(`final surface on screen after ${Date.now() - started} ms: ${await shown()}; longest stall ${await stall()}; heap ${await heap()}`);
} else {
  console.log("final mesh was small enough to load without asking");
  await waitFor("Final surface");
}

// Walk back and forth through every surface a few times: swaps must stay quick and memory flat.
const steps = await page.locator(".step").count();
for (let round = 0; round < 3; round++) {
  for (let index = 0; index < steps; index++) {
    const label = (await page.locator(".step .step-label").nth(index).innerText()).trim();
    started = Date.now();
    await page.locator(".step").nth(index).click();
    await waitFor(label);
    await page.waitForFunction((expected) => document.querySelector(".viewer-status") === null && document.querySelector("#mesh-heading + .sub")?.textContent?.startsWith(expected), label);
    if (round === 0 || round === 2) console.log(`  round ${round + 1}: ${label}: ${Date.now() - started} ms, stall ${await stall()}, heap ${await heap()}`);
  }
}

// Orbit while the largest mesh is shown.
await page.locator(".step").last().click();
await waitFor("Final surface");
const box = await page.locator(".viewer canvas").boundingBox();
started = Date.now();
await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
await page.mouse.down();
for (let i = 1; i <= 30; i++) await page.mouse.move(box.x + box.width / 2 + i * 8, box.y + box.height / 2 + i * 2);
await page.mouse.up();
console.log(`30 orbit moves on the final mesh: ${Date.now() - started} ms (GL: ${gl}), stall ${await stall()}`);

// The gallery with large sheets.
await page.locator(".gallery").scrollIntoViewIfNeeded();
await page.waitForLoadState("networkidle");
const images = await page.locator(".sheet img").evaluateAll((all) => all.map((image) => `${image.naturalWidth}x${image.naturalHeight}`));
console.log("sheets:", images.join(", "), "| stall", await stall());
await page.locator(".sheet-button").nth(3).click();
await page.locator(".lightbox-stage img:not(.loading)").waitFor();
await page.keyboard.press("1");
console.log("lightbox at 1:1 opened; stall", await stall());
if (out !== "") await page.screenshot({ path: resolve(out, "perf-lightbox.jpg"), type: "jpeg", quality: 70 });
await page.keyboard.press("Escape");
await page.evaluate(() => scrollTo(0, 0));
if (out !== "") await page.screenshot({ path: resolve(out, "perf-final.jpg"), type: "jpeg", quality: 70 });
if (out !== "") {
  await page.locator(".reports-panel").scrollIntoViewIfNeeded();
  await page.locator(".reports-panel").screenshot({ path: resolve(out, "perf-reports.jpg"), type: "jpeg", quality: 70 });
}
console.log("reports:", (await page.locator(".report h3").allInnerTexts()).join(", "), "| warnings shown:", await page.locator(".report .notice.warn").count());

// The same bundle at phone width: layout, the final-mesh button, the gallery.
const phone = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
await phone.addInitScript(() => localStorage.setItem("crispmesh.studio.v1", JSON.stringify({ replaySpeed: 0 })));
const small = await phone.newPage();
small.on("pageerror", (error) => failures.push("phone: " + error.message));
await small.goto(`${base}#/replay?bundle=${encodeURIComponent(bundle)}`);
await small.locator("#mesh-heading + .sub", { hasText: "triangles" }).waitFor({ timeout: 180_000 });
await small.waitForTimeout(500);
const sideways = await small.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
console.log(`phone: ${await small.locator("#mesh-heading + .sub").innerText()}; sideways scroll ${sideways}px`);
if (out !== "") {
  await small.screenshot({ path: resolve(out, "perf-phone.jpg"), type: "jpeg", quality: 70 });
  await small.locator(".mesh-panel").scrollIntoViewIfNeeded();
  await small.screenshot({ path: resolve(out, "perf-phone-surface.jpg"), type: "jpeg", quality: 70 });
  await small.locator(".gallery").scrollIntoViewIfNeeded();
  await small.waitForLoadState("networkidle");
  await small.screenshot({ path: resolve(out, "perf-phone-gallery.jpg"), type: "jpeg", quality: 70 });
}

await browser.close();
if (failures.length > 0) {
  console.error("page errors:\n  " + failures.join("\n  "));
  process.exitCode = 1;
}
