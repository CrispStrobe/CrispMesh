/**
 * The native shell (Tauri), when Studio runs inside one. In a browser `getShell()` is
 * null and nothing here is used. The shell offers the built-in engine (see
 * sources/localEngine.ts) and, on desktop, can also start the external Python engine as a
 * child process and say where it is.
 */

import { invoke, isTauri } from "@tauri-apps/api/core";
import { t } from "../i18n";
import { decodeForegroundPreview } from "../core/foregroundPreview";

export interface ShellInfo {
  /** The reconstruction is built into the app (crates/dense). */
  native_engine: boolean;
  /** The app can start the external Python engine: desktop builds outside the sandbox. */
  can_run_engine: boolean;
  /** Native folder and file pickers exist. */
  can_pick_paths: boolean;
  /** The app runs in the macOS App Sandbox. */
  sandboxed: boolean;
  os: string;
  version: string;
  auto_device: string;
}

export type EngineState = "stopped" | "starting" | "running" | "failed";

export interface EngineStatus {
  state: EngineState;
  url: string | null;
  /** Per-start random token of the local engine. Kept in memory only. */
  token: string | null;
  message: string | null;
  /** The command line that was started, token blanked. */
  command: string | null;
  log: string[];
  device: string | null;
}

export interface ShellConfig {
  repo: string;
  python: string;
  torch_python: string;
  runs_dir: string;
  data_dir: string;
  device: string;
  alicevision: string;
  alicevision_library_path: string;
  colmap: string;
  sam_python: string;
  sam_source: string;
  sam_checkpoint: string;
}

export const CONFIG_FIELDS = [
  "repo",
  "python",
  "torch_python",
  "runs_dir",
  "data_dir",
  "device",
  "alicevision",
  "alicevision_library_path",
  "colmap",
  "sam_python",
  "sam_source",
  "sam_checkpoint",
] as const;
export const TOOL_FIELDS = ["alicevision", "alicevision_library_path", "colmap", "sam_python", "sam_source", "sam_checkpoint"] as const;
export type ToolField = (typeof TOOL_FIELDS)[number];
export type ToolName = "alicevision" | "colmap" | "sam";

export interface ToolCheck {
  ok: boolean;
  /** One line: the version, or what is wrong. */
  summary: string;
  detail: string;
}
export type ConfigField = (typeof CONFIG_FIELDS)[number];

export type ValueSource = "setting" | "environment" | "found" | "default";

export interface ShellSettings {
  saved: ShellConfig;
  resolved: Record<Exclude<ConfigField, ToolField>, { value: string; source: ValueSource }>;
  /** Where the external programs of the photos start are. */
  tools: Record<ToolField, { value: string; source: ValueSource }>;
  problem: string | null;
  file: string;
}

export interface Shell {
  info(): Promise<ShellInfo>;
  settings(): Promise<ShellSettings>;
  saveSettings(config: ShellConfig): Promise<ShellSettings>;
  engineStatus(): Promise<EngineStatus>;
  restartEngine(): Promise<void>;
  stopEngine(): Promise<void>;
  /** Asks an external program of the photos start about itself, with the saved settings. */
  checkTool(tool: ToolName): Promise<ToolCheck>;
  /** Calls any command of the shell; the built-in engine is reached through this. */
  bridge<T>(command: string, args?: Record<string, unknown> | Uint8Array, options?: { headers: Record<string, string> }): Promise<T>;
  /** Native picker; resolves to null when cancelled. */
  pickPath(kind: "folder" | "file", title: string, start?: string): Promise<string | null>;
}

