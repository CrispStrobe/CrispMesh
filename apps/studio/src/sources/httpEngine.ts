/**
 * The HTTP engine of docs/ENGINE-CONTRACT.md section 3: list and start runs, poll a run's
 * events about once a second (backing off on errors), fetch its files.
 *
 * The bearer token is only ever put into the Authorization header. It is never logged,
 * never placed in a URL and never part of an error message.
 */

import { t } from "../i18n";
import { normaliseEvent, type RunEvent } from "../core/events";
import { parseSettingsSchema, type SettingSpec } from "../core/settings";
import { parseStartPoints } from "../core/startPoints";
import { parseMethods } from "./localEngine";
import { browserFetch, describe, encodePath, headSize, isAbort, readBytes, request, type FetchLike } from "./transport";
import {
  EngineError,
  type DataEntry,
  type DataListing,
  type Engine,
  type EngineHealth,
  type FetchOptions,
  type LinkStatus,
  type RunSource,
  type RunSummary,
  type SourceUpdate,
} from "./types";

export interface Timer {
  set(callback: () => void, delayMs: number): unknown;
  clear(handle: unknown): void;
}

const systemTimer: Timer = {
  set: (callback, delayMs) => setTimeout(callback, delayMs),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export interface HttpOptions {
  token?: string;
  /** Shown instead of the address (the desktop app's own engine is "This computer"). */
  label?: string;
  fetcher?: FetchLike;
  timer?: Timer;
  /** Wall clock in Unix seconds. */
  clock?: () => number;
  /** Creates/revokes object URLs for images fetched with a token. Defaults to the browser's. */
  objectUrls?: { create(blob: Blob): string; revoke(url: string): void };
}

export const POLL_MS = 1000;
export const BACKOFF_MS = [1000, 2000, 4000, 8000, 15000] as const;

/** `http://host:8765`, `host:8765/` and friends -> `http://host:8765`. Throws on anything that is not http(s). */
export function normaliseEngineUrl(input: string): string {
  let value = input.trim();
  if (value === "") throw new Error(t("engine.address.empty"));
  if (!/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(value)) value = "http://" + value;
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error(t("engine.address.invalid"));
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error(t("engine.address.scheme"));
  if (url.username !== "" || url.password !== "") throw new Error(t("engine.address.credentials"));
  return (url.origin + url.pathname).replace(/\/+$/, "");
}

export class HttpEngine implements Engine {
  readonly kind = "http" as const;
  readonly label: string;
  private readonly base: string;
  private readonly options: HttpOptions;
  private readonly fetcher: FetchLike;

  constructor(baseUrl: string, options: HttpOptions = {}) {
    this.base = normaliseEngineUrl(baseUrl);
    this.label = options.label ?? this.base;
    this.options = options;
    this.fetcher = options.fetcher ?? browserFetch;
  }

  private headers(extra: Record<string, string> = {}): Record<string, string> {
    return this.options.token ? { ...extra, Authorization: `Bearer ${this.options.token}` } : extra;
  }

  private async json(path: string, init: RequestInit = {}): Promise<unknown> {
    const response = await request(this.fetcher, `${this.base}/api/${path}`, {
      ...init,
      headers: this.headers(init.headers as Record<string, string> | undefined),
    });
    return response.json();
  }

  /**
   * Photos taken on this device, sent to the engine's data folder one by one (`POST /api/uploads`, an engine that
   * says `uploads` in its health answer, such as a desktop app that lets other devices use it). Set by `health`.
   */
  saveCapture?: (folder: string, files: File[], onProgress?: (done: number, total: number) => void) => Promise<string>;

  private async upload(folder: string, files: File[], onProgress?: (done: number, total: number) => void): Promise<string> {
    let last = "";
    for (const [index, file] of files.entries()) {
      const answer = (await this.json(`uploads?folder=${encodeURIComponent(folder)}&name=${encodeURIComponent(file.name)}`, {
        method: "POST",
        headers: { "Content-Type": "application/octet-stream" },
        body: file,
      })) as { path?: unknown } | null;
      if (typeof answer?.path !== "string") throw new EngineError(t("engine.photoNotKept"), 0);
      last = answer.path;
      onProgress?.(index + 1, files.length);
    }
    return last;
  }

  async health(signal?: AbortSignal): Promise<EngineHealth> {
    const body = (await this.json("health", { signal })) as Record<string, unknown> | null;
    if (typeof body !== "object" || body === null || (body.schema === undefined && body.can_start_runs === undefined)) {
      throw new EngineError(t("engine.notCrispMesh"), 0);
    }
    this.saveCapture = body.uploads === true ? (folder, files, onProgress) => this.upload(folder, files, onProgress) : undefined;
    return {
      schema: typeof body.schema === "string" ? body.schema : undefined,
      device: typeof body.device === "string" ? body.device : undefined,
      canStartRuns: body.can_start_runs === true,
      // An engine may say how runs can start there; otherwise the contract's three ways apply.
      startPoints: parseStartPoints(body.start_points).length > 0 ? parseStartPoints(body.start_points) : undefined,
      ...(Array.isArray(body.methods) ? { methods: parseMethods(body.methods) } : {}),
    };
  }

  async settings(signal?: AbortSignal): Promise<SettingSpec[]> {
    return parseSettingsSchema(await this.json("settings", { signal }));
  }

  async listRuns(signal?: AbortSignal): Promise<RunSummary[]> {
    const body = (await this.json("runs", { signal })) as { runs?: unknown } | null;
    const rows = Array.isArray(body?.runs) ? body.runs : [];
    const runs: RunSummary[] = [];
    for (const row of rows) {
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
    const answer = (await this.json("runs", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    })) as { id?: unknown } | null;
    if (typeof answer?.id !== "string") throw new EngineError(t("engine.noRunId"), 0);
    return answer.id;
  }

  async listData(path: string, signal?: AbortSignal): Promise<DataListing> {
    const body = (await this.json(`data?path=${encodeURIComponent(path)}`, { signal })) as {
      path?: unknown;
      entries?: unknown;
    } | null;
    const entries: DataEntry[] = [];
    for (const row of Array.isArray(body?.entries) ? body.entries : []) {
      if (typeof row !== "object" || row === null) continue;
      const record = row as Record<string, unknown>;
      if (typeof record.name !== "string" || record.name === "") continue;
      entries.push({
        name: record.name,
        directory: record.directory === true,
        inputs: record.inputs === true,
        ...(record.photos === true ? { photos: true } : {}),
        ...(record.calibration === true ? { calibration: true } : {}),
      });
    }
    return { path: typeof body?.path === "string" ? body.path : path, entries };
  }

  openRun(id: string): RunSource {
    return new HttpRunSource(this.base, id, this.options);
  }
}

export class HttpRunSource implements RunSource {
  readonly kind = "http" as const;
  readonly title: string;
  private readonly root: string;
  private readonly fetcher: FetchLike;
  private readonly timer: Timer;
  private readonly clock: () => number;
  private readonly token: string | undefined;
  private readonly objectUrls: NonNullable<HttpOptions["objectUrls"]>;
  private listeners = new Set<(update: SourceUpdate) => void>();
  private link: LinkStatus = { state: "connecting" };
  private since = 0;
  private failures = 0;
  private running = false;
  private handle: unknown = null;
  private abort: AbortController | null = null;
  private images = new Map<string, Promise<string>>();
  private created: string[] = [];

  constructor(base: string, id: string, options: HttpOptions = {}) {
    this.root = `${base}/api/runs/${encodeURIComponent(id)}`;
    this.title = id;
    this.token = options.token || undefined;
    this.fetcher = options.fetcher ?? browserFetch;
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
    this.abort = new AbortController();
    void this.poll();
  }

  stop(): void {
    this.running = false;
    this.abort?.abort();
    this.abort = null;
    if (this.handle !== null) this.timer.clear(this.handle);
    this.handle = null;
    for (const url of this.created) this.objectUrls.revoke(url);
    this.created = [];
    this.images.clear();
  }

  now(): number {
    return this.clock();
  }

  private headers(): Record<string, string> {
    return this.token ? { Authorization: `Bearer ${this.token}` } : {};
  }

  private fileUrl(path: string): string {
    return `${this.root}/files/${encodePath(path)}`;
  }

  imageUrl(path: string): Promise<string> {
    // Without a token the browser can load the file itself. With one, an <img> cannot send
    // the header, so the file is fetched here and handed over as an object URL.
    if (!this.token) return Promise.resolve(this.fileUrl(path));
    let pending = this.images.get(path);
    if (pending === undefined) {
      pending = request(this.fetcher, this.fileUrl(path), { headers: this.headers(), signal: this.abort?.signal })
        .then((response) => response.blob())
        .then((blob) => {
          const url = this.objectUrls.create(blob);
          this.created.push(url);
          return url;
        });
      pending.catch(() => this.images.delete(path));
      this.images.set(path, pending);
    }
    return pending;
  }

  async fetchBytes(path: string, options: FetchOptions = {}): Promise<ArrayBuffer> {
    const response = await request(this.fetcher, this.fileUrl(path), { headers: this.headers(), signal: options.signal });
    return readBytes(response, options);
  }

  async fetchJson(path: string, options: FetchOptions = {}): Promise<unknown> {
    return (await request(this.fetcher, this.fileUrl(path), { headers: this.headers(), signal: options.signal })).json();
  }

  fileSize(path: string, signal?: AbortSignal): Promise<number | undefined> {
    return headSize(this.fetcher, this.fileUrl(path), { headers: this.headers(), signal });
  }

  async cancel(): Promise<void> {
    await request(this.fetcher, `${this.root}/cancel`, { method: "POST", headers: this.headers() });
  }

  private async poll(): Promise<void> {
    if (!this.running) return;
    let delay = POLL_MS;
    try {
      const response = await request(this.fetcher, `${this.root}/events?since=${this.since}`, {
        headers: this.headers(),
        signal: this.abort?.signal,
        cache: "no-store",
      });
      const body = (await response.json()) as { events?: unknown; next?: unknown } | null;
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
      if (!this.running || isAbort(problem)) return;
      const status = problem instanceof EngineError ? problem.status : 0;
      if (status === 401 || status === 403 || status === 404) {
        // Asking again will not change the answer.
        this.setLink({ state: "failed", message: status === 404 ? t("engine.unknownRun") : describe(problem) });
        this.running = false;
        return;
      }
      delay = BACKOFF_MS[Math.min(this.failures, BACKOFF_MS.length - 1)]!;
      this.failures += 1;
      this.setLink({ state: "retrying", message: describe(problem) });
    }
    this.handle = this.timer.set(() => {
      this.handle = null;
      void this.poll();
    }, delay);
  }

  /** Takes the events of one answer, in order, skipping any the cursor has already passed. */
  private accept(body: { events?: unknown; next?: unknown } | null): RunEvent[] {
    const rows = Array.isArray(body?.events) ? body.events : [];
    const events: RunEvent[] = [];
    let cursor = this.since;
    for (const row of rows) {
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
