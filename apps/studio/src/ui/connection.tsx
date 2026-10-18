import { useEffect, useState } from "preact/hooks";
import { HttpEngine, normaliseEngineUrl } from "../sources/httpEngine";
import { describe } from "../sources/transport";
import { navigate } from "./app";
import { Icon } from "./icons";
import type { Prefs } from "./prefs";
import { getShell, type EngineStatus } from "../shell/shell";
import { remoteFetcher } from "../shell/remoteFetch";
import type { BrowserMode, EngineMode } from "./app";
import { servedByEngine } from "./served";
import { ShareEngine } from "./shareEngine";
import { PairScanner } from "./pairScanner";
import { FindComputers } from "./findComputers";
import { t, tParts, type MessageKey } from "../i18n";

interface Props {
  prefs: Prefs;
  onChange(change: Partial<Prefs>): void;
  /** Present inside the desktop app, which runs an engine itself. */
  localEngine: { native: boolean; python: boolean; status: EngineStatus | null; mode: EngineMode } | null;
  /** Present where the engine in this page's own worker is offered. */
  browser: BrowserMode | null;
}

const LOCAL_STATE: Record<EngineStatus["state"], MessageKey> = {
  stopped: "connection.python.state.stopped",
  starting: "connection.python.state.starting",
  running: "connection.python.state.running",
  failed: "connection.python.state.failed",
};

/** "It is running." and so on, for the Python engine's card. */
function pythonState(status: EngineStatus | null): string {
  if (status === null) return t("connection.python.state.starting");
  if (status.state === "failed" && status.message !== null) return t("connection.python.state.failedWith", { message: status.message });
  return t(LOCAL_STATE[status.state]);
}

