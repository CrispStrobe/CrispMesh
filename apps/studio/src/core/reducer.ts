/**
 * The one pure reducer every data source feeds: (state, event) -> state.
 * No DOM, no I/O, no clocks. Unknown event types, artifact kinds and fields are ignored.
 */

import { finite, SCHEMA, text, type RunEvent } from "./events";
import { t } from "../i18n";

export const KNOWN_STAGES = ["inputs", "stereo", "mesh", "check", "evaluate"] as const;

export type StageStatus = "pending" | "running" | "done" | "failed" | "cancelled" | "skipped";
export type RunStatus = "waiting" | "running" | "complete" | "failed" | "cancelled";

export interface StageState {
  name: string;
  status: StageStatus;
  /** Event time (Unix seconds) of `stage_started`. */
  startedAt?: number;
  /** Duration reported by `stage_finished`. */
  seconds?: number;
  /** 0..1 within the stage. */
  fraction: number;
  message: string;
  /** Set when the stage was only inferred to have happened from an artifact it produced (no `stage_started`). */
  implicit?: boolean;
  /** Time of the stage's latest event; freezes the elapsed clock of a stage that stopped without `stage_finished`. */
  lastEventAt?: number;
}

export interface MeshStep {
  seq: number;
  kind: "preview_mesh" | "final_mesh";
  path: string;
  label: string;
  stage: string | null;
  triangles?: number;
  time: number;
}

/** Sheet kinds of the contract, in the order they normally appear. */
export const SHEET_KINDS = [
  "mask_sheet",
  "sparse_overlay",
  "input_sheet",
  "mask_repair_sheet",
  "hull_mask_sheet",
  "depth_sheet",
  "photo_overlay",
  "preview_render",
  "scan_overlay",
] as const;
export type SheetKind = (typeof SHEET_KINDS)[number];

/** One step of the inspection: the same views after each surface (`inspection_sheet`, field `step`). */
export interface Inspection {
  seq: number;
  path: string;
  /** `00-hull`, `01-hull-repaired`, `1N-level-N`, `90-final`, or `steps` for the overview of all. */
  step: string;
  label: string;
}

/** A readable name for an inspection step when the event gives no label. */
export function inspectionTitle(step: string): string {
  if (step === "steps") return t("run.inspection.allSteps");
  if (/^\d+-hull$/.test(step)) return t("run.inspection.hull");
  if (/^\d+-hull-repaired$/.test(step)) return t("run.inspection.hullRepaired");
  const level = /^\d+-level-(\d+)$/.exec(step);
  if (level !== null) return t("run.inspection.afterLevel", { level: Number(level[1]) + 1 });
  if (/^\d+-final$/.test(step)) return t("run.inspection.final");
  return step;
}

/** Inspection steps in the order of the pipeline, the overview last. */
export function orderInspections(steps: Inspection[]): Inspection[] {
  return [...steps].sort((a, b) => (a.step === "steps" ? 1 : 0) - (b.step === "steps" ? 1 : 0) || a.step.localeCompare(b.step, undefined, { numeric: true }) || a.seq - b.seq);
}

export interface Sheet {
  seq: number;
  kind: SheetKind;
  path: string;
  label: string;
  stage: string | null;
  level?: number;
  time: number;
}

export interface ReportRef {
  seq: number;
  path: string;
  label: string;
  stage: string | null;
}

export interface Metric {
  name: string;
  value: number | string | boolean;
  stage: string | null;
  seq: number;
}

export interface RunError {
  seq: number;
  stage: string | null;
  message: string;
}

export interface RunState {
  status: RunStatus;
  schema?: string;
  /** True when `run_started` named a schema this front end does not know. It still shows what it understands. */
  schemaUnknown: boolean;
  device?: string;
  inputs?: string;
  configuration?: Record<string, unknown>;
  startedAt?: number;
  /** Total duration reported by `run_finished`. */
  seconds?: number;
  stages: StageState[];
  meshes: MeshStep[];
  sheets: Sheet[];
  inspections: Inspection[];
  reports: ReportRef[];
  downloads: ReportRef[];
  metrics: Metric[];
  errors: RunError[];
  /** Events applied so far, including ignored ones. */
  eventCount: number;
  lastSeq: number;
  /** Time of the newest event (Unix seconds). */
  lastTime?: number;
  /** Events whose type or artifact kind was not understood. Informational only. */
  ignored: number;
}

export function initialState(): RunState {
  return {
    status: "waiting",
    schemaUnknown: false,
    stages: KNOWN_STAGES.map((name) => ({ name, status: "pending", fraction: 0, message: "" })),
    meshes: [],
    sheets: [],
    inspections: [],
    reports: [],
    downloads: [],
    metrics: [],
    errors: [],
    eventCount: 0,
    lastSeq: -1,
    ignored: 0,
  };
}

export function reduceAll(events: readonly RunEvent[], from: RunState = initialState()): RunState {
  let state = from;
  for (const event of events) state = reduce(state, event);
  return state;
}

