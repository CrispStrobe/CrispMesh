/**
 * The local source: the engine built into the app (crates/dense, running in the shell's
 * own process). This is the "Local (desktop shell)" row of docs/ENGINE-CONTRACT.md.
 *
 * The shell offers the same operations as the HTTP engine, one command each, and the run
 * directory on disk stays the source of truth: events are read from `events.jsonl` by line
 * number, files by their path relative to the run. So this file is the HTTP source with
 * `invoke` in place of `fetch`. The bridge is injected, which keeps it free of Tauri and
 * testable.
 */

import { normaliseEvent, type RunEvent } from "../core/events";
import { parseSettingsSchema, type SettingSpec } from "../core/settings";
import { parseStartPoints } from "../core/startPoints";
import { describe } from "./transport";
import {
  EngineError,
  type DataEntry,
  type DataListing,
  type Engine,
  type EngineHealth,
  type FetchOptions,
  type LinkStatus,
  type MethodSpec,
  type RunSource,
  type RunSummary,
  type SourceUpdate,
} from "./types";

/** Calls a command of the shell. Rejects with the shell's sentence (a string or an Error). */
export type Bridge = <T>(command: string, args?: Record<string, unknown> | Uint8Array, options?: { headers: Record<string, string> }) => Promise<T>;

export interface LocalTimer {
  set(callback: () => void, delayMs: number): unknown;
  clear(handle: unknown): void;
}

export interface LocalOptions {
  label?: string;
  timer?: LocalTimer;
  /** Wall clock in Unix seconds. */
  clock?: () => number;
  objectUrls?: { create(blob: Blob): string; revoke(url: string): void };
}

/** Reading a local file is cheap, so the log is looked at more often than over HTTP. */
export const LOCAL_POLL_MS = 400;
export const LOCAL_BACKOFF_MS = [500, 1000, 2000, 5000] as const;

