// Runs the test page in headless Chromium with WebGPU and writes what came out.
//
//   node test/run.mjs --scene INPUTS_DIR --output DIR [--options JSON] [--keep a,b] [--timeout SECONDS]
//                     [--software] [--headed] [--expect-triangles]
//   node test/run.mjs --photos PHOTOS_DIR --calibration LENS_JSON --output DIR [...]
//   [--channel chrome]   an installed Google Chrome instead of Playwright's Chromium
//   [--isolated]         cross-origin isolation headers: the threaded package is used if built
//   [--threads N]        workers of the threaded package (default: cores, at most 4; 1 forces single-threaded)
//   [--trace FILE]       a Chrome performance trace of the run
//
// INPUTS_DIR is an inputs directory (cameras.json, masks, sparse_points.npy; photos may
// lie elsewhere, as absolute paths in cameras.json). The server hands the page a copy
// with relative paths. Files named by --keep are uploaded by the page into DIR, next to
// events.jsonl, pipeline.json and result.json (adapter, limits, memory, timings).
// --software asks Chromium for its software WebGPU adapter (SwiftShader), for machines
// without a GPU. With --photos the page starts from the photos and the lens calibration
// (masks and cameras recovered in the browser). Exit code 0 when the run completed.

import { execFileSync } from "node:child_process";
import { createReadStream, createWriteStream, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const web = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const value = (name, fallback) => (args.includes(name) ? args[args.indexOf(name) + 1] : fallback);
const scene = resolve(value("--scene", "scene"));
const photosDir = args.includes("--photos") ? resolve(value("--photos")) : null;
const calibration = args.includes("--calibration") ? resolve(value("--calibration")) : null;
if (photosDir && !calibration) throw new Error("--photos needs --calibration");
const output = resolve(value("--output", "browser-run"));
const options = value("--options", "{}");
const keep = value("--keep", "");
const timeout = Number(value("--timeout", "600")) * 1000;
mkdirSync(output, { recursive: true });

// The inputs directory as the page sees it: relative paths only.
const mapped = new Map();
let camerasText = null;
if (photosDir) {
  mapped.set("calibration.json", calibration);
  for (const name of readdirSync(photosDir).sort()) {
    if ([".png", ".jpg", ".jpeg"].includes(extname(name).toLowerCase())) mapped.set(`photos/${name}`, join(photosDir, name));
  }
} else {
  const cameras = JSON.parse(readFileSync(join(scene, "cameras.json"), "utf8"));
  mapped.set("sparse_points.npy", join(scene, "sparse_points.npy"));
  for (const view of cameras.views) {
    for (const [key, folder] of [["image", "images"], ["mask", "masks"]]) {
      const source = resolve(scene, view[key]);
      const name = `${folder}/${view.name}${extname(source)}`;
      mapped.set(name, source);
      view[key] = name;
    }
  }
  camerasText = JSON.stringify(cameras);
}
const types = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".wasm": "application/wasm", ".json": "application/json" };

// --isolated serves every response with the headers of cross-origin isolation, so that the page
// may use shared memory and loads the threaded package (pkg-threads) when it was built.
const isolation = flag("--isolated") ? { "cross-origin-opener-policy": "same-origin", "cross-origin-embedder-policy": "require-corp" } : {};

