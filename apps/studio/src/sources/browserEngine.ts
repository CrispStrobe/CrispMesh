/**
 * "This browser": the dense pipeline running in a worker of this page (WebAssembly and
 * WebGPU, the package of crates/dense/web). The fourth way to get events and files,
 * behind the same interfaces as the others. Runs live in memory and are gone when the
 * page is closed.
 *
 * No DOM here: the worker is made by an injected factory, object URLs by injected
 * functions, so this is tested with a stand-in worker.
 */

import { localDecimal, t } from "../i18n";
import { normaliseEvent, type RunEvent } from "../core/events";
import { parseSettingsSchema, type SettingSpec } from "../core/settings";
import { parseStartPoints, type StartPoint } from "../core/startPoints";
import type { FromWorker, WorkerLike } from "../browser/protocol";
import { parseMethods, parseBaseSurfaces } from "./localEngine";
import {
  EngineError,
  type Engine,
  type EngineHealth,
  type FetchOptions,
  type LinkStatus,
  type RunSource,
  type RunSummary,
  type SourceUpdate,
} from "./types";

/**
 * Live previews cost memory in a browser: each preview volume is written and meshed
 * inside the 4 GiB that WebAssembly can address. Measured on 73 photos of 2 megapixels
 * (146 megapixels in all): 1.9 GiB at the peak without previews, 2.5 GiB with them.
 * Up to this many megapixels in all, previews stay within about 1 GiB and are on.
 */
export const PREVIEW_LIMIT_MEGAPIXELS = 60;

export interface PickedInputs {
  /** Name of the picked folder. */
  name: string;
  /** Path inside the inputs folder, and the file. */
  files: [string, File][];
  views: number;
  /** Sum over all photos, from cameras.json. */
  megapixels: number;
}

/** 0.4, 3.9, 146: megapixels as a person would say them. */
export function megapixelText(megapixels: number): string {
  return megapixels < 10 ? megapixels.toFixed(1) : String(Math.round(megapixels));
}

/** Whether live previews are on by default for these inputs, and why. */
export function previewAdvice(views: number, megapixels: number): { on: boolean; reason: string } {
  const size = t("engine.browser.preview.size", { count: views, megapixels: localDecimal(megapixelText(megapixels)) });
  if (megapixels <= PREVIEW_LIMIT_MEGAPIXELS) return { on: true, reason: t("engine.browser.preview.on", { size }) };
  return { on: false, reason: t("engine.browser.preview.off", { size }) };
}

/**
 * Files of a picked folder as paths inside it. `entries` carry each file's path including
 * the folder itself ("bunny/cameras.json"), as browsers report it; hidden files are dropped.
 */
export function relativeFiles(entries: { path: string; file: File }[]): [string, File][] {
  const files: [string, File][] = [];
  for (const { path, file } of entries) {
    const parts = path.split("/").filter((part) => part !== "");
    const inside = parts.slice(1);
    if (inside.length === 0 || inside.some((part) => part.startsWith("."))) continue;
    files.push([inside.join("/"), file]);
  }
  return files.sort((a, b) => a[0].localeCompare(b[0]));
}

