// Drives the built app in headless Chromium and saves screenshots.
//
//   npm run build && npm run preview &          # or: engine_server.py --static dist
//   npm run screenshots                          # replay shots from http://127.0.0.1:1431/
//   STUDIO_URL=http://127.0.0.1:8765/ STUDIO_ENGINE_INPUTS=sphere npm run screenshots
//
// With STUDIO_ENGINE_INPUTS set, STUDIO_URL must be an engine serving the app (--static):
// the script then connects through the UI, starts a small run with the settings below and
// photographs it live and when finished.
//
// Environment: STUDIO_URL, STUDIO_OUT (default docs/), STUDIO_ENGINE_INPUTS,
// STUDIO_ENGINE_SETTINGS (JSON, setting name -> text typed into the form), STUDIO_DEVICE,
// STUDIO_ENGINE_TOKEN (the engine was started with --token), STUDIO_ENGINE_EXTRAS=1 (also
// cancel a run and start one on the folder "empty", which must exist and must fail).

import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const here = dirname(fileURLToPath(import.meta.url));
const base = process.env.STUDIO_URL ?? "http://127.0.0.1:1431/";
const out = resolve(process.env.STUDIO_OUT ?? resolve(here, "../docs"));
const engineInputs = process.env.STUDIO_ENGINE_INPUTS ?? "";
const device = process.env.STUDIO_DEVICE ?? "cpu";
const engineSettings = JSON.parse(
  process.env.STUDIO_ENGINE_SETTINGS ??
    JSON.stringify({
      sizes: "64, 128",
      grid: "96",
      planes: "48",
      neighbours: "4",
      best_of: "2",
      vote_neighbours: "4",
      min_votes: "2, 2",
      crop_padding: "6",
      hull_dilate: "1",
      windows: "5, 7",
      aggregates: "1, 1",
    }),
);

const DESKTOP = { width: 1440, height: 900 };
const PHONE = { width: 390, height: 844 };
const NARROW = { width: 360, height: 740 };

mkdirSync(out, { recursive: true });
const engineToken = process.env.STUDIO_ENGINE_TOKEN ?? "";
const extras = process.env.STUDIO_ENGINE_EXTRAS === "1";
const problems = [];
const expected = [];

async function page(browser, viewport, { theme = "light", prefs = {}, mobile = false } = {}) {
  const context = await browser.newContext({
    locale: "en-US",
    viewport,
    deviceScaleFactor: 1,
    colorScheme: theme,
    hasTouch: mobile,
    isMobile: mobile,
  });
  await context.addInitScript((stored) => {
    if (localStorage.getItem("crispmesh.studio.v1") === null) {
      localStorage.setItem("crispmesh.studio.v1", JSON.stringify(stored));
    }
  }, prefs);
  const tab = await context.newPage();
  tab.on("console", (message) => {
    if (message.type() === "error") problems.push(`console: ${message.text()}`);
  });
  tab.on("pageerror", (error) => problems.push(`page error: ${error.message}`));
  tab.on("request", (request) => {
    const url = new URL(request.url());
    if (!["127.0.0.1", "localhost"].includes(url.hostname) && url.protocol !== "data:" && url.protocol !== "blob:") {
      problems.push(`external request: ${request.url()}`);
    }
  });
  return tab;
}

async function shot(tab, name, options = {}) {
  await tab.waitForTimeout(350);
  await tab.screenshot({ path: resolve(out, name), type: "jpeg", quality: 62, ...options });
  console.log("saved", name);
}

/** Waits until the 3D view shows the named surface. */
async function surface(tab, label, timeout = 60_000) {
  await tab.locator("#mesh-heading + .sub", { hasText: label }).waitFor({ timeout });
}

async function overflow(tab, where) {
  const wide = await tab.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  if (wide > 1) problems.push(`${where}: page scrolls sideways by ${wide}px`);
}