export function reduce(state: RunState, event: RunEvent): RunState {
  const next: RunState = {
    ...state,
    eventCount: state.eventCount + 1,
    lastSeq: Math.max(state.lastSeq, event.seq),
    lastTime: event.time > 0 ? Math.max(state.lastTime ?? 0, event.time) : state.lastTime,
  };
  switch (event.type) {
    case "run_started": {
      const schema = text(event.schema);
      next.status = "running";
      next.schema = schema;
      next.schemaUnknown = schema !== undefined && schema !== SCHEMA;
      next.device = text(event.device);
      next.inputs = text(event.inputs);
      next.startedAt = event.time;
      if (isRecord(event.configuration)) next.configuration = event.configuration;
      return next;
    }
    case "stage_started":
      if (event.stage === null) return ignore(next);
      next.stages = withStage(state.stages, event.stage, event.time, (stage) => ({
        ...stage,
        status: "running",
        implicit: false,
        startedAt: event.time,
        seconds: undefined,
        fraction: 0,
        message: "",
      }));
      return next;
    case "progress": {
      if (event.stage === null) return ignore(next);
      const fraction = finite(event.fraction);
      const message = text(event.message);
      next.stages = withStage(state.stages, event.stage, event.time, (stage) => ({
        ...stage,
        status: stage.status === "pending" || stage.implicit ? "running" : stage.status,
        implicit: false,
        startedAt: stage.startedAt ?? event.time,
        fraction: fraction === undefined ? (stage.implicit ? 0 : stage.fraction) : Math.min(1, Math.max(0, fraction)),
        message: message ?? stage.message,
      }));
      return next;
    }
    case "stage_finished":
      if (event.stage === null) return ignore(next);
      next.stages = withStage(state.stages, event.stage, event.time, (stage) => ({
        ...stage,
        status: "done",
        seconds: finite(event.seconds) ?? (stage.startedAt === undefined ? undefined : event.time - stage.startedAt),
        fraction: 1,
      }));
      return next;
    case "metric": {
      const name = text(event.name);
      const value = event.value;
      if (name === undefined || !(typeof value === "number" || typeof value === "string" || typeof value === "boolean")) {
        return ignore(next);
      }
      const metric: Metric = { name, value, stage: event.stage, seq: event.seq };
      const at = state.metrics.findIndex((m) => m.name === name);
      next.metrics = at < 0 ? [...state.metrics, metric] : state.metrics.map((m, i) => (i === at ? metric : m));
      return next;
    }
    case "artifact":
      return artifact(state, next, event);
    case "error": {
      const message = text(event.message) ?? t("run.stageFailedNoMessage");
      next.errors = [...state.errors, { seq: event.seq, stage: event.stage, message }];
      if (event.stage !== null) {
        next.stages = withStage(state.stages, event.stage, event.time, (stage) => ({ ...stage, status: "failed" }));
      }
      return next;
    }
    case "run_finished": {
      const status = text(event.status);
      next.status = status === "complete" || status === "cancelled" ? status : "failed";
      next.seconds = finite(event.seconds) ?? (state.startedAt === undefined ? undefined : event.time - state.startedAt);
      const outcome = next.status;
      next.stages = state.stages.map((stage) => {
        if (stage.status === "running" || (stage.status === "failed" && outcome === "cancelled")) {
          return { ...stage, status: outcome === "complete" ? "done" : outcome };
        }
        return stage.status === "pending" ? { ...stage, status: "skipped" as const } : stage;
      });
      return next;
    }
    default:
      return ignore(next);
  }
}

function artifact(state: RunState, next: RunState, event: RunEvent): RunState {
  const kind = text(event.kind);
  const path = text(event.path);
  if (kind === undefined || path === undefined || path === "" || !safeRelativePath(path)) return ignore(next);
  const label = text(event.label) ?? path;
  // A stage that produced something without announcing itself (inputs given ready-made) has still happened.
  if (event.stage !== null) {
    next.stages = withStage(state.stages, event.stage, event.time, (stage) =>
      stage.status === "pending" ? { ...stage, status: "done", fraction: 1, implicit: true } : stage,
    );
  }
  // The neural surface and the 2D splats are further finished surfaces of the run, shown like the final mesh.
  if (kind === "preview_mesh" || kind === "final_mesh" || kind === "neural_surface" || kind === "splats") {
    const step: MeshStep = {
      seq: event.seq,
      kind: kind === "preview_mesh" ? "preview_mesh" : "final_mesh",
      path,
      label,
      stage: event.stage,
      triangles: finite(event.triangles),
      time: event.time,
    };
    next.meshes = bySeq(replaceByPath(state.meshes, step));
    return next;
  }
  if ((SHEET_KINDS as readonly string[]).includes(kind)) {
    const sheet: Sheet = {
      seq: event.seq,
      kind: kind as SheetKind,
      path,
      label,
      stage: event.stage,
      level: finite(event.level),
      time: event.time,
    };
    next.sheets = bySeq(replaceByPath(state.sheets, sheet));
    return next;
  }
  if (kind === "inspection_sheet") {
    const step = typeof event.step === "string" && event.step !== "" ? event.step : (path.split("/").pop() ?? path).replace(/\.png$/, "");
    const label = typeof event.label === "string" && event.label !== "" ? event.label : inspectionTitle(step);
    next.inspections = orderInspections([...state.inspections.filter((item) => item.path !== path), { seq: event.seq, path, step, label }]);
    return next;
  }
  if (kind === "textured_mesh") {
    next.downloads = bySeq(replaceByPath(state.downloads, { seq: event.seq, path, label, stage: event.stage }));
    return next;
  }
  if (kind === "report") {
    next.reports = bySeq(replaceByPath(state.reports, { seq: event.seq, path, label, stage: event.stage }));
    return next;
  }
  // preview_volume is internal by contract; anything else is a kind added later.
  return ignore(next);
}

