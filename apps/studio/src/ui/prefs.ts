/** Remembered choices (localStorage). Everything is optional and failure to store is harmless. */

import type { ReplaySpeed } from "../core/replay";

export type ThemeChoice = "system" | "light" | "dark";

export interface Prefs {
  theme: ThemeChoice;
  engineUrl: string;
  /** Bearer token of the engine. Stored on this device only; never logged or put in a URL. */
  engineToken: string;
  bundleUrl: string;
  /** 0 stands for "instant" (Infinity does not survive JSON). */
  replaySpeed: number;
  up: string;
  flat: boolean;
  inputs: string;
  device: string;
  /** Inside the app: "native" (built in), "python" (external Python engine the app starts) or "remote" (`engineUrl`). */
  engineChoice: string;
  /** Shows diagnostic features (inside the app: the engine in the web view's own worker). */
  diagnostics: boolean;
}

const KEY = "crispmesh.studio.v1";

const DEFAULTS: Prefs = {
  theme: "system",
  engineUrl: "",
  engineToken: "",
  bundleUrl: "",
  replaySpeed: 4,
  up: "+Y",
  flat: false,
  inputs: "",
  device: "",
  engineChoice: "native",
  diagnostics: false,
};

let cache: Prefs | null = null;

export function loadPrefs(): Prefs {
  if (cache !== null) return cache;
  let stored: Partial<Prefs> = {};
  try {
    const raw = localStorage.getItem(KEY);
    if (raw !== null) {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed === "object" && parsed !== null) stored = parsed as Partial<Prefs>;
    }
  } catch {
    // private mode, blocked storage, or damaged JSON: start from defaults
  }
  const merged: Prefs = { ...DEFAULTS };
  for (const key of Object.keys(DEFAULTS) as (keyof Prefs)[]) {
    const value = stored[key];
    if (typeof value === typeof DEFAULTS[key]) (merged as unknown as Record<string, unknown>)[key] = value;
  }
  cache = merged;
  return merged;
}

export function savePrefs(change: Partial<Prefs>): Prefs {
  const next = { ...loadPrefs(), ...change };
  cache = next;
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // not remembered; nothing else to do
  }
  return next;
}

export function speedFromPref(value: number): ReplaySpeed {
  return value === 1 || value === 4 || value === 16 ? value : value === 0 ? Infinity : 4;
}

export function speedToPref(speed: ReplaySpeed): number {
  return speed === Infinity ? 0 : speed;
}

/** Applies the theme choice to the document; "system" removes the override so CSS follows the OS. */
export function applyTheme(choice: ThemeChoice): void {
  const root = document.documentElement;
  if (choice === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", choice);
}
