// Live desktop smoke test. Run after `ctest --test-dir build-opencv -R sparse_synthetic_image`.
// Tool bootstrap, if absent: npm install --prefix .local-tools/browser --no-save playwright@1.63.0
// Then: PLAYWRIGHT_BROWSERS_PATH=.local-tools/browsers .local-tools/browser/node_modules/.bin/playwright install chromium
// This local browser tool adds no desktop package dependency.
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fixture = path.join(root, "build-opencv/synthetic-sparse-fixture");
const projectPath = path.join(fixture, "project.json");
const reportPath = path.join(fixture, "sparse-cli-report.json");
const cli = path.join(root, "build-opencv/bin/crispmesh");
const vite = path.join(root, "apps/desktop/node_modules/vite/bin/vite.js");
const browserPath = path.join(root, ".local-tools/browsers");
process.env.PLAYWRIGHT_BROWSERS_PATH = browserPath;
const playwrightPackage = JSON.parse(await readFile(path.join(root, ".local-tools/browser/node_modules/playwright/package.json"), "utf8"));
const { chromium } = await import(path.join(root, ".local-tools/browser/node_modules/playwright/index.mjs"));

const projectSource = await readFile(projectPath);
const project = JSON.parse(projectSource.toString());
const cliOutput = execFileSync(cli, ["reconstruct-sparse", projectPath], { encoding: "utf8", maxBuffer: 20 * 1024 * 1024 });
const report = JSON.parse(cliOutput);
assert.equal(report.ok, true);
assert.ok(report.points.length >= 25);
await writeFile(reportPath, cliOutput);