export function Connection({ prefs, onChange, localEngine, browser }: Props) {
  const [bundle, setBundle] = useState(prefs.bundleUrl);
  const [url, setUrl] = useState(prefs.engineUrl);
  const [token, setToken] = useState(prefs.engineToken);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [bundleError, setBundleError] = useState("");
  const [sameOrigin, setSameOrigin] = useState(false);
  const [scanning, setScanning] = useState(false);

  // If this page is being served by an engine (`engine_server.py --static`), offer it.
  useEffect(() => {
    if (localEngine !== null || !servedByEngine(location)) return;
    const abort = new AbortController();
    new HttpEngine(location.origin)
      .health(abort.signal)
      .then(() => {
        setSameOrigin(true);
        setUrl((current) => (current === "" ? location.origin : current));
      })
      .catch(() => undefined);
    return () => abort.abort();
  }, []);

  const connect = async (event: Event | null, given?: { url: string; token: string }) => {
    event?.preventDefault();
    setError("");
    let engine: HttpEngine;
    let address: string;
    const chosenToken = given?.token ?? token;
    try {
      address = normaliseEngineUrl(given?.url ?? url);
      const shell = getShell();
      engine = new HttpEngine(address, { token: chosenToken || undefined, ...(shell !== null ? { fetcher: remoteFetcher(shell) } : {}) });
    } catch (problem) {
      setError(describe(problem));
      return;
    }
    setBusy(true);
    try {
      await engine.health();
      onChange({ engineUrl: address, engineToken: chosenToken, engineChoice: "remote" });
      navigate("#/engine");
    } catch (problem) {
      setError(describe(problem));
    } finally {
      setBusy(false);
    }
  };

  const openBundle = (event: Event) => {
    event.preventDefault();
    const value = bundle.trim();
    if (value === "") {
      setBundleError(t("connection.bundle.empty"));
      return;
    }
    try {
      new URL(value, document.baseURI);
    } catch {
      setBundleError(t("connection.bundle.invalid"));
      return;
    }
    onChange({ bundleUrl: value });
    navigate(`#/replay?bundle=${encodeURIComponent(value)}`);
  };

  return (
    <section class="page narrow">
      <h1>{t("connection.title")}</h1>
      {browser !== null && localEngine === null ? (
        <p class="lead">{t("connection.leadBrowser")}</p>
      ) : (
        <p class="lead">{t("connection.leadStudio")}</p>
      )}

      <div class="cards">
        {localEngine?.native && (
          <section class="card wide" aria-labelledby="c-local">
            <h2 id="c-local">{t("connection.local.title")}</h2>
            <p>{t("connection.local.text")}</p>
            <div class="actions">
              <a class="button primary" href="#/engine" onClick={() => onChange({ engineChoice: "native" })}>
                {localEngine.mode === "native" ? t("connection.openRuns") : t("connection.local.use")}
              </a>
              <a class="button" href="#/shell">
                {t("connection.local.folders")}
              </a>
            </div>
            <ShareEngine />
          </section>
        )}
        {localEngine?.python && (
          <section class="card wide" aria-labelledby="c-python">
            <h2 id="c-python">{t("connection.python.title")}</h2>
            <p>
              {t("connection.python.text")}
              {localEngine.mode === "python" && ` ${pythonState(localEngine.status)}`}
            </p>
            <div class="actions">
              <a class="button" href="#/engine" onClick={() => onChange({ engineChoice: "python" })}>
                {localEngine.mode === "python" ? t("connection.openRuns") : t("connection.python.use")}
              </a>
              <a class="button" href="#/shell">
                {t("connection.engineSettings")}
              </a>
            </div>
          </section>
        )}
        {browser !== null && (
          <section class={`card wide${localEngine === null ? " featured" : ""}`} aria-labelledby="c-browser">
            <h2 id="c-browser">{localEngine === null ? t("connection.browser.titleFeatured") : t("connection.browser.title")}</h2>
            <p>{tParts("connection.browser.text", { command: <span class="mono">crispmesh-dense inputs --pack</span> })}</p>
            {browser.built && browser.webgpu && (
              <p class="help" role="note">
                {globalThis.crossOriginIsolated
                  ? t("connection.browser.threads", { count: Math.min(4, navigator.hardwareConcurrency || 1) })
                  : t("connection.browser.singleThread")}
              </p>
            )}
            {!browser.built ? (
              <p class="field-error">{t("connection.browser.notBuilt")}</p>
            ) : !browser.webgpu ? (
              <p class="field-error">{t("connection.browser.noWebgpu")}</p>
            ) : (
              <div class="actions">
                <a class="button primary" href="#/engine" onClick={() => onChange({ engineChoice: "browser" })}>
                  {browser.inUse ? t("connection.openRuns") : t("connection.browser.start")}
                </a>
                <a class="button" href="#/replay">
                  <Icon name="play" /> {t("connection.browser.recordedFirst")}
                </a>
              </div>
            )}
          </section>
        )}
        {(localEngine?.native === true || (browser !== null && browser.built && browser.webgpu)) && (
          <section class="card" aria-labelledby="c-examples">
            <h2 id="c-examples">{t("connection.examples.title")}</h2>
            <p>{t(localEngine?.native === true ? "connection.examples.textComputer" : "connection.examples.textBrowser")}</p>
            <a class="button" href="#/examples" onClick={() => onChange({ engineChoice: localEngine?.native === true ? "native" : "browser" })}>
              {t("connection.examples.choose")}
            </a>
          </section>
        )}
        <section class="card" aria-labelledby="c-demo">
          <h2 id="c-demo">{t("connection.demo.title")}</h2>
          <p>{t("connection.demo.text")}</p>
          <a class="button primary" href="#/replay">
            <Icon name="play" /> {t("connection.demo.play")}
          </a>
        </section>

        <section class="card tall" aria-labelledby="c-engine">
          <h2 id="c-engine">{localEngine !== null ? t("connection.remote.titleOther") : t("connection.remote.title")}</h2>
          <p>
            {t("connection.remote.text")}
            {sameOrigin && ` ${t("connection.remote.sameOrigin")}`}
          </p>
          <form onSubmit={(event) => void connect(event)} novalidate>
            <label class="field">
              <span class="field-label">{t("connection.remote.address")}</span>
              <input
                type="url"
                inputMode="url"
                autocomplete="url"
                spellcheck={false}
                placeholder="http://127.0.0.1:8765"
                value={url}
                onInput={(event) => setUrl(event.currentTarget.value)}
                aria-describedby={error !== "" ? "engine-error" : undefined}
                aria-invalid={error !== ""}
              />
            </label>
            <label class="field">
              <span class="field-label">
                {t("connection.remote.token")} <span class="optional">{t("connection.remote.tokenOptional")}</span>
              </span>
              <input
                type="password"
                autocomplete="off"
                spellcheck={false}
                value={token}
                onInput={(event) => setToken(event.currentTarget.value)}
              />
              <span class="help">{t("connection.remote.tokenKept")}</span>
            </label>
            {error !== "" && (
              <p class="field-error" id="engine-error" role="alert">
                {error}
              </p>
            )}
            <div class="actions">
              <button type="submit" class="button primary" disabled={busy}>
                {busy ? t("connection.remote.connecting") : t("connection.remote.connect")}
              </button>
              <button type="button" class="button" disabled={busy} onClick={() => setScanning(true)}>
                {t("connection.remote.scan")}
              </button>
              {prefs.engineUrl !== "" && (
                <button
                  type="button"
                  class="button"
                  onClick={() => {
                    onChange({ engineUrl: "", engineToken: "", engineChoice: "native" });
                    setUrl("");
                    setToken("");
                  }}
                >
                  {t("connection.remote.forget")}
                </button>
              )}
            </div>
          </form>
          {getShell() !== null && (
            <FindComputers
              shell={getShell()!}
              disabled={busy}
              onPaired={(pairing) => {
                setUrl(pairing.url);
                setToken(pairing.token);
                void connect(null, pairing);
              }}
            />
          )}
          {scanning && (
            <PairScanner
              onCancel={() => setScanning(false)}
              onFound={(pairing) => {
                setScanning(false);
                setUrl(pairing.url);
                setToken(pairing.token);
                void connect(null, pairing);
              }}
            />
          )}
        </section>

        <section class="card" aria-labelledby="c-bundle">
          <h2 id="c-bundle">{t("connection.bundle.title")}</h2>
          <p>{t("connection.bundle.text")}</p>
          <form onSubmit={openBundle} novalidate>
            <label class="field">
              <span class="field-label">{t("connection.bundle.address")}</span>
              <input
                type="url"
                inputMode="url"
                spellcheck={false}
                placeholder="https://example.org/runs/dragon/"
                value={bundle}
                onInput={(event) => {
                  setBundle(event.currentTarget.value);
                  setBundleError("");
                }}
                aria-invalid={bundleError !== ""}
              />
            </label>
            {bundleError !== "" && (
              <p class="field-error" role="alert">
                {bundleError}
              </p>
            )}
            <div class="actions">
              <button type="submit" class="button">
                {t("connection.bundle.open")}
              </button>
            </div>
          </form>
        </section>
      </div>
    </section>
  );
}