function ignore(state: RunState): RunState {
  return { ...state, ignored: state.ignored + 1 };
}

/** Applies `change` to the named stage, appending the stage if this front end has never heard of it. */
function withStage(
  stages: readonly StageState[],
  name: string,
  time: number,
  change: (stage: StageState) => StageState,
): StageState[] {
  const known = stages.some((stage) => stage.name === name);
  const all = known ? stages : insertStage(stages, { name, status: "pending" as const, fraction: 0, message: "" });
  return all.map((stage) => (stage.name === name ? { ...change(stage), lastEventAt: time } : stage));
}

/**
 * Stages that only some runs have. `masks` and `cameras` belong in front (runs started from
 * plain photos); a stage this front end has never heard of goes to the end.
 */
const EARLY_STAGES = ["masks", "cameras"];

function insertStage(stages: readonly StageState[], stage: StageState): StageState[] {
  const rank = EARLY_STAGES.indexOf(stage.name);
  if (rank < 0) return [...stages, stage];
  // After the early stages that come before it, in front of everything else.
  const at = stages.filter((other) => EARLY_STAGES.indexOf(other.name) >= 0 && EARLY_STAGES.indexOf(other.name) < rank).length;
  return [...stages.slice(0, at), stage, ...stages.slice(at)];
}

function replaceByPath<T extends { path: string }>(items: readonly T[], item: T): T[] {
  return [...items.filter((other) => other.path !== item.path), item];
}

function bySeq<T extends { seq: number }>(items: T[]): T[] {
  return items.sort((a, b) => a.seq - b.seq);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Artifact paths are relative to the run. Anything absolute or climbing out of it is not shown. */
export function safeRelativePath(path: string): boolean {
  if (path.startsWith("/") || path.includes("\\") || /^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(path)) return false;
  return !path.split("/").some((part) => part === ".." || part === "");
}

// ---------------------------------------------------------------------------------------------
// Selectors (pure, used by the views)

export interface SheetGroup {
  stage: string;
  sheets: Sheet[];
}

/**
 * Sheets grouped by stage in stage order, in arrival order within a stage, except that depth
 * sheets are ordered by `level` (per-level sheets before the merged one at the same level).
 */
export function groupSheets(state: RunState): SheetGroup[] {
  const groups = new Map<string, Sheet[]>();
  for (const stage of state.stages) groups.set(stage.name, []);
  for (const sheet of state.sheets) {
    const key = sheet.stage ?? "run";
    const list = groups.get(key) ?? [];
    list.push(sheet);
    groups.set(key, list);
  }
  const result: SheetGroup[] = [];
  for (const [stage, sheets] of groups) {
    if (sheets.length === 0) continue;
    const depth = sheets
      .filter((sheet) => sheet.kind === "depth_sheet")
      .sort((a, b) => (a.level ?? Infinity) - (b.level ?? Infinity) || a.seq - b.seq);
    let next = 0;
    result.push({ stage, sheets: sheets.map((sheet) => (sheet.kind === "depth_sheet" ? depth[next++]! : sheet)) });
  }
  return result;
}

/** Seconds a stage has taken: reported once finished, otherwise measured against `now` (Unix seconds). */
export function stageElapsed(stage: StageState, now: number | undefined): number | undefined {
  if (stage.seconds !== undefined) return stage.seconds;
  if (stage.startedAt === undefined) return undefined;
  if (stage.status === "running" && now !== undefined) return Math.max(0, now - stage.startedAt);
  if (stage.lastEventAt !== undefined) return Math.max(0, stage.lastEventAt - stage.startedAt);
  return undefined;
}

/** Whole-run progress for a list row: finished stages plus the running stage's fraction, over the stages that ran or will. */
export function overallFraction(state: RunState): number {
  if (state.status === "complete") return 1;
  const counted = state.stages.filter((stage) => stage.status !== "skipped");
  if (counted.length === 0) return 0;
  const sum = counted.reduce((total, stage) => total + (stage.status === "done" ? 1 : stage.fraction), 0);
  return Math.min(1, sum / counted.length);
}

/** A binary STL is exactly 84 bytes plus 50 per triangle, so the size is known before downloading. */
export function stlBytes(triangles: number | undefined): number | undefined {
  return triangles === undefined ? undefined : 84 + 50 * triangles;
}
