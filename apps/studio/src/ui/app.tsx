import { useEffect, useMemo, useState } from "preact/hooks";
import { getShell, type ShellInfo } from "../shell/shell";
import type { WorkerLike } from "../browser/protocol";
import { BrowserEngine } from "../sources/browserEngine";
import { LocalEngine } from "../sources/localEngine";
import { HttpEngine } from "../sources/httpEngine";
import { ReplaySource } from "../sources/replaySource";
import type { Engine, RunSource } from "../sources/types";
import { Connection } from "./connection";
import { Icon } from "./icons";
import { NewRun } from "./newRun";
import { applyTheme, loadPrefs, savePrefs, speedFromPref, type ThemeChoice } from "./prefs";
import { Examples } from "./examples";
import { Notices } from "./notices";
import { Runs } from "./runs";
import { RunView } from "./runView";
import { LocalEngineGate, ShellSettingsScreen, useLocalEngine } from "./shellScreens";

/** Hash routes, so the app works from any base path and on static hosts without rewrites. */
export type Route =
  | { screen: "connection" }
  | { screen: "replay"; bundle: string | null }
  | { screen: "runs" }
  | { screen: "new" }
  | { screen: "run"; id: string }
  | { screen: "shell" }
  | { screen: "notices" }
  | { screen: "examples" };

