/**
 * The native shell (Tauri), when Studio runs inside one. In a browser `getShell()` is
 * null and nothing here is used. The shell offers the built-in engine (see
 * sources/localEngine.ts) and, on desktop, can also start the external Python engine as a
 * child process and say where it is.
 */

import { invoke, isTauri } from "@tauri-apps/api/core";

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
    return variable !== undefined ? `from ${variable}` : "from the environment";
  }
  if (source === "found") return "found automatically";
  if (source === "default") return field === "torch_python" ? "same as the interpreter above" : "default";
  return "set here";
}

/** Only fields that differ from what is saved need saving. */
export function sameConfig(a: ShellConfig, b: ShellConfig): boolean {
  return CONFIG_FIELDS.every((field) => (a[field] ?? "").trim() === (b[field] ?? "").trim());
}
