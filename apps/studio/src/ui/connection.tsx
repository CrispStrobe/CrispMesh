import { useEffect, useState } from "preact/hooks";
import { HttpEngine, normaliseEngineUrl } from "../sources/httpEngine";
import { describe } from "../sources/transport";
import { navigate } from "./app";
import { Icon } from "./icons";
import type { Prefs } from "./prefs";
import type { EngineStatus } from "../shell/shell";
import type { BrowserMode, EngineMode } from "./app";
import { servedByEngine } from "./served";
import { ShareEngine } from "./shareEngine";
import { PairScanner } from "./pairScanner";

interface Props {
  prefs: Prefs;
  onChange(change: Partial<Prefs>): void;
  /** Present inside the desktop app, which runs an engine itself. */
  localEngine: { native: boolean; python: boolean; status: EngineStatus | null; mode: EngineMode } | null;
  /** Present where the engine in this page's own worker is offered. */
  browser: BrowserMode | null;
}

const LOCAL_STATE: Record<EngineStatus["state"], string> = {
  stopped: "is stopped",
  starting: "is starting",
  running: "is running",
  failed: "could not be started",
};

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
      engine = new HttpEngine(address, { token: chosenToken || undefined });
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
      setBundleError("Enter the address of a recording.");
      return;
    }
    try {
      new URL(value, document.baseURI);
    } catch {
      setBundleError("That is not a valid address.");
      return;
    }
    onChange({ bundleUrl: value });
    navigate(`#/replay?bundle=${encodeURIComponent(value)}`);
  };

  return (
    <section class="page narrow">
      <h1>Photos to a printable model</h1>
      {browser !== null && localEngine === null ? (
        <p class="lead">
          CrispMesh turns photos of an object on a turntable into a closed surface you can print (STL), and shows the
          surface getting better step by step. It computes right here, in this browser, on this device's graphics processor:
          nothing is uploaded and nothing needs to be installed.
        </p>
      ) : (
        <p class="lead">
          Studio shows a reconstruction as it happens: the photos, the masks, the depth found at each level and the surface
          getting better step by step.
        </p>
      )}

      <div class="cards">
        {localEngine?.native && (
          <section class="card wide" aria-labelledby="c-local">
            <h2 id="c-local">This computer</h2>
            <p>
              The reconstruction is built into this app and runs on this computer's graphics processor. It starts from
              cameras and masks; nothing else needs to be installed.
            </p>
            <div class="actions">
              <a class="button primary" href="#/engine" onClick={() => onChange({ engineChoice: "native" })}>
                {localEngine.mode === "native" ? "Open runs" : "Use the built-in engine"}
              </a>
              <a class="button" href="#/shell">
                Folders
              </a>
            </div>
            <ShareEngine />
          </section>
        )}
        {localEngine?.python && (
          <section class="card wide" aria-labelledby="c-python">
            <h2 id="c-python">External Python engine</h2>
            <p>
              The reference pipeline in Python, started by this app with an interpreter you installed. For runs that start
              from plain photos, for scoring against a scan, and for comparisons.
              {localEngine.mode === "python" &&
                ` It ${localEngine.status === null ? "is starting" : LOCAL_STATE[localEngine.status.state]}${
                  localEngine.status?.state === "failed" && localEngine.status.message !== null ? `: ${localEngine.status.message}` : "."
                }`}
            </p>
            <div class="actions">
              <a class="button" href="#/engine" onClick={() => onChange({ engineChoice: "python" })}>
                {localEngine.mode === "python" ? "Open runs" : "Use the Python engine"}
              </a>
              <a class="button" href="#/shell">
                Engine settings
              </a>
            </div>
          </section>
        )}
        {browser !== null && (
          <section class={`card wide${localEngine === null ? " featured" : ""}`} aria-labelledby="c-browser">
            <h2 id="c-browser">{localEngine === null ? "Reconstruct in this browser" : "This browser"}</h2>
            <p>
              Choose a folder with the cameras, the photos and their masks (an inputs folder, as{" "}
              <span class="mono">crispmesh-dense inputs --pack</span> writes it). It is read on this device and nothing is
              uploaded. The model can be downloaded as STL when the run is done; runs are kept until this page is closed.
            </p>
            {browser.built && browser.webgpu && (
              <p class="help" role="note">
                {globalThis.crossOriginIsolated
                  ? `This page can compute with up to ${Math.min(4, navigator.hardwareConcurrency || 1)} threads.`
                  : "This page computes single-threaded: it is not cross-origin isolated, so it cannot share memory between threads."}
              </p>
            )}
            {!browser.built ? (
              <p class="field-error">This build of Studio does not include the engine for browsers.</p>
            ) : !browser.webgpu ? (
              <p class="field-error">
                This browser has no WebGPU, which the engine computes with. Current Chrome and Edge have it; Safari and Firefox
                have it in their newest versions or behind a setting.
              </p>
            ) : (
              <div class="actions">
                <a class="button primary" href="#/engine" onClick={() => onChange({ engineChoice: "browser" })}>
                  {browser.inUse ? "Open runs" : "Start in this browser"}
                </a>
                <a class="button" href="#/replay">
                  <Icon name="play" /> See a recorded run first
                </a>
              </div>
            )}
          </section>
        )}
        {(localEngine?.native === true || (browser !== null && browser.built && browser.webgpu)) && (
          <section class="card" aria-labelledby="c-examples">
            <h2 id="c-examples">Example objects</h2>
            <p>
              Rendered household objects and photographed figures (about 72 photos each): downloaded on request from public datasets, then
              reconstructed {localEngine?.native === true ? "on this computer" : "in this browser"}.
            </p>
            <a class="button" href="#/examples" onClick={() => onChange({ engineChoice: localEngine?.native === true ? "native" : "browser" })}>
              Choose an object
            </a>
          </section>
        )}
        <section class="card" aria-labelledby="c-demo">
          <h2 id="c-demo">Demo recording</h2>
          <p>A recorded run on a synthetic sphere, included with this app. Needs no engine.</p>
          <a class="button primary" href="#/replay">
            <Icon name="play" /> Play the demo
          </a>
        </section>

        <section class="card tall" aria-labelledby="c-engine">
          <h2 id="c-engine">{localEngine !== null ? "Another engine" : "Engine"}</h2>
          <p>
            Connect to a running engine to watch its runs live and start new ones.
            {sameOrigin && " An engine is serving this page."}
          </p>
          <form onSubmit={(event) => void connect(event)} novalidate>
            <label class="field">
              <span class="field-label">Address</span>
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
                Access token <span class="optional">only if the engine asks for one</span>
              </span>
              <input
                type="password"
                autocomplete="off"
                spellcheck={false}
                value={token}
                onInput={(event) => setToken(event.currentTarget.value)}
              />
              <span class="help">Kept on this device only.</span>
            </label>
            {error !== "" && (
              <p class="field-error" id="engine-error" role="alert">
                {error}
              </p>
            )}
            <div class="actions">
              <button type="submit" class="button primary" disabled={busy}>
                {busy ? "Connecting..." : "Connect"}
              </button>
              <button type="button" class="button" disabled={busy} onClick={() => setScanning(true)}>
                Scan the computer's code
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
                  Forget this engine
                </button>
              )}
            </div>
          </form>
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
          <h2 id="c-bundle">Recorded run</h2>
          <p>Play back a replay bundle: a folder with an event log and the files it names, on any web server.</p>
          <form onSubmit={openBundle} novalidate>
            <label class="field">
              <span class="field-label">Bundle address</span>
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
                Open recording
              </button>
            </div>
          </form>
        </section>
      </div>
    </section>
  );
}
