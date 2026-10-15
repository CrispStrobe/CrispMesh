// Runs the engine off the page's thread. Messages in:
//   { type: "run", base, options, keep, mode }   fetch `${base}/files.json` (a list of paths) and those files, then run;
//                                     mode "photos": the files are calibration.json and photos/<name>
//   { type: "cancel" }
// Messages out: { type: "status", text }, { type: "event", event }, { type: "memory", ... },
//   { type: "done", report, files, seconds, adapter, limits }, { type: "error", message },
//   { type: "file", path, bytes } for every path in `keep` (transferred).

import { createRun, load, memory, threadCount } from "./crispmesh-dense.js";

let run;

async function describeGpu() {
  if (!navigator.gpu) return { adapter: null, limits: null };
  const adapter = await navigator.gpu.requestAdapter({ powerPreference: "high-performance" });
  if (!adapter) return { adapter: null, limits: null };
  const info = adapter.info ?? {};
  const names = ["maxStorageBufferBindingSize", "maxBufferSize", "maxStorageBuffersPerShaderStage", "maxComputeWorkgroupsPerDimension",
    "maxComputeInvocationsPerWorkgroup", "maxComputeWorkgroupStorageSize", "maxComputeWorkgroupSizeX", "maxUniformBufferBindingSize", "maxBindGroups"];
  return {
    adapter: { vendor: info.vendor, architecture: info.architecture, device: info.device, description: info.description, fallback: info.isFallbackAdapter ?? adapter.isFallbackAdapter ?? null },
    limits: Object.fromEntries(names.map((name) => [name, adapter.limits[name]])),
  };
}

self.onmessage = async ({ data }) => {
  if (data.type === "cancel") {
    run?.cancel();
    return;
  }
  if (data.type !== "run") return;
  try {
    const gpu = await describeGpu();
    self.postMessage({ type: "status", text: "Fetching input files" });
    const list = await (await fetch(`${data.base}/files.json`)).json();
    const files = new Map();
    for (const path of list) {
      const response = await fetch(`${data.base}/${path}`);
      if (!response.ok) throw new Error(`cannot fetch ${path}: ${response.status}`);
      files.set(path, new Uint8Array(await response.arrayBuffer()));
    }
    await load(data.threads ? { threads: data.threads } : {});
    self.postMessage({ type: "status", text: `Running on ${files.size} files with ${threadCount()} thread(s)` });
    const started = performance.now();
    let peak = 0;
    // From photos: files.json lists calibration.json and photos/<name>.
    let source = { files };
    if (data.mode === "photos") {
      const photos = new Map();
      for (const [path, bytes] of files) if (path.startsWith("photos/")) photos.set(path.slice("photos/".length), bytes);
      source = { photos, calibration: files.get("calibration.json") };
    }
    run = await createRun({
      ...source,
      options: data.options ?? {},
      onEvent: (event) => {
        peak = Math.max(peak, memory().wasm);
        const now = memory();
        self.postMessage({ type: "event", event, wasmBytes: now.wasm, fileBytes: now.files });
      },
    });
    // The run keeps the photos it was given; the fetched map is not needed twice.
    if (data.mode === "photos") files.clear();
    const report = await run.finished;
    peak = Math.max(peak, memory().wasm);
    for (const path of data.keep ?? []) {
      const bytes = run.file(path);
      if (bytes) self.postMessage({ type: "file", path, bytes }, [bytes.buffer]);
    }
    self.postMessage({ type: "done", report, files: run.files(), seconds: (performance.now() - started) / 1000, peakWasmBytes: peak, threads: threadCount(), ...gpu });
    run.dispose();
  } catch (error) {
    self.postMessage({ type: "error", message: String(error?.message ?? error), wasmBytes: memory().wasm });
  }
};