const server = createServer((request, response) => {
  for (const [name, value] of Object.entries(isolation)) response.setHeader(name, value);
  const path = decodeURIComponent(new URL(request.url, "http://localhost").pathname);
  if (request.method === "POST" && path.startsWith("/upload/")) {
    const target = join(output, path.slice("/upload/".length));
    if (!target.startsWith(output)) return response.writeHead(400).end();
    mkdirSync(dirname(target), { recursive: true });
    request.pipe(createWriteStream(target)).on("finish", () => response.writeHead(204).end());
    return;
  }
  let file;
  if (path === "/scene/files.json") {
    return response.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify([...(camerasText ? ["cameras.json"] : []), ...mapped.keys()]));
  } else if (path === "/scene/cameras.json" && camerasText) {
    return response.writeHead(200, { "content-type": "application/json" }).end(camerasText);
  } else if (path.startsWith("/scene/")) {
    file = mapped.get(path.slice("/scene/".length));
  } else {
    file = join(web, path === "/" ? "index.html" : path);
    if (!file.startsWith(web)) file = undefined;
  }
  if (!file || !existsSync(file) || !statSync(file).isFile()) return response.writeHead(404).end();
  response.writeHead(200, { "content-type": types[extname(file)] ?? "application/octet-stream" });
  createReadStream(file).pipe(response);
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const origin = `http://127.0.0.1:${server.address().port}`;

// Chromium's new headless mode keeps the GPU process; WebGPU needs to be switched on in it.
const launch = ["--enable-unsafe-webgpu", "--enable-features=WebGPU"];
if (flag("--software")) launch.push("--use-webgpu-adapter=swiftshader", "--use-angle=swiftshader", "--enable-unsafe-swiftshader");
else if (process.platform === "darwin") launch.push("--use-angle=metal");
else launch.push("--enable-features=Vulkan", "--use-angle=vulkan");
// --channel chrome uses an installed Google Chrome instead of Playwright's Chromium download.
const channel = value("--channel", "chromium");
const browser = await chromium.launch({ channel, headless: !flag("--headed"), args: launch });
const page = await browser.newPage();
const consoleLines = [];
page.on("console", (message) => consoleLines.push(`${message.type()}: ${message.text()}`));
page.on("pageerror", (error) => consoleLines.push(`pageerror: ${error.message}`));
page.on("worker", (worker) => worker.on?.("console", (message) => consoleLines.push(`worker ${message.type()}: ${message.text()}`)));

// Peak resident memory of Chromium's processes by kind, sampled once a second.
const executable = dirname(chromium.executablePath());
const peaks = {};
function sampleMemory() {
  // An installed Chrome shares its executable with the user's own windows: no per-process figures then.
  if (process.platform === "win32" || channel !== "chromium") return;
  try {
    const rows = execFileSync("ps", ["-axo", "rss=,command="], { encoding: "utf8", maxBuffer: 1 << 24 }).split("\n");
    const now = {};
    for (const row of rows) {
      const match = row.trim().match(/^(\d+)\s+(.*)$/);
      // Chromium's processes run from the directory of its executable (helpers lie below it).
      if (!match || !match[2].includes(dirname(executable))) continue;
      const kind = match[2].match(/--type=([\w-]+)/)?.[1] ?? "browser";
      now[kind] = Math.max(now[kind] ?? 0, Number(match[1]) * 1024);
    }
    for (const [kind, bytes] of Object.entries(now)) peaks[kind] = Math.max(peaks[kind] ?? 0, bytes);
  } catch {
    // ps is a convenience; the run does not depend on it
  }
}
const sampler = setInterval(sampleMemory, 1000);

const query = new URLSearchParams({ base: "scene", options, keep, upload: "upload", mode: photosDir ? "photos" : "inputs" });
if (args.includes("--threads")) query.set("threads", value("--threads"));
const started = Date.now();
// --trace FILE records a Chrome performance trace of the whole run (threads, tasks, GPU), for
// chrome://tracing or the Performance panel. Large: about 1 MB per second of run.
const tracePath = value("--trace", null);
if (tracePath) {
  await browser.startTracing(page, {
    path: resolve(tracePath),
    categories: ["devtools.timeline", "disabled-by-default-devtools.timeline", "v8", "v8.execute", "blink.user_timing", "gpu", "toplevel", "disabled-by-default-webgpu"],
  });
}
await page.goto(`${origin}/index.html?${query}`);
let state;
try {
  await page.waitForFunction(() => ["done", "error"].includes(window.crispmesh?.status) && window.crispmesh.uploading === 0, null, { timeout, polling: 500 });
  state = await page.evaluate(() => {
    const { kept, ...rest } = window.crispmesh;
    return { ...rest, jsHeapBytes: performance.memory?.usedJSHeapSize ?? null };
  });
} catch (error) {
  state = await page.evaluate(() => ({ status: window.crispmesh?.status ?? "no page state", events: window.crispmesh?.events ?? [], wasmBytes: window.crispmesh?.wasmBytes, error: "timeout" })).catch(() => ({ status: "crashed", events: [], error: String(error) }));
}
clearInterval(sampler);
sampleMemory();
if (tracePath) await browser.stopTracing();
await browser.close();
server.close();

const events = state.events ?? [];
writeFileSync(join(output, "events.jsonl"), events.map((event) => JSON.stringify(event) + "\n").join(""));
if (state.report) writeFileSync(join(output, "pipeline.json"), JSON.stringify(state.report, null, 2) + "\n");
const result = {
  status: state.status,
  error: state.error ?? null,
  wall_seconds: (Date.now() - started) / 1000,
  run_seconds: state.seconds ?? null,
  threads: state.threads ?? null,
  cross_origin_isolated: Boolean(flag("--isolated")),
  adapter: state.adapter ?? null,
  limits: state.limits ?? null,
  peak_wasm_bytes: state.peakWasmBytes ?? state.wasmBytes ?? null,
  page_js_heap_bytes: state.jsHeapBytes ?? null,
  peak_process_resident_bytes: peaks,
  wasm_bytes_at_events: state.memoryAtEvents ?? null,
  events: events.length,
  triangles: state.report?.triangles ?? null,
  closed: state.report?.closed ?? null,
  genus: state.report?.genus ?? null,
  files: state.files ?? null,
  console: consoleLines.slice(-40),
};
writeFileSync(join(output, "result.json"), JSON.stringify(result, null, 2) + "\n");
const { files, ...summary } = result;
console.log(JSON.stringify(summary, null, 2));
process.exit(state.status === "done" ? 0 : 1);