async function replay(browser) {
  // Desktop, finished run.
  let tab = await page(browser, DESKTOP, { prefs: { replaySpeed: 0 } });
  await tab.goto(base + "#/replay");
  await surface(tab, "Final surface");
  await tab.locator(".sheet img").last().waitFor();
  await tab.waitForLoadState("networkidle");
  await shot(tab, "replay-desktop.jpg");
  await overflow(tab, "replay desktop");

  // Scrub back to the middle: an earlier surface, a running stage.
  await tab.locator(".replay-scrub input").fill("12");
  await surface(tab, "Silhouette hull after mask repair");
  await shot(tab, "replay-desktop-midway.jpg");

  // Step through the surfaces by keyboard.
  await tab.locator(".replay-scrub input").fill("35");
  await surface(tab, "Final surface");
  await tab.getByRole("button", { name: "Earlier surface" }).click();
  await surface(tab, "Surface after level 1");
  await tab.getByRole("button", { name: /^1\s*Silhouette hull\b/ }).first().click();
  await surface(tab, "Silhouette hull");
  await tab.getByRole("button", { name: "Flat" }).click();
  await shot(tab, "replay-desktop-hull-flat.jpg", { clip: { x: 0, y: 150, width: 960, height: 750 } });

  // Lightbox.
  await tab.getByRole("button", { name: /Open: Depth at level 2/ }).click();
  await tab.locator(".lightbox-stage img:not(.loading)").waitFor();
  await tab.keyboard.press("+");
  await tab.keyboard.press("+");
  await shot(tab, "lightbox-desktop.jpg");
  await tab.keyboard.press("Escape");
  await tab.locator(".lightbox").waitFor({ state: "detached" });
  await tab.context().close();

  // Desktop, dark theme.
  tab = await page(browser, DESKTOP, { theme: "dark", prefs: { replaySpeed: 0 } });
  await tab.goto(base + "#/replay");
  await surface(tab, "Final surface");
  await tab.waitForLoadState("networkidle");
  await shot(tab, "replay-desktop-dark.jpg");
  await tab.context().close();

  // Phone: top of the page and the whole page.
  for (const [viewport, name] of [
    [PHONE, "replay-phone"],
    [NARROW, "replay-phone-360"],
  ]) {
    tab = await page(browser, viewport, { prefs: { replaySpeed: 0 }, mobile: true });
    await tab.goto(base + "#/replay");
    await surface(tab, "Final surface");
    await tab.waitForLoadState("networkidle");
    await shot(tab, `${name}.jpg`);
    await overflow(tab, name);
    if (viewport === PHONE) {
      await tab.locator(".gallery").scrollIntoViewIfNeeded();
      await shot(tab, `${name}-gallery.jpg`);
      await tab.getByRole("button", { name: /Open: Photos with mask outlines/ }).click();
      await tab.locator(".lightbox-stage img:not(.loading)").waitFor();
      await shot(tab, "lightbox-phone.jpg");
      await tab.getByRole("button", { name: "Close" }).click();
    }
    await tab.context().close();
  }

  // Connection screen.
  tab = await page(browser, DESKTOP);
  await tab.goto(base + "#/");
  await tab.getByRole("heading", { name: "Engine" }).waitFor();
  await shot(tab, "connection-desktop.jpg");
  await tab.context().close();
  tab = await page(browser, NARROW, { mobile: true, theme: "dark" });
  await tab.goto(base + "#/");
  await tab.getByRole("heading", { name: "Engine" }).waitFor();
  await shot(tab, "connection-phone-dark.jpg");
  await overflow(tab, "connection phone");
  await tab.context().close();
}