/** Reads what the form needs from the picked folder's cameras.json. Throws a sentence if it is not an inputs folder. */
export function describeInputs(name: string, files: [string, File][], camerasJson: string): PickedInputs {
  let views: { width?: unknown; height?: unknown; image?: unknown; texture_image?: unknown; mask?: unknown }[];
  try {
    const parsed = JSON.parse(camerasJson) as { views?: unknown };
    if (!Array.isArray(parsed.views)) throw new Error();
    views = parsed.views as typeof views;
  } catch {
    throw new Error(t("engine.browser.inputs.unreadable"));
  }
  let pixels = 0;
  // A browser reads only the chosen folder: every photo and mask the cameras name has to be in it.
  const present = new Set(files.map(([path]) => path));
  const outside: string[] = [];
  for (const view of views) {
    if (typeof view.width === "number" && typeof view.height === "number") pixels += view.width * view.height;
    for (const named of [view.image, view.mask, view.texture_image]) {
      if (typeof named === "string" && !present.has(named.replace(/^\.\//, ""))) outside.push(named);
    }
  }
  if (outside.length > 0) {
    const absolute = outside.some((path) => path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path));
    throw new Error(
      t("engine.browser.inputs.outside", { count: outside.length, first: outside[0]! }) +
        " " +
        (absolute ? t("engine.browser.inputs.absolute") + " " : "") +
        t("engine.browser.inputs.inside"),
    );
  }
  return { name, files, views: views.length, megapixels: pixels / 1e6 };
}

/** The browser's own start points, in the active language (made fresh, so a language change shows). */
const browserStart = (): StartPoint[] => [
  {
    id: "inputs",
    label: t("engine.browser.start.inputs.label"),
    meaning: t("engine.browser.start.inputs.meaning"),
    fields: [{ key: "inputs", label: t("engine.browser.start.inputs.label"), kind: "inputs", required: true, help: t("engine.browser.start.inputs.help") }],
    providers: [],
    optionGroups: [],
  },
  {
    id: "photos",
    label: t("engine.browser.start.photos.label"),
    meaning: t("engine.browser.start.photos.meaning"),
    fields: [
      { key: "photos", label: t("engine.browser.start.photos.photos"), kind: "folder", required: true, help: t("engine.browser.start.photos.photosHelp") },
      { key: "calibration", label: t("engine.browser.start.photos.calibration"), kind: "file", required: true, automatic: true, help: t("engine.browser.start.photos.calibrationHelp") },
    ],
    providers: [
      {
        module: "masks",
        label: t("engine.browser.masks"),
        default: "threshold",
        settings: [],
        options: [
          { id: "threshold", label: "threshold", meaning: t("engine.browser.masks.threshold"), available: true, external: [], settings: [], inputs: [] },
          { id: "background", label: "background", meaning: t("engine.browser.masks.background"), available: true, external: [], settings: [], inputs: [] },
          { id: "import", label: "import", meaning: t("engine.browser.masks.import"), available: false, reason: t("engine.browser.masks.importReason"), external: [], settings: [], inputs: [] },
        ],
      },
      {
        module: "cameras",
        label: t("engine.browser.cameras"),
        default: "turntable",
        settings: [],
        options: [
          { id: "turntable", label: "turntable", meaning: t("engine.browser.cameras.turntable"), available: true, external: [], settings: [], inputs: [] },
          { id: "markers", label: "markers", meaning: t("engine.browser.cameras.markers"), available: false, reason: t("engine.browser.cameras.markersReason"), external: [], settings: [], inputs: [] },
          { id: "import", label: "import", meaning: t("engine.browser.cameras.import"), available: false, reason: t("engine.browser.cameras.importReason"), external: [], settings: [], inputs: [] },
        ],
      },
    ],
    optionGroups: [],
  },
];

const PLACE_KINDS = new Set(["path", "directory", "executable", "path_list"]);

/** Why the worker could not load the engine, as text in the page's language. */
const WORKER_REASONS = { "no-webgpu": "engine.browser.noWebGpu", "no-adapter": "engine.browser.noAdapter" } as const;

/**
 * The start points of the package's describe(), shaped for this browser: an inputs folder and
 * turntable photos (the scene start needs a camera solution on disk). Providers that run in a
 * browser stay; those that need a path of their own (imported masks or cameras, the marker mat)
 * are listed as not available, with the reason; options that name a place are left out. Returns
 * [] when the description cannot be used, so the caller keeps its own list.
 */
export function browserStartPoints(described: unknown): StartPoint[] {
  const raw = (described as { start_points?: unknown } | null)?.start_points;
  if (!Array.isArray(raw)) return [];
  const tunable = (rows: unknown) => (Array.isArray(rows) ? rows.filter((row) => !PLACE_KINDS.has((row as { kind?: string }).kind ?? "")) : []);
  const shaped = raw
    .filter((point) => ["inputs", "photos"].includes((point as { id?: string }).id ?? ""))
    .map((point) => {
      const p = point as Record<string, unknown>;
      const providers = (Array.isArray(p.providers) ? p.providers : []).map((choice) => {
        const c = choice as Record<string, unknown>;
        const options = (Array.isArray(c.options) ? c.options : [])
          .filter((option) => (option as { platforms?: { wasm?: boolean } }).platforms?.wasm !== false)
          .map((option) => {
            const o = option as Record<string, unknown>;
            const needsPath = o.id === "import" || (Array.isArray(o.settings) && o.settings.some((row) => PLACE_KINDS.has((row as { kind?: string }).kind ?? "") && (row as { name?: string }).name !== "turntable-matches"));
            return {
              ...o,
              available: o.available !== false && !needsPath,
              reason: needsPath ? t("engine.browser.notYet") : o.reason,
              settings: tunable(o.settings),
              inputs: [],
            };
          });
        return { ...c, options, settings: tunable(c.settings) };
      });
      const groups = (Array.isArray(p.option_groups) ? p.option_groups : [])
        .map((group) => ({ ...(group as Record<string, unknown>), id: (group as { id?: unknown }).id, settings: tunable((group as { settings?: unknown }).settings) }))
        .filter((group) => !["tools", "deadlines", "machine"].includes(String(group.id)) && group.settings.length > 0);
      // The browser's own fields: a folder of photos and a calibration are picked here, not typed.
      return { ...p, providers, option_groups: groups };
    });
  const points = ownWording(parseStartPoints(shaped));
  return points.length === 2 ? points : [];
}

/** Keeps the browser's wording (active language) for what is picked on this device. */
function ownWording(points: StartPoint[]): StartPoint[] {
  const own = browserStart();
  for (const point of points) {
    const mine = own.find((candidate) => candidate.id === point.id);
    if (mine !== undefined) {
      point.meaning = mine.meaning;
      point.fields = mine.fields;
    }
  }
  return points;
}

/** Photos picked on this device for the photos start, sorted by name (the capture order). */
export interface PickedPhotos {
  name: string;
  files: [string, File][];
}

/** Images among picked files, by file name, in natural name order ("2.png" before "10.png"). Hidden files are dropped. */
export function photoFiles(files: { name: string; file: File }[]): [string, File][] {
  const image = /\.(png|jpe?g)$/i;
  const chosen = files.filter(({ name }) => image.test(name) && !name.startsWith("."));
  const seen = new Set<string>();
  const unique = chosen.filter(({ name }) => !seen.has(name) && seen.add(name) !== undefined);
  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
  return unique.sort((a, b) => collator.compare(a.name, b.name)).map(({ name, file }) => [name, file]);
}

/** The label of the automatic lens in the calibration picker and the run's record. */
export const AUTOMATIC_LENS = "automatic (focal length found from the turn)";

/** A calibration file is JSON that says what it is. Throws a sentence otherwise. */
export function checkCalibration(text: string): string {
  let parsed: { schema?: unknown };
  try {
    parsed = JSON.parse(text) as { schema?: unknown };
  } catch {
    throw new Error(t("engine.browser.calibration.notJson"));
  }
  if (parsed.schema !== "crispmesh_lens_calibration_v1") throw new Error(t("engine.browser.calibration.notCalibration"));
  return text;
}

interface Held {
  id: string;
  events: RunEvent[];
  status: string;
  started: number;
  stage: string | null;
  fraction: number;
  /** Peak WebAssembly memory seen for this run, bytes. */
  peak: number;
  ended: boolean;
  attribution?: string;
  listeners: Set<() => void>;
}

export interface BrowserOptions {
  /** Absolute URL of `crispmesh-dense.js` of the engine package. */
  module: string;
  createWorker(): WorkerLike;
  /** Wall clock in Unix seconds. */
  clock?: () => number;
  objectUrls?: { create(blob: Blob): string; revoke(url: string): void };
  /** For the shipped calibrations; injected in tests. */
  fetcher?: (url: string) => Promise<Response>;
  /** Base URL the shipped files are relative to (default: the page's). */
  base?: string;
}

const IMAGE_TYPES: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg" };

export class BrowserEngine implements Engine {
  readonly kind = "browser" as const;
  readonly label = "this browser";
  private worker: WorkerLike | null = null;
  private loaded: Promise<{ settings: unknown; adapter: string | null; described?: unknown; threads?: number }> | null = null;
  private resolveLoaded: ((value: { settings: unknown; adapter: string | null; described?: unknown; threads?: number }) => void) | null = null;
  /** The start points this browser offers, from the package's describe() once loaded. */
  /** From the package's describe(); null until then (the browser's own list is used). */
  private startPoints: StartPoint[] | null = null;
  private rejectLoaded: ((problem: Error) => void) | null = null;
  private runs = new Map<string, Held>();
  private requests = new Map<number, (bytes: ArrayBuffer | null) => void>();
  private nextRequest = 1;
  private counter = 0;
  private picked: PickedInputs | null = null;
  private previews: boolean | null = null;
  private photos: PickedPhotos | null = null;
  private calibration: { label: string; text: string } | null = null;
  private readonly clock: () => number;

  constructor(private readonly options: BrowserOptions) {
    this.clock = options.clock ?? (() => Date.now() / 1000);
  }

  /** The folder a run will start from, and whether intermediate surfaces are wanted (null: by the advice). */
  setInputs(picked: PickedInputs | null, livePreviews: boolean | null = null): void {
    this.picked = picked;
    this.previews = livePreviews;
  }

  inputs(): PickedInputs | null {
    return this.picked;
  }

  /** The photos for the photos start, and whether intermediate surfaces are wanted (null: off, the default for photo sets). */
  setPhotos(photos: PickedPhotos | null, livePreviews: boolean | null = null): void {
    this.photos = photos;
    this.previews = livePreviews;
  }

  pickedPhotos(): PickedPhotos | null {
    return this.photos;
  }

  setCalibration(calibration: { label: string; text: string } | null): void {
    this.calibration = calibration;
  }

  pickedCalibration(): { label: string; text: string } | null {
    return this.calibration;
  }

  /** No lens file: the turntable solver finds the focal length (centred principal point, no distortion). */
  useAutomaticLens(): void {
    this.calibration = { label: AUTOMATIC_LENS, text: "auto" };
  }

  /** Lens calibrations shipped with the app (calibrations/index.json next to it). */
  async calibrations(): Promise<{ label: string; path: string }[]> {
    const fetcher = this.options.fetcher ?? ((url: string) => fetch(url));
    try {
      const response = await fetcher(new URL("calibrations/index.json", this.options.base ?? document.baseURI).href);
      if (!response.ok) return [];
      const list = (await response.json()) as { file?: unknown; label?: unknown }[];
      return list.filter((row) => typeof row.file === "string").map((row) => ({ label: typeof row.label === "string" ? row.label : String(row.file), path: `calibrations/${String(row.file)}` }));
    } catch {
      return [];
    }
  }

  /** Reads a shipped calibration (a path from `calibrations()`) and makes it the run's. */
  async useShippedCalibration(path: string): Promise<void> {
    const fetcher = this.options.fetcher ?? ((url: string) => fetch(url));
    const response = await fetcher(new URL(path, this.options.base ?? document.baseURI).href);
    if (!response.ok) throw new EngineError(`${path}: not found`, response.status);
    this.calibration = { label: path.split("/").pop() ?? path, text: checkCalibration(await response.text()) };
  }

  /** Peak WebAssembly memory of a run in bytes, as far as seen. */
  peakMemory(id: string): number {
    return this.runs.get(id)?.peak ?? 0;
  }

  private load(): Promise<{ settings: unknown; adapter: string | null; described?: unknown; threads?: number }> {
    if (this.loaded === null) {
      this.loaded = new Promise((resolve, reject) => {
        this.resolveLoaded = resolve;
        this.rejectLoaded = reject;
      });
      try {
        const worker = this.options.createWorker();
        worker.onmessage = ({ data }) => this.receive(data);
        worker.onerror = () => this.rejectLoaded?.(new EngineError(t("engine.browser.workerFailed"), 0));
        this.worker = worker;
        worker.postMessage({ type: "load", module: this.options.module });
      } catch {
        this.rejectLoaded?.(new EngineError(t("engine.browser.noWorker"), 0));
      }
    }
    return this.loaded;
  }

  private receive(message: FromWorker): void {
    if (message.type === "loaded") {
      if (message.ok) {
        const points = browserStartPoints(message.described);
        if (points.length > 0) this.startPoints = points;
        this.resolveLoaded?.({ settings: message.settings, adapter: message.adapter, described: message.described, threads: message.threads });
      }
      else this.rejectLoaded?.(new EngineError(message.reason !== undefined ? t(WORKER_REASONS[message.reason]) : message.message, 0));
    } else if (message.type === "event") {
      const run = this.runs.get(message.id);
      if (run === undefined) return;
      const event = normaliseEvent(message.event, run.events.length);
      if (event === null) return;
      // Line numbers as in a log on disk: the position in the list.
      run.events.push({ ...event, seq: run.events.length });
      run.peak = Math.max(run.peak, message.wasmBytes);
      if (event.type === "run_started") run.status = "running";
      else if (event.type === "stage_started") {
        run.stage = event.stage;
        run.fraction = 0;
      } else if (event.type === "progress" && typeof event.fraction === "number") run.fraction = event.fraction;
      else if (event.type === "run_finished") {
        run.status = typeof event.status === "string" ? event.status : "failed";
        if (run.status === "complete") run.stage = null;
      }
      for (const listener of [...run.listeners]) listener();
    } else if (message.type === "finished") {
      const run = this.runs.get(message.id);
      if (run === undefined) return;
      run.peak = Math.max(run.peak, message.peakWasmBytes);
      // The engine closes its own log. If it could not even start one, say why here.
      if (!run.events.some((event) => event.type === "run_finished")) {
        const time = this.clock();
        const add = (type: string, fields: Record<string, unknown>) => run.events.push({ type, time, stage: null, seq: run.events.length, ...fields });
        if (run.events.length === 0) add("run_started", { schema: "crispmesh_dense_events_v1", device: "WebGPU" });
        add("error", { message: message.message ?? t("engine.browser.stoppedSilently") });
        add("run_finished", { status: "failed", seconds: message.seconds });
        run.status = "failed";
      }
      run.ended = true;
      for (const listener of [...run.listeners]) listener();
    } else if (message.type === "file") {
      this.requests.get(message.request)?.(message.bytes);
      this.requests.delete(message.request);
    }
  }

  /** The start points to offer: the package's, or the browser's own; picked fields in the active language. */
  private currentStartPoints(): StartPoint[] {
    return this.startPoints === null ? browserStart() : ownWording(this.startPoints);
  }

  async health(): Promise<EngineHealth> {
    const { adapter, threads, described } = await this.load();
    const lanes = (threads ?? 1) > 1 ? t("engine.browser.threads", { count: threads ?? 1 }) : t("engine.browser.singleThreaded");
    return {
      schema: "crispmesh_dense_events_v1",
      device: `${adapter === null ? "WebGPU" : `WebGPU: ${adapter}`} · ${lanes}`,
      canStartRuns: true,
      startPoints: this.currentStartPoints(),
      choosesDevice: false,
      scoresReference: false,
      baseSurfaces: parseBaseSurfaces((described as { base_surfaces?: unknown } | null | undefined)?.base_surfaces),
      methods: parseMethods((described as { methods?: unknown } | null | undefined)?.methods),
    };
  }

  async settings(): Promise<SettingSpec[]> {
    return parseSettingsSchema((await this.load()).settings);
  }

  listRuns(): Promise<RunSummary[]> {
    const runs = [...this.runs.values()].map((run) => ({
      id: run.id,
      status: run.status,
      started: run.started,
      stage: run.stage,
      stageFraction: run.fraction,
      events: run.events.length,
    }));
    return Promise.resolve(runs.sort((a, b) => b.started - a.started || b.id.localeCompare(a.id)));
  }

  async startRun(body: Record<string, unknown>): Promise<string> {
    await this.load();
    if (body.photos !== undefined) return this.startFromPhotos(body);
    if (this.picked === null) throw new EngineError(`inputs: ${t("engine.browser.chooseInputs")}`, 400);
    if ([...this.runs.values()].some((run) => !run.ended)) {
      throw new EngineError(t("engine.browser.busy"), 400);
    }
    const name = typeof body.name === "string" && body.name.trim() !== "" ? body.name.trim() : this.picked.name;
    const id = `browser-${String(++this.counter).padStart(2, "0")}-${name.replace(/[^A-Za-z0-9._-]+/g, "-").slice(0, 40) || "run"}`;
    const previews = this.previews ?? previewAdvice(this.picked.views, this.picked.megapixels).on;
    const options: Record<string, unknown> = { live_previews: previews };
    if (body.texture === true) options.texture = true;
    if (typeof body.settings === "object" && body.settings !== null) options.settings = body.settings;
    Object.assign(options, methodOptions(body));
    this.runs.set(id, { id, events: [], status: "running", started: this.clock(), stage: null, fraction: 0, peak: 0, ended: false, listeners: new Set() });
    this.worker?.postMessage({ type: "run", id, files: this.picked.files, options });
    return id;
  }

  private startFromPhotos(body: Record<string, unknown>): string {
    if (this.photos === null || this.photos.files.length === 0) throw new EngineError(`photos: ${t("engine.browser.choosePhotos")}`, 400);
    if (this.photos.files.length < 3) throw new EngineError(`photos: ${t("engine.browser.tooFewPhotos")}`, 400);
    if (this.calibration === null) throw new EngineError(`calibration: ${t("engine.browser.chooseCalibration")}`, 400);
    if ([...this.runs.values()].some((run) => !run.ended)) {
      throw new EngineError(t("engine.browser.busy"), 400);
    }
    const providers = (typeof body.providers === "object" && body.providers !== null ? body.providers : {}) as Record<string, unknown>;
    const point = this.currentStartPoints().find((candidate) => candidate.id === "photos");
    const pick = (module: string, fallback: string): string => {
      const choice = point?.providers.find((candidate) => candidate.module === module);
      const wanted = typeof providers[module] === "string" ? (providers[module] as string) : (choice?.default ?? fallback);
      const option = choice?.options.find((candidate) => candidate.id === wanted);
      if (choice !== undefined && (option === undefined || !option.available)) {
        throw new EngineError(`${module}: ${option?.reason ?? t("engine.browser.noProvider", { name: wanted })}`, 400);
      }
      return wanted;
    };
    const masks = pick("masks", "threshold");
    const cameras = pick("cameras", "turntable");
    const extra = Array.isArray(body.photo_options) ? body.photo_options.filter((word): word is string => typeof word === "string") : [];
    const name = typeof body.name === "string" && body.name.trim() !== "" ? body.name.trim() : this.photos.name;
    const id = `browser-${String(++this.counter).padStart(2, "0")}-${name.replace(/[^A-Za-z0-9._-]+/g, "-").slice(0, 40) || "run"}`;
    // Photo sets are large: intermediate surfaces only when asked for.
    const options: Record<string, unknown> = { live_previews: this.previews ?? false, photo_options: ["--masks", masks, "--cameras", cameras, ...extra] };
    if (body.texture === true) options.texture = true;
    if (typeof body.settings === "object" && body.settings !== null) options.settings = body.settings;
    Object.assign(options, methodOptions(body));
    const attribution = typeof body.attribution === "string" && body.attribution.trim() !== "" ? body.attribution.trim() : undefined;
    if (attribution !== undefined) options.attribution = attribution;
    this.runs.set(id, { id, events: [], status: "running", started: this.clock(), stage: null, fraction: 0, peak: 0, ended: false, listeners: new Set(), attribution });
    this.worker?.postMessage({ type: "run-photos", id, photos: this.photos.files, calibration: this.calibration.text, options });
    return id;
  }

  openRun(id: string): RunSource {
    return new BrowserRunSource(this, id, this.options.objectUrls, this.clock);
  }

  /** For the run source. */
  held(id: string): Held | undefined {
    return this.runs.get(id);
  }

  cancel(id: string): void {
    this.worker?.postMessage({ type: "cancel", id });
  }

  file(id: string, path: string): Promise<ArrayBuffer | null> {
    if (this.worker === null || !this.runs.has(id)) return Promise.resolve(null);
    const request = this.nextRequest++;
    return new Promise((resolve) => {
      this.requests.set(request, resolve);
      this.worker?.postMessage({ type: "file", request, id, path });
    });
  }

  /** Forgets a finished run and frees what it holds in the engine's memory. */
  remove(id: string): void {
    if (this.runs.get(id)?.ended !== true) return;
    this.worker?.postMessage({ type: "dispose", id });
    this.runs.delete(id);
  }
}

/** The optional photo-optimised surfaces of a start body (`surface`, `splats`: `{quality, settings}`), as run options. */
export function methodOptions(body: Record<string, unknown>): Record<string, unknown> {
  const options: Record<string, unknown> = {};
  if (body.base_surface === "silhouette" || body.base_surface === "stereo") options.base_surface = body.base_surface;
  for (const id of ["surface", "splats"]) {
    const method = body[id] as { quality?: unknown; settings?: unknown } | undefined;
    if (typeof method !== "object" || method === null || typeof method.quality !== "string") continue;
    const settings = Array.isArray(method.settings) ? method.settings.filter((item): item is string => typeof item === "string") : [];
    options[id] = { quality: method.quality, settings };
  }
  return options;
}

export class BrowserRunSource implements RunSource {
  readonly kind = "browser" as const;
  private listeners = new Set<(update: SourceUpdate) => void>();
  private link: LinkStatus = { state: "connecting" };
  private delivered = 0;
  private detach: (() => void) | null = null;
  private images = new Map<string, Promise<string>>();
  private created: string[] = [];
  private readonly objectUrls: NonNullable<BrowserOptions["objectUrls"]>;

  constructor(
    private readonly engine: BrowserEngine,
    readonly title: string,
    objectUrls: BrowserOptions["objectUrls"],
    private readonly clock: () => number,
  ) {
    this.objectUrls = objectUrls ?? { create: (blob) => URL.createObjectURL(blob), revoke: (url) => URL.revokeObjectURL(url) };
  }

  subscribe(listener: (update: SourceUpdate) => void): () => void {
    this.listeners.add(listener);
    listener({ type: "link", link: this.link });
    return () => this.listeners.delete(listener);
  }

  start(): void {
    const run = this.engine.held(this.title);
    if (run === undefined) {
      this.setLink({ state: "failed", message: t("engine.browser.runGone") });
      return;
    }
    const deliver = () => {
      if (run.events.length > this.delivered) {
        const events = run.events.slice(this.delivered);
        this.delivered = run.events.length;
        this.emit({ type: "events", events });
      }
      this.setLink({ state: run.events.some((event) => event.type === "run_finished") ? "ended" : "live" });
    };
    run.listeners.add(deliver);
    this.detach = () => run.listeners.delete(deliver);
    deliver();
  }

  stop(): void {
    this.detach?.();
    this.detach = null;
    this.delivered = 0;
    for (const url of this.created) this.objectUrls.revoke(url);
    this.created = [];
    this.images.clear();
  }

  now(): number {
    return this.clock();
  }

  imageUrl(path: string): Promise<string> {
    let pending = this.images.get(path);
    if (pending === undefined) {
      pending = this.fetchBytes(path).then((bytes) => {
        const extension = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
        const url = this.objectUrls.create(new Blob([bytes], { type: IMAGE_TYPES[extension] ?? "application/octet-stream" }));
        this.created.push(url);
        return url;
      });
      pending.catch(() => this.images.delete(path));
      this.images.set(path, pending);
    }
    return pending;
  }

  async fetchBytes(path: string, options: FetchOptions = {}): Promise<ArrayBuffer> {
    const bytes = await this.engine.file(this.title, path);
    options.signal?.throwIfAborted();
    if (bytes === null) throw new EngineError(t("engine.notFound"), 404);
    options.onProgress?.(bytes.byteLength, bytes.byteLength);
    return bytes;
  }

  async fetchJson(path: string, options: FetchOptions = {}): Promise<unknown> {
    return JSON.parse(new TextDecoder().decode(await this.fetchBytes(path, options)));
  }

  attribution(): Promise<string | null> {
    return Promise.resolve(this.engine.held(this.title)?.attribution ?? null);
  }

  /** Peak memory of the engine for this run, for the status line. */
  memoryNote(): string | undefined {
    const peak = this.engine.peakMemory(this.title);
    return peak > 0 ? `engine memory at the peak ${(peak / 2 ** 30).toFixed(peak >= 2 ** 30 ? 1 : 2)} GiB` : undefined;
  }

  cancel(): Promise<void> {
    this.engine.cancel(this.title);
    return Promise.resolve();
  }

  private setLink(link: LinkStatus): void {
    if (link.state === this.link.state && link.message === this.link.message) return;
    this.link = link;
    this.emit({ type: "link", link });
  }

  private emit(update: SourceUpdate): void {
    for (const listener of [...this.listeners]) listener(update);
  }
}
