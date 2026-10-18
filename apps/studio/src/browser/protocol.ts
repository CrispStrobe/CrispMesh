/** Messages between the page and the worker that runs the engine in the browser. */

export type ToWorker =
  /** Load the engine package from `module` (an absolute URL) and say whether WebGPU is there. */
  | { type: "load"; module: string }
  /** Start a run on the picked files. `files` are paths inside the inputs folder with their File. */
  | { type: "run"; id: string; files: [string, File][]; options: Record<string, unknown> }
  /** Start a run from turntable photos (name, File; one turn in name order) and a lens calibration (JSON text). */
  | { type: "run-photos"; id: string; photos: [string, File][]; calibration: string; options: Record<string, unknown> }
  | { type: "cancel"; id: string }
  /** Ask for a file of a run's directory, held in the engine's memory. */
  | { type: "file"; request: number; id: string; path: string }
  /** Free everything a run holds. */
  | { type: "dispose"; id: string };

export type FromWorker =
  /** `described`: the package's describe() (start points, providers, their options), or null from an older package. */
  | { type: "loaded"; ok: true; settings: unknown; adapter: string | null; described?: unknown; threads?: number }
  /** `reason`: a known failure the page words in its own language; `message` otherwise (English). */
  | { type: "loaded"; ok: false; message: string; reason?: "no-webgpu" | "no-adapter" }
  | { type: "event"; id: string; event: unknown; wasmBytes: number }
  | { type: "finished"; id: string; ok: boolean; message?: string; seconds: number; peakWasmBytes: number }
  | { type: "file"; request: number; bytes: ArrayBuffer | null };

/** What a worker looks like to the engine; lets tests stand one in. */
export interface WorkerLike {
  postMessage(message: ToWorker): void;
  onmessage: ((event: { data: FromWorker }) => void) | null;
  onerror?: ((event: unknown) => void) | null;
  terminate(): void;
}
