// Downloads an example object through the app's own Example objects screen in headless Chrome
// (WebGPU) and reconstructs it in the browser: what is shown before the download, the time the
// download took, the run, and the attribution kept with it.
//
//   STUDIO_URL=https://crispstrobe.github.io/CrispMesh/ STUDIO_OBJECT=Dragon node scripts/examples-check.mjs

import { chromium } from "playwright";
import { statSync } from "node:fs";

const base = process.env.STUDIO_URL ?? "http://127.0.0.1:1431/";
const object = process.env.STUDIO_OBJECT ?? "Dragon";
const launch = ["--enable-unsafe-webgpu", "--enable-features=WebGPU"];
if (process.platform === "darwin") launch.push("--use-angle=metal");
const browser = await chromium.launch({ channel: "chrome", headless: true, args: launch });
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
const problems = [];
page.on("pageerror", (error) => problems.push(`page error: ${error.message}`));
page.on("console", (message) => message.type() === "error" && problems.push(`console: ${message.text()}`));
try {
  await page.goto(base + "#/");
  await page.waitForFunction(() => globalThis.crossOriginIsolated, null, { timeout: 15000 }).catch(() => undefined);
  await page.waitForLoadState("load");
  await page.getByRole("link", { name: "Choose an object" }).click();
  await page.getByRole("heading", { name: "Example objects", level: 1 }).waitFor();
  await page.locator(".example").first().waitFor({ timeout: 60000 });
  console.log("source shown:", (await page.locator(".attribution-card").first().innerText()).replace(/\s+/g, " ").slice(0, 500));
  console.log("sections:", (await page.locator(".example-source h2").allInnerTexts()).join(" | "));
  console.log("objects:", (await page.locator(".example-text strong").allInnerTexts()).join(", "));
  // The object's name, or the start of it ("Rhino" for "Rhino figurine (...)").
  const row = page.locator(".example", { has: page.locator("strong", { hasText: new RegExp(`^${object}`) }) }).first();
  console.log("row:", (await row.locator(".example-text").innerText()).replace(/\s+/g, " "));
  await row.getByRole("button", { name: /Download and reconstruct|Reconstruct/ }).first().click();
  const confirm = row.locator(".example-confirm");
  if ((await confirm.count()) > 0) {
    console.log("asked before download:", (await confirm.locator("p").innerText()).replace(/\s+/g, " "));
    await confirm.getByRole("button", { name: "Download and reconstruct" }).click();
  }
  const started = Date.now();
  await page.waitForURL(/#\/engine\/run\//, { timeout: 900000 });
  console.log(`downloaded and started after ${((Date.now() - started) / 1000).toFixed(1)} s: ${decodeURIComponent(page.url().split("/").pop())}`);
  const runStart = Date.now();
  await page.locator(".run-meta .badge.status-complete, .run-meta .badge.status-failed").waitFor({ timeout: 1800000 });
  const status = await page.locator(".run-meta .badge").first().innerText();
  console.log(`run ${status} after ${((Date.now() - runStart) / 1000).toFixed(1)} s | ${(await page.locator(".run-meta").innerText()).replace(/\s+/g, " ")}`);
  if (status !== "Complete") throw new Error("the run did not complete: " + (await page.locator(".notice").first().innerText().catch(() => "")));
  console.log("notice:", (await page.locator(".notice").first().innerText().catch(() => "")).replace(/\s+/g, " ").slice(0, 400));
  console.log("stages:", (await page.locator(".stage").allInnerTexts()).map((text) => text.replace(/\s+/g, " ")).join(" | "));
  const load = page.getByRole("button", { name: /^Load final/ });
  if ((await load.count()) > 0) await load.click();
  await page.locator("#mesh-heading + .sub", { hasText: "Final surface" }).waitFor({ timeout: 300000 });
  console.log("surface:", await page.locator("#mesh-heading + .sub").innerText());
  console.log("numbers:", (await page.locator(".metrics > div").allInnerTexts()).map((row) => row.replace(/\s+/g, " ")).filter((row) => /Registered|Input photos|Reprojection median|Silhouette iou/i.test(row)).join(" | "));
  await page.locator("details.attribution summary").click();
  console.log("kept with the run:", (await page.locator("details.attribution pre").innerText()).replace(/\s+/g, " ").slice(0, 300));
  const waiting = page.waitForEvent("download");
  await page.getByRole("button", { name: /^Download STL/ }).click();
  const download = await waiting;
  const path = await download.path();
  if (path === null) throw new Error("STL download failed: " + await download.failure());
  const bytes = statSync(path).size;
  if (bytes <= 84 || (bytes - 84) % 50 !== 0) throw new Error(`invalid binary STL size: ${bytes}`);
  console.log(`downloaded ${download.suggestedFilename()}: ${bytes} bytes = ${(bytes - 84) / 50} triangles`);
} catch (problem) {
  problems.push(String(problem?.message ?? problem).split("\n")[0]);
} finally {
  await browser.close();
}
if (problems.length > 0) {
  console.error("Problems:\n  " + [...new Set(problems)].join("\n  "));
  process.exitCode = 1;
}
