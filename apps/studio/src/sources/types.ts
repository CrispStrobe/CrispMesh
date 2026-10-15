/**
 * The one interface the UI talks to, whatever is behind it: a recorded bundle (replay),
 * an engine over HTTP, or -- later -- the local engine of the Tauri shell.
 *
 * A `RunSource` delivers the events of one run and its files. An `Engine` lists and
 * starts runs and hands out `RunSource`s. Nothing here touches the DOM.
 */

import type { RunEvent } from "../core/events";
import type { ReplaySnapshot, ReplaySpeed } from "../core/replay";
import type { SettingSpec } from "../core/settings";
import type { StartPoint } from "../core/startPoints";

export type SourceKind = "replay" | "http" | "local" | "browser";

export type LinkState =
  /** First request not answered yet. */
  | "connecting"
  /** Events are flowing (or the recording is loaded). */
  | "live"
  /** A request failed; trying again with backoff. `message` says why. */
  | "retrying"
  /** The run has finished; nothing more will come. */
  | "ended"
  /** Gave up: unknown run, refused token, unreadable bundle. `message` says why. */
  | "failed";

export interface LinkStatus {
  state: LinkState;
  message?: string;
}

export type SourceUpdate =
  /** New events, in order, to append. */
  | { type: "events"; events: RunEvent[] }
  /** Forget everything and start again from these events (replay scrubbing backwards). */
  | { type: "reset"; events: RunEvent[] }
  | { type: "link"; link: LinkStatus };

export interface FetchOptions {
  signal?: AbortSignal;
  /** Bytes received so far and the total when the server said it. */
  onProgress?: (received: number, total: number | undefined) => void;
}

export interface ReplayControls {
  snapshot(): ReplaySnapshot;
  subscribe(listener: (snapshot: ReplaySnapshot) => void): () => void;
  play(): void;
  pause(): void;
  setSpeed(speed: ReplaySpeed): void;
  seek(position: number): void;
  step(delta: number): void;
}

export interface RunSource {
  readonly kind: SourceKind;
  /** Short human description: the bundle URL or the run id. */
  readonly title: string;
  /** Updates are delivered only between `start()` and `stop()`. */
  subscribe(listener: (update: SourceUpdate) => void): () => void;
  start(): void;
  /** Stops polling/playing and releases everything the source holds (object URLs, timers). */
  stop(): void;
  /** "Now" on the run's own time axis (Unix seconds), for elapsed-time displays. */
  now(): number;
  /** A URL an `<img>` can show for a run file. May be an object URL; owned by the source. */
  imageUrl(path: string): Promise<string>;
  fetchBytes(path: string, options?: FetchOptions): Promise<ArrayBuffer>;
  fetchJson(path: string, options?: FetchOptions): Promise<unknown>;
  /** Size of a run file in bytes without downloading it (HTTP HEAD), or undefined when it cannot be told. */
  fileSize?(path: string, signal?: AbortSignal): Promise<number | undefined>;
  /** A short note on what the run costs where it runs (the browser engine: its peak memory). */
  memoryNote?(): string | undefined;
  /** Where the photos of the run come from and under which license, when it was recorded. */
  attribution?(): Promise<string | null>;
  /** Present when the run can be cancelled from here. */
  cancel?(): Promise<void>;
  /** Present for recordings. */
  readonly replay?: ReplayControls;
}

/** An optional surface optimised against the photos after the mesh (the neural surface, the 2D splats), as the engine describes it. */
export interface MethodSpec {
  id: string;
  label: string;
  meaning: string;
  /** Whether this engine can run it. */
  available: boolean;
  qualities: { id: string; label: string; meaning: string }[];
  /** Single numeric settings that override the quality level's. */
  settings: { name: string; kind: string; meaning: string }[];
}

export interface EngineHealth {
  schema?: string;
  device?: string;
  canStartRuns: boolean;
  /** How a run may start here. Absent: what the contract guarantees (inputs, or scene + prepared + raw masks). */
  startPoints?: StartPoint[];
  /** False when the engine picks its device itself (the built-in engine: the system's GPU). */
  choosesDevice?: boolean;
  /** False when the engine cannot score against a reference scan. */
  scoresReference?: boolean;
  /** Where the engine keeps its data, when it runs on this computer and says so. */
  dataFolder?: string;
  /** The engine runs in the App Sandbox: it cannot start other programs or read folders the user did not pick. */
  sandboxed?: boolean;
  /** A sentence the engine wants shown on the "New run" form: how data gets to it. */
  note?: string;
  /** Files can be brought in from the device's own picker (`Engine.importFiles`). */
  imports?: boolean;
  /** Optional photo-optimised surfaces this engine knows. */
  methods?: MethodSpec[];
}

export interface RunSummary {
  id: string;
  status: string;
  /** Unix seconds, or null when the run has not written its first event. */
  started: number | null;
  stage: string | null;
  stageFraction: number;
  events: number;
}

/** A request the engine answered with an error status. `status` 0 means it could not be reached. */
export class EngineError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = "EngineError";
  }
}

export interface DataEntry {
  name: string;
  directory: boolean;
  /** A run can start from this folder. */
  inputs: boolean;
  /** A folder with photos a run can start from. */
  photos?: boolean;
  /** A lens calibration file. */
  calibration?: boolean;
}

export interface DataListing {
  /** The listed folder, relative to the data directory, "" for the top. */
  path: string;
  entries: DataEntry[];
}

export interface Engine {
  readonly kind: SourceKind;
  /** Where it is, without credentials. */
  readonly label: string;
  health(signal?: AbortSignal): Promise<EngineHealth>;
  settings(signal?: AbortSignal): Promise<SettingSpec[]>;
  listRuns(signal?: AbortSignal): Promise<RunSummary[]>;
  /** Returns the id of the new run. Throws `EngineError` with status 400 and the engine's sentence on invalid input. */
  startRun(body: Record<string, unknown>): Promise<string>;
  openRun(id: string): RunSource;
  /** Folders and files under the engine's data directory; `path` is relative to it ("" is the top). */
  listData?(path: string, signal?: AbortSignal): Promise<DataListing>;
  /**
   * Present when the engine runs on this computer inside the app: opens a native picker and
   * resolves to the chosen absolute path (null when cancelled), which the engine then accepts.
   */
  pickPath?(kind: "folder" | "file", title: string): Promise<string | null>;
  /** Lens calibration files the engine found on its computer, for the photos start. */
  /** Copies picked files into `<data folder>/<folder>/`; returns the last one's path relative to the data folder. */
  importFiles?(folder: string, files: File[], onProgress?: (done: number, total: number) => void): Promise<string>;
  calibrations?(): Promise<{ label: string; path: string }[]>;
}