const tauriShell: Shell = {
  info: () => invoke<ShellInfo>("shell_info"),
  settings: () => invoke<ShellSettings>("get_settings"),
  saveSettings: (config) => invoke<ShellSettings>("save_settings", { config }),
  engineStatus: () => invoke<EngineStatus>("engine_status"),
  restartEngine: () => invoke<void>("restart_engine"),
  stopEngine: () => invoke<void>("stop_engine"),
  checkTool: (tool) => invoke<ToolCheck>("check_tool", { tool }),
  bridge: (command, args, options) => invoke(command, args, options),
  pickPath: (kind, title, start) => invoke<string | null>("pick_path", { kind, title, start: start ?? null }),
};

export function getShell(): Shell | null {
  return isTauri() ? tauriShell : null;
}

/**
 * The horizontal field of view, in degrees across the frame's width, of the camera *Take photos* uses, for frames of
 * this size: from the app (AVFoundation on iOS). Null in a browser, without a camera, or when the system does not say.
 */
export async function cameraFieldOfView(width: number, height: number): Promise<number | null> {
  if (!isTauri()) return null;
  const degrees = await invoke<number | null>("camera_field_of_view", { width: Math.round(width), height: Math.round(height) }).catch(() => null);
  return typeof degrees === "number" && Number.isFinite(degrees) ? degrees : null;
}

/** How a setting's origin is worded next to its field. */
export function sourceText(source: ValueSource, field: ConfigField): string {
  if (source === "environment") {
    const variable = {
      repo: "CRISPMESH_REPO",
      python: "CRISPMESH_PYTHON",
      torch_python: "CRISPMESH_TORCH_PYTHON",
      runs_dir: "CRISPMESH_RUNS_DIR",
      data_dir: "CRISPMESH_DATA_DIR",
      alicevision: "CRISPMESH_ALICEVISION",
      alicevision_library_path: "CRISPMESH_ALICEVISION_LIBRARY_PATH",
      colmap: "CRISPMESH_COLMAP",
      sam_python: "CRISPMESH_SAM_PYTHON",
      sam_source: "CRISPMESH_SAM_SOURCE",
      sam_checkpoint: "CRISPMESH_SAM_CHECKPOINT",
    }[field as string];
    return variable !== undefined ? t("connection.shell.source.variable", { variable }) : t("connection.shell.source.environment");
  }
  if (source === "found") return t("connection.shell.source.found");
  if (source === "default") return field === "torch_python" ? t("connection.shell.source.sameAsAbove") : t("connection.shell.source.default");
  return t("connection.shell.source.setting");
}

/** Only fields that differ from what is saved need saving. */
export function sameConfig(a: ShellConfig, b: ShellConfig): boolean {
  return CONFIG_FIELDS.every((field) => (a[field] ?? "").trim() === (b[field] ?? "").trim());
}

/** Foreground previews use the device OS; nothing is sent to the reconstruction computer. */
export async function foregroundAvailable(): Promise<boolean> {
  return isTauri() && await invoke<boolean>("capture_foreground_available").catch(()=>false);
}
export async function foregroundPreview(file: File, focus: number, threshold: number, region?: import("../core/foregroundPreview").ImageRegion, strictRegion = false): Promise<import("../core/foregroundPreview").ForegroundPreview | null> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  // Follow the previous complete object before trying the full scene: a small figure must not
  // be replaced by its turntable or backdrop. A clipped detection is rejected by the native side.
  const attempts: { area: number; region?: import("../core/foregroundPreview").ImageRegion }[] =
    strictRegion && region ? [{area:1,region}] : focus === 1 ? [...(region ? [{area:1,region}] : []), {area:1}, {area:0.7}, {area:0.4}] : [{area:focus}];
  for (const attempt of attempts) {
    try {
      const headers: Record<string,string> = { "x-focus": String(attempt.area), "x-threshold": String(threshold) };
      if (attempt.region) headers["x-region"] = JSON.stringify(attempt.region);
      const buffer = await invoke<ArrayBuffer>("capture_foreground", bytes, { headers });
      return decodeForegroundPreview(buffer);
    } catch (error) {
      if (String(error) !== "foreground-not-found") throw error;
    }
  }
  return null;
}