export function parseRoute(hash: string): Route {
  const [path = "", query = ""] = hash.replace(/^#\/?/, "").split("?");
  const parts = path.split("/").filter((part) => part !== "");
  if (parts[0] === "replay") return { screen: "replay", bundle: new URLSearchParams(query).get("bundle") };
  if (parts[0] === "shell") return { screen: "shell" };
  if (parts[0] === "notices") return { screen: "notices" };
  if (parts[0] === "examples") return { screen: "examples" };
  if (parts[0] === "engine") {
    if (parts[1] === "new") return { screen: "new" };
    if (parts[1] === "run" && parts[2] !== undefined) return { screen: "run", id: decodeURIComponent(parts[2]) };
    return { screen: "runs" };
  }
  return { screen: "connection" };
}

/** `native`: built into the app. `python`: the external Python engine the app starts. `remote`: an engine by address. */
export type EngineMode = "native" | "python" | "remote" | "browser";

declare const __BROWSER_ENGINE__: boolean;
/** Whether this build carries the engine for "This browser" (the WebAssembly package next to the app). */
const BROWSER_ENGINE_BUILT = typeof __BROWSER_ENGINE__ !== "undefined" && __BROWSER_ENGINE__;

export interface BrowserMode {
  /** The WebAssembly package is part of this build. */
  built: boolean;
  /** The browser has WebGPU. */
  webgpu: boolean;
  inUse: boolean;
}

export function navigate(hash: string): void {
  location.hash = hash;
}

const THEME_NEXT: Record<ThemeChoice, ThemeChoice> = { system: "light", light: "dark", dark: "system" };
const THEME_LABEL: Record<ThemeChoice, string> = { system: "Auto", light: "Light", dark: "Dark" };
const THEME_ICON = { system: "auto", light: "sun", dark: "moon" } as const;

/** The demo recording ships next to index.html. */
const DEMO_BUNDLE = "demo/";

/** License line, and the way to the licenses of everything in the app (shown inside the app). */
function Footer() {
  return (
    <footer class="footer">
      <span>CrispMesh is free software under AGPL-3.0-only.</span>
      <a href="#/notices">Licenses and third-party notices</a>
    </footer>
  );
}

export function App() {
  const [route, setRoute] = useState<Route>(() => parseRoute(location.hash));
  const [prefs, setPrefs] = useState(loadPrefs);
  const [themeTick, setThemeTick] = useState(0);
  // Inside the desktop app the shell runs an engine of its own. In a browser `shell` is null.
  const shell = useMemo(getShell, []);
  const [shellInfo, setShellInfo] = useState<ShellInfo | null>(null);
  useEffect(() => {
    shell?.info().then(setShellInfo, () => undefined);
  }, [shell]);
  const hasNative = shell !== null && shellInfo?.native_engine === true;
  const hasPython = shell !== null && shellInfo?.can_run_engine === true;
  // Which engine is in use: the one built into the app unless the user chose otherwise.
  // The engine in this page's own worker. Made once: its runs live in it.
  const browserEngine = useMemo(
    () =>
      BROWSER_ENGINE_BUILT && typeof Worker !== "undefined"
        ? new BrowserEngine({
            module: new URL("engine/crisp3ds-dense.js", document.baseURI).href,
            createWorker: () => new Worker(new URL("../browser/engine.worker.ts", import.meta.url), { type: "module" }) as unknown as WorkerLike,
          })
        : null,
    [],
  );
  const webgpu = typeof navigator !== "undefined" && "gpu" in navigator;
  // Inside the app the built-in engine does this job; there the browser engine is a hidden diagnostic.
  const offerBrowser = shell === null || prefs.diagnostics;
  const mode: EngineMode =
    prefs.engineChoice === "browser" && browserEngine !== null && offerBrowser
      ? "browser"
      : prefs.engineChoice === "remote" || prefs.engineChoice === "browser"
      ? "remote"
      : prefs.engineChoice === "python" && hasPython
        ? "python"
        : hasNative
          ? "native"
          : hasPython
            ? "python"
            : "remote";
  const local = useLocalEngine(shell, mode === "python");
  const hasLocal = hasNative || hasPython;
  const useLocal = mode === "native" || mode === "python";

  useEffect(() => {
    const onHash = () => setRoute(parseRoute(location.hash));
    addEventListener("hashchange", onHash);
    return () => removeEventListener("hashchange", onHash);
  }, []);

  // The desktop app opens on its own engine's runs instead of the connection form.
  useEffect(() => {
    if (useLocal && (location.hash === "" || location.hash === "#/" || location.hash === "#")) navigate("#/engine");
  }, [hasLocal]);

  // `#/?diagnostics=1` (or `=0`) switches diagnostic features on or off, and is remembered.
  useEffect(() => {
    const wanted = /[?&]diagnostics=([01])/.exec(location.hash)?.[1];
    if (wanted !== undefined && (wanted === "1") !== prefs.diagnostics) setPrefs(savePrefs({ diagnostics: wanted === "1" }));
  }, [route]);

  useEffect(() => {
    applyTheme(prefs.theme);
    setThemeTick((tick) => tick + 1);
    const media = matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setThemeTick((tick) => tick + 1);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, [prefs.theme]);

  const localUrl = local.status?.state === "running" ? local.status.url : null;
  const localToken = local.status?.state === "running" ? local.status.token : null;
  const engine = useMemo<Engine | null>(() => {
    try {
      if (mode === "browser") return browserEngine;
      if (mode === "native" && shell !== null) return new LocalEngine(shell.bridge, { label: "this computer" });
      if (mode === "python") {
        return localUrl !== null ? new HttpEngine(localUrl, { token: localToken ?? undefined, label: "this computer" }) : null;
      }
      return prefs.engineUrl !== "" ? new HttpEngine(prefs.engineUrl, { token: prefs.engineToken || undefined }) : null;
    } catch {
      return null;
    }
  }, [mode, shell, browserEngine, localUrl, localToken, prefs.engineUrl, prefs.engineToken]);

  const runSource = useMemo<RunSource | null>(() => {
    if (route.screen === "replay") {
      const bundle = route.bundle ?? DEMO_BUNDLE;
      return new ReplaySource(new URL(bundle, document.baseURI).href, { speed: speedFromPref(loadPrefs().replaySpeed) });
    }
    if (route.screen === "run" && engine !== null) return engine.openRun(route.id);
    return null;
    // `route` is a fresh object per navigation; key on what identifies the run.
  }, [route.screen, route.screen === "replay" ? route.bundle : route.screen === "run" ? route.id : "", engine]);

  const update = (change: Partial<typeof prefs>) => setPrefs(savePrefs(change));
  const needsEngine = route.screen === "runs" || route.screen === "new" || route.screen === "run" || route.screen === "examples";
  const showRuns = useLocal || mode === "browser" || prefs.engineUrl !== "";

  let title = "Studio";
  if (route.screen === "replay") title = route.bundle === null ? "Demo recording" : "Recording";
  else if (route.screen === "runs") title = "Runs";
  else if (route.screen === "new") title = "New run";
  else if (route.screen === "run") title = route.id;
  else if (route.screen === "shell") title = "Engine settings";
  else if (route.screen === "notices") title = "Licenses";
  else if (route.screen === "examples") title = "Example objects";
  useEffect(() => {
    document.title = title === "Studio" ? "CrispMesh" : `${title} - CrispMesh`;
  }, [title]);

  return (
    <div class="app">
      <a class="skip-link" href="#main">
        Skip to content
      </a>
      <header class="topbar">
        <a class="brand" href="#/" aria-label="CrispMesh, connection">
          <Icon name="cube" size={20} />
          <span>
            Crisp<strong>Mesh</strong>
          </span>
        </a>
        <nav class="topnav" aria-label="Main">
          <a href="#/" aria-current={route.screen === "connection" ? "page" : undefined}>
            Connection
          </a>
          <a href="#/replay" aria-current={route.screen === "replay" && route.bundle === null ? "page" : undefined}>
            Demo
          </a>
          {showRuns && (
            <a href="#/engine" aria-current={needsEngine ? "page" : undefined}>
              Runs
            </a>
          )}
        </nav>
        <button
          type="button"
          class="button quiet theme-toggle"
          onClick={() => update({ theme: THEME_NEXT[prefs.theme] })}
          aria-label={`Theme: ${THEME_LABEL[prefs.theme]}. Switch to ${THEME_LABEL[THEME_NEXT[prefs.theme]]}.`}
          title="Switch theme"
        >
          <Icon name={THEME_ICON[prefs.theme]} />
          <span class="theme-label">{THEME_LABEL[prefs.theme]}</span>
        </button>
      </header>
      <main id="main" tabIndex={-1}>
        {route.screen === "connection" && (
          <Connection prefs={prefs} onChange={update} localEngine={hasLocal ? { native: hasNative, python: hasPython, status: local.status, mode } : null}
            browser={offerBrowser ? { built: browserEngine !== null, webgpu, inUse: mode === "browser" } : null} />
        )}
        {route.screen === "notices" && <Notices />}
        {route.screen === "examples" && engine !== null && <Examples engine={engine} bridge={mode === "native" && shell !== null ? shell.bridge : null} />}
        {route.screen === "shell" &&
          (shell !== null && hasLocal ? (
            <ShellSettingsScreen shell={shell} info={shellInfo} status={local.status} mode={mode} onChange={local.refresh} />
          ) : (
            <section class="page narrow">
              <div class="notice" role="note">
                <p>These settings exist only in the desktop app, which runs an engine itself.</p>
                <a class="button" href="#/">
                  Connection
                </a>
              </div>
            </section>
          ))}
        {needsEngine && engine === null && mode === "python" && shell !== null && (
          <LocalEngineGate status={local.status} shell={shell} onChange={local.refresh} />
        )}
        {needsEngine && engine === null && mode !== "python" && (
          <section class="page narrow">
            <div class="notice bad" role="alert">
              <p>No engine is connected.</p>
              <a class="button primary" href="#/">
                Choose a connection
              </a>
            </div>
          </section>
        )}
        {route.screen === "runs" && engine !== null && <Runs engine={engine} settingsHref={useLocal ? "#/shell" : undefined} />}
        {route.screen === "new" && engine !== null && <NewRun engine={engine} prefs={prefs} onChange={update} />}
        {runSource !== null && (route.screen === "replay" || route.screen === "run") && (
          <RunView
            key={runSource.kind + runSource.title}
            source={runSource}
            themeTick={themeTick}
            backHref={route.screen === "run" ? "#/engine" : "#/"}
            heading={route.screen === "run" ? route.id : route.bundle === null ? "Demo: synthetic sphere" : "Recorded run"}
          />
        )}
      </main>
      <Footer />
    </div>
  );
}
