// Minimal test page: runs the engine in a worker on files it fetches and shows
// status text and the final triangle count. `window.crispmesh` holds the state
// for automated tests: { status, events, report, files, kept, error, ... }.

const query = new URLSearchParams(location.search);
const base = query.get("base") ?? "scene";
const options = JSON.parse(query.get("options") ?? "{}");
const keep = (query.get("keep") ?? "").split(",").filter(Boolean);
const upload = query.get("upload");
const element = (id) => document.getElementById(id);
element("base").textContent = base;

const state = { status: "starting", events: [], memoryAtEvents: [], kept: {}, uploading: 0, report: null, error: null };
window.crispmesh = state;

const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
element("cancel").onclick = () => worker.postMessage({ type: "cancel" });
worker.onerror = (event) => fail(event.message ?? "worker failed to load");
worker.onmessage = ({ data }) => {
  if (data.type === "status") {
    element("status").textContent = data.text;
  } else if (data.type === "event") {
    state.events.push(data.event);
    state.wasmBytes = Math.max(state.wasmBytes ?? 0, data.wasmBytes ?? 0);
    // [stage, kind or progress message, WebAssembly bytes, bytes of files in the tree]
    if (data.event.type !== "metric") {
      const label = data.event.type === "progress" ? `progress ${data.event.message}` : data.event.kind ?? data.event.type;
      const last = state.memoryAtEvents.at(-1);
      // Progress repeats: keep a row only when the label or the memory changed.
      if (!last || last[1] !== label || last[2] !== data.wasmBytes || last[3] !== data.fileBytes) {
        state.memoryAtEvents.push([data.event.stage, label, data.wasmBytes ?? 0, data.fileBytes ?? 0]);
      }
    }
    const event = data.event;
    if (event.type === "progress") {
      element("status").textContent = `${event.stage}: ${event.message} (${Math.round(100 * event.fraction)} %)`;
    } else if (event.type !== "metric") {
      element("log").textContent += `${event.type} ${event.stage ?? ""} ${event.kind ?? event.status ?? event.message ?? ""}\n`;
    }
  } else if (data.type === "file") {
    if (upload) {
      // Hand the file to the test server instead of keeping it in the page.
      state.uploading += 1;
      fetch(`${upload}/${data.path}`, { method: "POST", body: data.bytes })
        .catch((error) => fail(`upload of ${data.path}: ${error}`))
        .finally(() => (state.uploading -= 1));
    } else {
      state.kept[data.path] = data.bytes;
    }
  } else if (data.type === "done") {
    Object.assign(state, data, { status: "done" });
    element("status").textContent = `Finished in ${data.seconds.toFixed(1)} s`;
    element("result").textContent =
      `${data.report.triangles.toLocaleString("en")} triangles, ${data.report.closed ? "closed" : "open"}, genus ${data.report.genus}; ` +
      `peak WebAssembly memory ${(data.peakWasmBytes / 2 ** 20).toFixed(0)} MiB; GPU: ${data.adapter?.description || data.adapter?.device || data.adapter?.vendor || "unknown"}`;
  } else if (data.type === "error") {
    fail(data.message, data);
  }
};
const start = () =>
  worker.postMessage({ type: "run", base: new URL(base, location.href).href.replace(/\/$/, ""), options, keep, mode: query.get("mode") ?? "inputs", threads: query.get("threads") ? Number(query.get("threads")) : undefined });
// coi-worker.js may be about to reload the page under its service worker (cross-origin isolation).
if (window.crispmeshReloading) window.addEventListener("crispmesh-coi-settled", start, { once: true });
else start();

function fail(message, data = {}) {
  Object.assign(state, data, { status: "error", error: message });
  element("status").textContent = `Failed: ${message}`;
}
