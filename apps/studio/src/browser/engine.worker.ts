/**
 * Runs the dense pipeline in the browser, off the page's thread: the WebAssembly package
 * of crates/dense/web, loaded at run time from next to the app (it is not part of the
 * bundle, so the app builds without it).
 *
 * Input files stay on disk as File objects. The engine asks for one at a time and gets
 * its bytes read synchronously then, so the photos are never all in memory at once.
 */

import type { FromWorker, ToWorker } from "./protocol";

interface EngineRun {
  finished: Promise<unknown>;
  cancel(): void;
  file(path: string): Uint8Array | undefined;
  dispose(): void;
}

interface EnginePackage {
  createRun(arguments_: {
    files?: Map<string, Uint8Array>;
    photos?: Map<string, Uint8Array>;
    calibration?: string;
    options: Record<string, unknown>;
    onEvent(event: unknown): void;
  }): Promise<EngineRun>;
  settingsSchema(): Promise<unknown>;
  describe?(): Promise<unknown>;
  memory(): { wasm: number; files: number };
  threadCount?(): number;
}

// Typed by hand: the WebWorker lib collides with the DOM lib in one program.
const scope = self as unknown as {
  onmessage: ((message: MessageEvent<ToWorker>) => void) | null;
  postMessage(message: FromWorker, transfer?: Transferable[]): void;
  navigator: { gpu?: { requestAdapter(options?: unknown): Promise<{ info?: { vendor?: string; architecture?: string; description?: string } } | null> } };
};
declare const FileReaderSync: { new (): { readAsArrayBuffer(file: Blob): ArrayBuffer } };

/** A Map of path to bytes whose values are read from disk when asked for. */
class FilesOnDisk extends Map<string, Uint8Array> {
  private readonly disk = new Map<string, File>();
  private readonly reader = new FileReaderSync();

  constructor(files: [string, File][]) {
    super();
    for (const [path, file] of files) this.disk.set(path, file);
  }

  override get(path: string): Uint8Array | undefined {
    const file = this.disk.get(path);
    return file === undefined ? undefined : new Uint8Array(this.reader.readAsArrayBuffer(file));
  }

  override has(path: string): boolean {
    return this.disk.has(path);
  }

  override get size(): number {
    return this.disk.size;
  }
}

let engine: EnginePackage | null = null;
const runs = new Map<string, EngineRun>();

async function load(module: string): Promise<void> {
  try {
    const gpu = scope.navigator.gpu;
    if (gpu === undefined) {
      scope.postMessage({ type: "loaded", ok: false, reason: "no-webgpu", message: "This browser has no WebGPU, which the engine computes with." });
      return;
    }
    const adapter = await gpu.requestAdapter({ powerPreference: "high-performance" });
    if (adapter === null) {
      scope.postMessage({ type: "loaded", ok: false, reason: "no-adapter", message: "WebGPU is there, but the browser found no graphics adapter it can use." });
      return;
    }
    engine = (await import(/* @vite-ignore */ module)) as EnginePackage;
    const settings = await engine.settingsSchema();
    const info = adapter.info ?? {};
    // Some browsers say the same word for all three ("apple apple apple").
    const parts = [info.vendor, info.architecture, info.description].filter((part): part is string => typeof part === "string" && part !== "");
    const name = [...new Set(parts)].join(" ");
    const described = engine.describe === undefined ? null : await engine.describe().catch(() => null);
    scope.postMessage({ type: "loaded", ok: true, settings: { settings }, adapter: name === "" ? null : name, described, threads: engine.threadCount?.() ?? 1 });
  } catch (problem) {
    scope.postMessage({ type: "loaded", ok: false, message: problem instanceof Error ? problem.message : String(problem) });
  }
}

async function run(id: string, from: { files: [string, File][] } | { photos: [string, File][]; calibration: string }, options: Record<string, unknown>): Promise<void> {
  const started = performance.now();
  let peak = 0;
  const note = () => {
    peak = Math.max(peak, engine?.memory().wasm ?? 0);
    return peak;
  };
  try {
    if (engine === null) throw new Error("The engine is not loaded.");
    // An inputs folder is read one file at a time; photos are copied into the engine when the run is made.
    const source =
      "files" in from
        ? { files: new FilesOnDisk(from.files) }
        : { photos: new Map(from.photos.map(([name, file]) => [name, new Uint8Array(new FileReaderSync().readAsArrayBuffer(file))] as [string, Uint8Array])), calibration: from.calibration };
    const handle = await engine.createRun({
      ...source,
      options,
      onEvent: (event) => {
        note();
        scope.postMessage({ type: "event", id, event, wasmBytes: engine?.memory().wasm ?? 0 });
      },
    });
    runs.set(id, handle);
    await handle.finished;
    scope.postMessage({ type: "finished", id, ok: true, seconds: (performance.now() - started) / 1000, peakWasmBytes: note() });
  } catch (problem) {
    // A cancelled or failed run has said so in its events; this only ends the exchange.
    const message = problem instanceof Error ? problem.message : String(problem);
    scope.postMessage({ type: "finished", id, ok: false, message, seconds: (performance.now() - started) / 1000, peakWasmBytes: note() });
  }
}

scope.onmessage = ({ data }) => {
  if (data.type === "load") void load(data.module);
  else if (data.type === "run") void run(data.id, { files: data.files }, data.options);
  else if (data.type === "run-photos") void run(data.id, { photos: data.photos, calibration: data.calibration }, data.options);
  else if (data.type === "cancel") runs.get(data.id)?.cancel();
  else if (data.type === "file") {
    const bytes = runs.get(data.id)?.file(data.path);
    if (bytes === undefined) scope.postMessage({ type: "file", request: data.request, bytes: null });
    else {
      // A copy leaves the engine's memory; the page owns it from here.
      const copy = bytes.slice().buffer;
      scope.postMessage({ type: "file", request: data.request, bytes: copy }, [copy]);
    }
  } else if (data.type === "dispose") {
    runs.get(data.id)?.dispose();
    runs.delete(data.id);
  }
};