/** Fills the new-run form and starts the run; returns once the run view is open. */
async function startRun(tab, inputs, name) {
  await tab.goto(base + "#/engine/new");
  await tab.getByRole("heading", { name: "New run" }).waitFor();
  await tab.locator("#s-sizes").waitFor();
  await tab.locator("#f-inputs").fill(inputs);
  await tab.locator("details.settings-group").evaluateAll((groups) => groups.forEach((group) => group.setAttribute("open", "")));
  for (const [setting, value] of Object.entries(engineSettings)) await tab.locator(`#s-${setting}`).fill(value);
  await tab.locator("#f-name").fill(name);
  await tab.locator("#f-device").selectOption(device);
  const posted = tab.waitForRequest((request) => request.method() === "POST" && request.url().endsWith("/api/runs"));
  await tab.getByRole("button", { name: "Start run" }).click();
  const body = (await posted).postData();
  await tab.waitForURL(/#\/engine\/run\//, { timeout: 30_000 });
  return body;
}

async function engine(browser) {
  const origin = new URL(base).origin;
  const tab = await page(browser, DESKTOP);
  await tab.goto(base + "#/");
  // Connect through the form, as a person would.
  await tab.getByLabel("Address", { exact: true }).fill(origin);
  if (engineToken !== "") {
    // A wrong token first: the refusal must be said plainly, and nothing remembered.
    expected.push("401");
    await tab.locator("input[type=password]").fill("not-the-token");
    await tab.getByRole("button", { name: "Connect" }).click();
    await tab.locator("#engine-error", { hasText: "refused the access token" }).waitFor();
    await shot(tab, "connection-token-refused.jpg");
    await tab.locator("input[type=password]").fill(engineToken);
  }
  await tab.getByRole("button", { name: "Connect" }).click();
  await tab.getByRole("heading", { name: "Runs", level: 1 }).waitFor();
  await tab.getByRole("link", { name: /New run|Start the first run/ }).first().click();
  await tab.getByRole("heading", { name: "New run" }).waitFor();
  await tab.locator("#s-sizes").waitFor();

  // A mistake first, to see the engine's own validation arrive at the field.
  expected.push("400");
  await tab.locator("#f-inputs").fill(engineInputs);
  await tab.locator("details.settings-group").evaluateAll((groups) => groups.forEach((group) => group.setAttribute("open", "")));
  await tab.locator("#s-sizes").fill("128, 64");
  await tab.getByRole("button", { name: "Start run" }).click();
  await tab.locator("#s-sizes-error", { hasText: "Sizes must increase" }).waitFor();
  await tab.locator("#s-sizes").scrollIntoViewIfNeeded();
  await shot(tab, "new-run-validation.jpg");
  await tab.locator("#s-sizes").fill("64, 128");
  await tab.locator("#f-inputs").fill("no/such/folder");
  await tab.getByRole("button", { name: "Start run" }).click();
  await tab.locator("#f-inputs-error", { hasText: "existing path" }).waitFor();

  // Find the inputs with the folder browser instead of typing them.
  await tab.locator("#f-inputs").fill("");
  await tab.locator("#f-inputs + button", { hasText: "Browse" }).click();
  const parts = engineInputs.split("/");
  for (const part of parts.slice(0, -1)) await tab.locator("#f-inputs-browser .browser-entry", { hasText: part }).click();
  await tab.locator("#f-inputs-browser li.pickable", { hasText: parts.at(-1) }).waitFor();
  await tab.locator("#f-inputs").scrollIntoViewIfNeeded();
  await shot(tab, "new-run-browser.jpg");
  await tab.locator("#f-inputs-browser li.pickable", { hasText: parts.at(-1) }).getByRole("button", { name: /^Use/ }).click();
  const browsed = await tab.locator("#f-inputs").inputValue();
  if (browsed !== engineInputs) problems.push(`folder browser chose "${browsed}", expected "${engineInputs}"`);
  // Type-ahead: the datalist offers what is in the folder being typed.
  await tab.locator("#f-reference").fill(parts.slice(0, -1).join("/") + (parts.length > 1 ? "/" : ""));
  await tab.locator("#f-reference-list option").first().waitFor({ state: "attached" });
  console.log("reference suggestions:", await tab.locator("#f-reference-list option").evaluateAll((all) => all.map((o) => o.value)));
  await tab.locator("#f-reference").fill("");

  console.log("start body:", await startRun(tab, engineInputs, "studio check"));
  await tab.evaluate(() => scrollTo(0, 0));

  // Live: the first surface, then a later one, then the end.
  await tab.getByRole("button", { name: "Cancel run" }).waitFor({ timeout: 30_000 });
  const runUrl = tab.url();
  await surface(tab, "Silhouette hull", 180_000);
  await tab.locator(".sheet img").first().waitFor();
  await shot(tab, "engine-live-desktop.jpg");

  const phone = await page(browser, PHONE, { mobile: true, prefs: { engineUrl: origin, engineToken } });
  await phone.goto(runUrl);
  await surface(phone, "Silhouette hull", 180_000);
  await shot(phone, "engine-live-phone.jpg");
  await overflow(phone, "engine live phone");

  await surface(tab, "Surface after level 1", 300_000);
  await shot(tab, "engine-live-desktop-level1.jpg");

  await tab.locator(".badge.status-complete").waitFor({ timeout: 600_000 });
  await surface(tab, "Final surface");
  await tab.getByRole("heading", { name: "Mesh report" }).waitFor();
  await tab.getByRole("heading", { name: "Photo check" }).waitFor();
  await tab.waitForLoadState("networkidle");
  const loaded = await tab.locator(".sheet img").evaluateAll((images) => images.filter((image) => image.naturalWidth > 0).length);
  const cards = await tab.locator(".sheet").count();
  if (loaded !== cards) problems.push(`engine run: ${cards} sheets but only ${loaded} images loaded`);
  console.log(`sheets shown: ${loaded} of ${cards}`);
  await shot(tab, "engine-done-desktop.jpg");
  await tab.locator(".reports-panel").scrollIntoViewIfNeeded();
  await tab.getByRole("heading", { name: "Photo check" }).scrollIntoViewIfNeeded();
  await shot(tab, "engine-done-desktop-reports.jpg");
  await phone.locator(".badge.status-complete").waitFor({ timeout: 60_000 });
  await surface(phone, "Final surface");
  await shot(phone, "engine-done-phone.jpg");
  await phone.locator(".gallery").scrollIntoViewIfNeeded();
  await shot(phone, "engine-done-phone-gallery.jpg");
  await phone.context().close();

  if (extras) {
    // Cancel a run from the UI.
    await startRun(tab, engineInputs, "to cancel");
    await surface(tab, "Silhouette hull", 180_000);
    await tab.getByRole("button", { name: "Cancel run" }).click();
    await tab.locator(".badge.status-cancelled").waitFor({ timeout: 120_000 });
    await tab.evaluate(() => scrollTo(0, 0));
    await shot(tab, "engine-cancelled-desktop.jpg");

    // A run that fails: inputs that are a folder, but not an inputs folder.
    // The engine starts it, fails before the first stage and says why.
    await startRun(tab, "empty", "to fail");
    await tab.locator(".badge.status-failed").waitFor({ timeout: 45_000 });
    await tab.locator(".notice.bad .error-text").waitFor();
    await shot(tab, "engine-failed-desktop.jpg");
  }

  await tab.goto(base + "#/engine");
  await tab.locator(".run-row").first().waitFor();
  await shot(tab, "runs-desktop.jpg");
  await tab.context().close();
}

const browser = await chromium.launch({ args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"] });
try {
  if (engineInputs === "") await replay(browser);
  else await engine(browser);
} finally {
  await browser.close();
}
// Refusals provoked on purpose show up in the console as failed requests.
const unexpected = [...new Set(problems)].filter((problem) => !expected.some((status) => problem.includes(`status of ${status}`)));
if (unexpected.length > 0) {
  console.error("Problems seen:\n  " + unexpected.join("\n  "));
  process.exitCode = 1;
}