const systemTimer: LocalTimer = {
  set: (callback, delayMs) => setTimeout(callback, delayMs),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

function refusal(problem: unknown, status: number): EngineError {
  const message = typeof problem === "string" ? problem : describe(problem);
  return new EngineError(message, status);
}

const IMAGE_TYPES: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", webp: "image/webp" };

export class LocalEngine implements Engine {
  readonly kind = "local" as const;
  readonly label: string;
  /** Whether the shell takes files from the device's picker (phones); known after `health`. */
  private imports = false;

  constructor(
    private readonly bridge: Bridge,
    private readonly options: LocalOptions = {},
  ) {
    this.label = options.label ?? "this computer";
  }

  async health(): Promise<EngineHealth> {
    const body = await this.bridge<Record<string, unknown> | null>("native_health").catch((problem) => {
      throw refusal(problem, 0);
    });
    const startPoints = parseStartPoints(body?.start_points);
    this.imports = body?.imports === true;
    if (this.imports) this.pickPath = undefined;
    return {
      schema: typeof body?.schema === "string" ? body.schema : undefined,
      device: typeof body?.device === "string" ? body.device : undefined,
      canStartRuns: body?.can_start_runs === true,
      startPoints: startPoints.length > 0 ? startPoints : undefined,
      choosesDevice: false,
      scoresReference: false,
      dataFolder: typeof body?.data_dir === "string" ? body.data_dir : undefined,
      sandboxed: body?.sandboxed === true,
      note: typeof body?.note === "string" && body.note !== "" ? body.note : undefined,
      imports: this.imports,
      methods: parseMethods(body?.methods),
    };
  }

  async settings(): Promise<SettingSpec[]> {
    return parseSettingsSchema(await this.bridge<unknown>("native_settings").catch((problem) => {
      throw refusal(problem, 0);
    }));
  }

  async listRuns(): Promise<RunSummary[]> {
    const body = await this.bridge<{ runs?: unknown } | null>("native_runs").catch((problem) => {
      throw refusal(problem, 0);
    });
    const runs: RunSummary[] = [];
    for (const row of Array.isArray(body?.runs) ? body.runs : []) {
      if (typeof row !== "object" || row === null) continue;
      const record = row as Record<string, unknown>;
      if (typeof record.id !== "string") continue;
      runs.push({
        id: record.id,
        status: typeof record.status === "string" ? record.status : "unknown",
        started: typeof record.started === "number" ? record.started : null,
        stage: typeof record.stage === "string" ? record.stage : null,
        stageFraction: typeof record.stage_fraction === "number" ? record.stage_fraction : 0,
        events: typeof record.events === "number" ? record.events : 0,
      });
    }
    return runs;
  }

  async startRun(body: Record<string, unknown>): Promise<string> {
    // The shell refuses an invalid request with one sentence, like the HTTP engine's 400.
    const answer = await this.bridge<{ id?: unknown } | null>("native_start", { body }).catch((problem) => {
      throw refusal(problem, 400);
    });
    if (typeof answer?.id !== "string") throw new EngineError("The engine did not return a run id.", 0);
    return answer.id;
  }

  async listData(path: string): Promise<DataListing> {
    const body = await this.bridge<{ path?: unknown; entries?: unknown } | null>("native_data", { path }).catch((problem) => {
      throw refusal(problem, 400);
    });
    const entries: DataEntry[] = [];
    for (const row of Array.isArray(body?.entries) ? body.entries : []) {
      if (typeof row !== "object" || row === null) continue;
      const record = row as Record<string, unknown>;
      if (typeof record.name !== "string" || record.name === "") continue;
      entries.push({ name: record.name, directory: record.directory === true, inputs: record.inputs === true });
    }
    return { path: typeof body?.path === "string" ? body.path : path, entries };
  }

  /**
   * Copies files the user picked on the device into `<data folder>/<folder>/`, one command
   * per file with its bytes as they are. Returns the last file's path relative to the data folder.
   */
  async importFiles(folder: string, files: File[], onProgress?: (done: number, total: number) => void): Promise<string> {
    if (!this.imports) throw new EngineError("This engine takes files from the data folder; it has no import.", 0);
    let last = "";
    for (const [index, file] of files.entries()) {
      const bytes = new Uint8Array(await file.arrayBuffer());
      try {
        const answer = await this.bridge<{ path: string }>("native_import", bytes, { headers: { "x-folder": encodeURIComponent(folder), "x-name": encodeURIComponent(file.name) } });
        last = answer.path;
      } catch (problem) {
        throw refusal(problem, 400);
      }
      onProgress?.(index + 1, files.length);
    }
    return last;
  }

  async calibrations(): Promise<{ label: string; path: string }[]> {
    const rows = await this.bridge<unknown>("native_calibrations").catch(() => []);
    const found: { label: string; path: string }[] = [];
    for (const row of Array.isArray(rows) ? rows : []) {
      const record = row as Record<string, unknown> | null;
      if (typeof record?.path === "string") found.push({ path: record.path, label: typeof record.label === "string" ? record.label : record.path });
    }
    return found;
  }

  /** The shell's native folder and file dialogs; absent on phones (known after `health`), where files come in through `importFiles`. */
  pickPath?: (kind: "folder" | "file", title: string) => Promise<string | null> = (kind, title) =>
    this.bridge<string | null>("pick_path", { kind, title, start: null });

  openRun(id: string): RunSource {
    return new LocalRunSource(this.bridge, id, this.options);
  }
}

export class LocalRunSource implements RunSource {
  readonly kind = "local" as const;
  private readonly timer: LocalTimer;
  private readonly clock: () => number;
  private readonly objectUrls: NonNullable<LocalOptions["objectUrls"]>;
  private listeners = new Set<(update: SourceUpdate) => void>();
  private link: LinkStatus = { state: "connecting" };
  private since = 0;
  private failures = 0;
  private running = false;
  private handle: unknown = null;
  private images = new Map<string, Promise<string>>();
  private created: string[] = [];

  constructor(
    private readonly bridge: Bridge,
    readonly title: string,
    options: LocalOptions = {},
  ) {
    this.timer = options.timer ?? systemTimer;
    this.clock = options.clock ?? (() => Date.now() / 1000);
    this.objectUrls = options.objectUrls ?? {
      create: (blob) => URL.createObjectURL(blob),
      revoke: (url) => URL.revokeObjectURL(url),
    };
  }

  subscribe(listener: (update: SourceUpdate) => void): () => void {
    this.listeners.add(listener);
    listener({ type: "link", link: this.link });
    return () => this.listeners.delete(listener);
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    void this.poll();
  }

  stop(): void {
    this.running = false;
    if (this.handle !== null) this.timer.clear(this.handle);
    this.handle = null;
    for (const url of this.created) this.objectUrls.revoke(url);
    this.created = [];
    this.images.clear();
  }

  now(): number {
    return this.clock();
  }

  /** The web view cannot read the run folder itself: the bytes come through the shell and are shown as an object URL. */
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
    const data = await this.bridge<ArrayBuffer | Uint8Array | number[]>("native_file", { id: this.title, path }).catch((problem) => {
      throw refusal(problem, 404);
    });
    options.signal?.throwIfAborted();
    let buffer: ArrayBuffer;
    if (data instanceof ArrayBuffer) buffer = data;
    else if (data instanceof Uint8Array) buffer = data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as ArrayBuffer;
    else buffer = new Uint8Array(data).buffer;
    options.onProgress?.(buffer.byteLength, buffer.byteLength);
    return buffer;
  }

  /** ATTRIBUTION.txt of the run, which the shell writes for runs from downloaded photos. */
  async attribution(): Promise<string | null> {
    try {
      return new TextDecoder().decode(await this.fetchBytes("ATTRIBUTION.txt")).trim() || null;
    } catch {
      return null;
    }
  }

  async fetchJson(path: string, options: FetchOptions = {}): Promise<unknown> {
    return JSON.parse(new TextDecoder().decode(await this.fetchBytes(path, options)));
  }

  async fileSize(path: string): Promise<number | undefined> {
    const size = await this.bridge<number>("native_file_size", { id: this.title, path }).catch(() => undefined);
    return typeof size === "number" && size > 0 ? size : undefined;
  }

  async cancel(): Promise<void> {
    await this.bridge("native_cancel", { id: this.title }).catch((problem) => {
      throw refusal(problem, 400);
    });
  }

  private async poll(): Promise<void> {
    if (!this.running) return;
    let delay = LOCAL_POLL_MS;
    try {
      const body = await this.bridge<{ events?: unknown; next?: unknown } | null>("native_events", { id: this.title, since: this.since });
      if (!this.running) return;
      const events = this.accept(body);
      this.failures = 0;
      if (events.length > 0) this.emit({ type: "events", events });
      if (events.some((event) => event.type === "run_finished")) {
        this.setLink({ state: "ended" });
        this.running = false;
        return;
      }
      this.setLink({ state: "live" });
    } catch (problem) {
      if (!this.running) return;
      const message = typeof problem === "string" ? problem : describe(problem);
      if (message === "unknown run") {
        this.setLink({ state: "failed", message: "There is no such run on this computer." });
        this.running = false;
        return;
      }
      delay = LOCAL_BACKOFF_MS[Math.min(this.failures, LOCAL_BACKOFF_MS.length - 1)]!;
      this.failures += 1;
      this.setLink({ state: "retrying", message });
    }
    this.handle = this.timer.set(() => {
      this.handle = null;
      void this.poll();
    }, delay);
  }

  /** Takes the events of one answer, in order, skipping any the cursor has already passed. */
  private accept(body: { events?: unknown; next?: unknown } | null): RunEvent[] {
    const events: RunEvent[] = [];
    let cursor = this.since;
    for (const row of Array.isArray(body?.events) ? body.events : []) {
      const event = normaliseEvent(row, cursor);
      if (event === null) {
        cursor += 1;
        continue;
      }
      if (event.seq < this.since) continue;
      events.push(event);
      cursor = event.seq + 1;
    }
    const next = typeof body?.next === "number" && Number.isFinite(body.next) ? body.next : cursor;
    this.since = Math.max(this.since, next, cursor);
    return events;
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

/** The engine's optional photo-optimised surfaces (`methods` of its health answer); malformed entries are left out. */
export function parseMethods(value: unknown): MethodSpec[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const text = (item: unknown): string | undefined => (typeof item === "string" ? item : undefined);
  const methods: MethodSpec[] = [];
  for (const entry of value) {
    if (entry === null || typeof entry !== "object") continue;
    const raw = entry as Record<string, unknown>;
    const id = text(raw.id);
    const label = text(raw.label);
    if (id === undefined || label === undefined || !Array.isArray(raw.qualities)) continue;
    const qualities = raw.qualities
      .map((quality) => (quality !== null && typeof quality === "object" ? (quality as Record<string, unknown>) : {}))
      .filter((quality) => text(quality.id) !== undefined)
      .map((quality) => ({ id: text(quality.id)!, label: text(quality.label) ?? text(quality.id)!, meaning: text(quality.meaning) ?? "" }));
    const settings = (Array.isArray(raw.settings) ? raw.settings : [])
      .map((setting) => (setting !== null && typeof setting === "object" ? (setting as Record<string, unknown>) : {}))
      .filter((setting) => text(setting.name) !== undefined)
      .map((setting) => ({ name: text(setting.name)!, kind: text(setting.kind) ?? "number", meaning: text(setting.meaning) ?? "" }));
    methods.push({ id, label, meaning: text(raw.meaning) ?? "", available: raw.available === true, qualities, settings });
  }
  return methods;
}
