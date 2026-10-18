// Starts downloading an example object in "This browser", cancels after a few files, checks
// that the partial download is reported and can be deleted, and that it is gone afterwards.
//
//   STUDIO_URL=https://crispstrobe.github.io/CrispMesh/ node scripts/examples-cancel-check.mjs

import { chromium } from "playwright";

const base = process.env.STUDIO_URL ?? "http://127.0.0.1:1431/";
const object = process.env.STUDIO_OBJECT ?? "Lucy";
const browser = await chromium.launch({ channel: "chrome", headless: true, args: ["--enable-unsafe-webgpu", "--enable-features=WebGPU", "--use-angle=metal"] });
const page = await browser.newPage({ locale: "en-US", viewport: { width: 1280, height: 900 } });
const problems = [];
page.on("pageerror", (error) => problems.push(`page error: ${error.message}`));
try {
  await page.goto(base + "#/");
  await page.waitForFunction(() => globalThis.crossOriginIsolated, null, { timeout: 15000 }).catch(() => undefined);
  await page.waitForLoadState("load");
  await page.getByRole("link", { name: "Choose an object" }).click();
  const row = page.locator(".example", { has: page.locator("strong", { hasText: new RegExp(`^${object}$`) }) });
  await row.waitFor({ timeout: 60000 });
  await page.waitForTimeout(1500);
  console.log("before:", (await row.locator(".example-text").innerText()).replace(/\s+/g, " "));
  await row.getByRole("button", { name: /reconstruct/i }).first().click();
  await row.locator(".example-confirm").getByRole("button", { name: "Download and reconstruct" }).click();
  await page.waitForFunction((name) => {
    const target = [...document.querySelectorAll(".example")].find((r) => r.querySelector("strong")?.textContent === name);
    const match = /Downloading (\d+) of/.exec(target?.querySelector("[role=status]")?.textContent ?? "");
    return match !== null && Number(match[1]) >= 5;
  }, object, { timeout: 300000 });
  console.log("cancelling at:", (await row.locator("[role=status]").innerText()).replace(/\s+/g, " "));
  await row.getByRole("button", { name: "Cancel" }).click();
  await row.locator("progress").waitFor({ state: "detached", timeout: 60000 });
  await page.waitForTimeout(1500);
  console.log("after cancel:", (await row.locator(".example-text").innerText()).replace(/\s+/g, " "), "| buttons:", (await row.locator("button").allInnerTexts()).join(", "));
  console.log("cached files:", await page.evaluate(async () => (await (await caches.open("crispmesh-example-objects")).keys()).length));
  await row.getByRole("button", { name: "Delete download" }).click();
  await page.waitForTimeout(2500);
  console.log("after delete:", (await row.locator(".example-text").innerText()).replace(/\s+/g, " "), "| buttons:", (await row.locator("button").allInnerTexts()).join(", "));
  console.log("cached files:", await page.evaluate(async () => (await (await caches.open("crispmesh-example-objects")).keys()).length));
} catch (problem) {
  problems.push(String(problem?.message ?? problem).split("\n")[0]);
} finally {
  await browser.close();
}
if (problems.length > 0) {
  console.error("Problems:\n  " + [...new Set(problems)].join("\n  "));
  process.exitCode = 1;
}