const server = spawn(process.execPath, [vite, "--host", "127.0.0.1", "--port", "1430", "--strictPort"], {
  cwd: path.join(root, "apps/desktop"), stdio: ["ignore", "pipe", "pipe"],
});
let serverLog = "";
server.stdout.on("data", (chunk) => { serverLog += chunk; });
server.stderr.on("data", (chunk) => { serverLog += chunk; });
let browser;
try {
  const url = "http://127.0.0.1:1430/";
  let ready = false;
  for (let attempt = 0; attempt < 100; attempt++) {
    if (server.exitCode !== null) throw new Error(`Vite exited early: ${serverLog}`);
    try { const response = await fetch(url); if (serverLog.includes(url) && response.ok) { ready = true; break; } }
    catch { /* Server is starting. */ }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.ok(ready, `Vite did not start on port 1430: ${serverLog}`);
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 1 });
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(url);
  await page.locator("#file-input").setInputFiles({ name: "project.json", mimeType: "application/json", buffer: projectSource });
  await page.getByRole("heading", { name: project.name }).waitFor();
  const stageBefore = await page.locator(".stage-state").allTextContents();
  assert.equal(stageBefore.length, 6);
  await page.locator("#sparse-report-file").setInputFiles({ name: "sparse-cli-report.json", mimeType: "application/json", buffer: Buffer.from(cliOutput) });
  await page.waitForFunction(() => Boolean(document.querySelector(".sparse-report-card .pose-report-summary, .sparse-report-card .issues")));
  assert.equal(await page.locator(".sparse-report-card .issues").count(), 0,
    `CLI report rejected: ${await page.locator(".sparse-report-card .issues").allTextContents()}`);
  assert.match(await page.locator(".sparse-report-card .pose-report-summary").innerText(), new RegExp(`${report.points.length.toLocaleString("en-US")} points`));
  assert.equal(await page.locator(".sparse-view-row").count(), project.images.length);
  const canvas = page.locator("#sparse-canvas");
  const canvasMetrics = () => page.locator("#sparse-canvas").evaluate((element) => {
    const canvas = /** @type {HTMLCanvasElement} */ (element);
    const { width, height } = canvas;
    const bytes = canvas.getContext("2d").getImageData(0, 0, width, height).data;
    let nonbackground = 0, hash = 2166136261;
    for (let i = 0; i < bytes.length; i += 4) {
      if (bytes[i] !== 16 || bytes[i + 1] !== 32 || bytes[i + 2] !== 42) nonbackground++;
      hash = Math.imul(hash ^ bytes[i], 16777619);
      hash = Math.imul(hash ^ bytes[i + 1], 16777619);
      hash = Math.imul(hash ^ bytes[i + 2], 16777619);
    }
    return { width, height, nonbackground, hash: hash >>> 0 };
  });
  await page.waitForFunction(() => document.querySelector("#sparse-canvas")?.getAttribute("width") !== "0");
  const initial = await canvasMetrics();
  assert.ok(initial.width > 200 && initial.height > 200 && initial.nonbackground > 500, `Canvas did not render points: ${JSON.stringify(initial)}`);
  const changedAfter = async (before) => {
    for (let attempt = 0; attempt < 30; attempt++) {
      const current = await canvasMetrics();
      if (current.hash !== before.hash) return current;
      await page.waitForTimeout(50);
    }
    throw new Error(`Canvas did not redraw after interaction; hash ${before.hash}`);
  };
  await canvas.scrollIntoViewIfNeeded();
  const bounds = await canvas.boundingBox();
  assert.ok(bounds);
  await page.mouse.move(bounds.x + bounds.width * 0.45, bounds.y + bounds.height * 0.45);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width * 0.7, bounds.y + bounds.height * 0.55, { steps: 5 });
  await page.mouse.up();
  const orbited = await changedAfter(initial);
  await page.mouse.wheel(0, -280);
  const zoomed = await changedAfter(orbited);
  assert.deepEqual(await page.locator(".stage-state").allTextContents(), stageBefore);

  const foreign = { ...report, projectId: "another-project" };
  await page.locator("#sparse-report-file").setInputFiles({ name: "foreign-report.json", mimeType: "application/json", buffer: Buffer.from(JSON.stringify(foreign)) });
  await page.locator(".sparse-report-card .issues").waitFor();
  assert.match(await page.locator(".sparse-report-card .issues").innerText(), /projectId/);
  assert.ok(await canvas.isVisible(), "Rejected report displaced the valid imported report");
  assert.match(await page.locator(".sparse-report-card .pose-report-summary").innerText(), /sparse-cli-report\.json/);

  await page.setViewportSize({ width: 390, height: 844 });
  const narrow = await canvas.boundingBox();
  assert.ok(narrow && narrow.width > 200 && narrow.x >= 0 && narrow.x + narrow.width <= 391, `Canvas overflows narrow viewport: ${JSON.stringify(narrow)}`);
  const pageWidth = await page.evaluate(() => document.documentElement.scrollWidth);
  assert.ok(pageWidth <= 391, `Page overflows narrow viewport: ${pageWidth}px`);
  await page.screenshot({ path: path.join(fixture, "browser-narrow.png"), fullPage: true });

  await page.locator('#calibration-form input[name="fx"]').fill(String(project.calibration.fx + 1));
  await page.locator('#calibration-form button[type="submit"]').click();
  await page.locator("#sparse-canvas").waitFor({ state: "detached" });
  assert.equal(await page.locator(".sparse-report-card .pose-report-summary").count(), 0);
  assert.deepEqual(await page.locator(".stage-state").allTextContents(), stageBefore);
  assert.deepEqual(pageErrors, [], `Browser errors: ${pageErrors.join("; ")}`);
  console.log(`Live browser test passed: Chromium ${browser.version()}, Playwright ${playwrightPackage.version}, ${report.points.length} CLI points, canvas orbit/zoom, report identity, stage immutability, calibration invalidation, 390px layout`);
} finally {
  await browser?.close();
  server.kill("SIGTERM");
  await new Promise((resolve) => { if (server.exitCode !== null) resolve(); else server.once("exit", resolve); });
}
