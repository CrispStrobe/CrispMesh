import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import {
  downloadExample,
  EXAMPLE_SOURCES,
  parseExampleManifest,
  runAttribution,
  type DownloadProgress,
  type ExampleManifest,
  type ExampleObject,
  type ExampleSource,
} from "../core/examples";
import { formatBytes } from "../core/format";
import type { BrowserEngine } from "../sources/browserEngine";
import { AppExampleStore, BrowserExampleStore } from "../sources/exampleStores";
import type { Bridge } from "../sources/localEngine";
import { describe } from "../sources/transport";
import type { Engine } from "../sources/types";
import { navigate } from "./app";
import { t, tParts } from "../i18n";

interface Props {
  engine: Engine;
  /** The shell's command bridge, when the engine is the one built into the app. */
  bridge: Bridge | null;
  /** Where the lists come from; the dataset repositories by default. */
  sources?: ExampleSource[];
}

type Row = { state: "unknown" | "absent" | "partial" | "ready"; have: number } | { state: "downloading"; progress: DownloadProgress } | { state: "failed"; message: string };

/**
 * Real photo sets that can be downloaded on request and reconstructed: nothing of them comes
 * with the app. Before anything is fetched the source, the license and the download size are
 * shown; the attribution is kept with every run made from them.
 */
export function Examples({ engine, bridge, sources = EXAMPLE_SOURCES }: Props) {
  return (
    <section class="page narrow examples">
      <a class="back" href="#/">
        {t("inputs.examples.back")}
      </a>
      <h1>{t("inputs.examples.title")}</h1>
      <p class="sub">{t(engine.kind === "browser" ? "inputs.examples.introPage" : "inputs.examples.introApp")}</p>
      {sources.map((source) => (
        <SourceSection key={source.id} engine={engine} bridge={bridge} source={source} />
      ))}
    </section>
  );
}

/** One dataset repository: its attribution, then its objects. */
function SourceSection({ engine, bridge, source }: { engine: Engine; bridge: Bridge | null; source: ExampleSource }) {
  const manifestUrl = source.manifest;
  const [manifest, setManifest] = useState<ExampleManifest | null>(null);
  const [problem, setProblem] = useState("");
  const [rows, setRows] = useState<Record<string, Row>>({});
  const [confirm, setConfirm] = useState<ExampleObject | null>(null);
  const abort = useRef<AbortController | null>(null);

  useEffect(() => {
    let alive = true;
    fetch(manifestUrl)
      .then((response) => (response.ok ? response.json() : Promise.reject(new Error(t("inputs.examples.listFailed", { status: response.status })))))
      .then((json) => alive && setManifest(parseExampleManifest(json, manifestUrl, source.purpose)))
      .catch((reason) => alive && setProblem(describe(reason)));
    return () => {
      alive = false;
      abort.current?.abort();
    };
  }, [manifestUrl]);

  const store = useMemo(() => {
    if (manifest === null) return null;
    if (engine.kind === "browser") return new BrowserExampleStore(manifest, engine as BrowserEngine);
    if (engine.kind === "local" && bridge !== null) return new AppExampleStore(bridge, engine);
    return null;
  }, [manifest, engine, bridge]);

  const refresh = async (object: ExampleObject) => {
    if (store === null) return;
    const have = (await store.have(object)).size;
    const total = object.files.length + 1;
    setRows((current) => ({ ...current, [object.id]: { state: have === 0 ? "absent" : have === total ? "ready" : "partial", have } }));
  };
  useEffect(() => {
    for (const object of manifest?.objects ?? []) void refresh(object);
  }, [store]);

  const run = async (object: ExampleObject) => {
    if (manifest === null || store === null) return;
    setConfirm(null);
    const controller = new AbortController();
    abort.current = controller;
    try {
      await downloadExample(manifest, object, store, {
        signal: controller.signal,
        onProgress: (progress) => setRows((current) => ({ ...current, [object.id]: { state: "downloading", progress } })),
      });
      await refresh(object);
      const id = await store.start(manifest, object, runAttribution(manifest, object));
      navigate(`#/engine/run/${encodeURIComponent(id)}`);
    } catch (reason) {
      if (controller.signal.aborted) await refresh(object);
      else setRows((current) => ({ ...current, [object.id]: { state: "failed", message: describe(reason) } }));
    }
  };

  const remove = async (object: ExampleObject) => {
    if (store === null) return;
    try {
      await store.remove(object);
    } catch (reason) {
      setRows((current) => ({ ...current, [object.id]: { state: "failed", message: describe(reason) } }));
      return;
    }
    await refresh(object);
  };

  return (
    <section class="example-source" aria-labelledby={`ex-${source.id}`}>
      <h2 id={`ex-${source.id}`}>{source.title}</h2>
      {problem !== "" && <p class="notice bad">{problem}</p>}
      {manifest === null && problem === "" && <p role="status">{t("inputs.examples.loadingList")}</p>}
      {manifest !== null && (
        <>
          <section class="card attribution-card" aria-label={t("inputs.examples.sourceLabel", { source: source.title })}>
            <h3>{t("inputs.examples.sourceHeading")}</h3>
            <p>{manifest.attribution.text}</p>
            {manifest.purpose !== "" && <p class="help">{source.shownPurpose ?? manifest.purpose}</p>}
            <p class="help">
              {manifest.attribution.source !== undefined && (
                <>
                  {tParts("inputs.examples.originalDataset", { source: <span class="mono wrap">{manifest.attribution.source}</span> })}{" "}
                </>
              )}
              {tParts("inputs.examples.downloadedFrom", { url: <span class="mono wrap">{manifest.baseUrl}</span> })}
            </p>
          </section>
          {store === null && <p class="notice">{t("inputs.examples.noStore")}</p>}
          <ul class="example-list">
            {manifest.objects.map((object) => {
              const row = rows[object.id] ?? { state: "unknown", have: 0 };
              const busy = Object.values(rows).some((other) => other.state === "downloading");
              return (
                <li key={object.id} class="example">
                  <div class="example-text">
                    <strong>{object.name}</strong>
                    <span class="help">
                      {t("inputs.examples.photos", { count: object.photos })} · {formatBytes(object.bytes)}
                      {row.state === "ready" && t("inputs.examples.downloaded")}
                      {row.state === "partial" && t("inputs.examples.partial", { have: row.have, total: object.files.length + 1 })}
                    </span>
                    {row.state === "downloading" && (
                      <span class="help" role="status">
                        {t("inputs.examples.downloading", {
                          done: row.progress.done,
                          total: row.progress.total,
                          bytes: formatBytes(row.progress.bytes),
                          totalBytes: formatBytes(row.progress.totalBytes),
                        })}
                        <progress max={row.progress.totalBytes} value={row.progress.bytes} />
                      </span>
                    )}
                    {row.state === "failed" && <span class="field-error">{row.message}</span>}
                  </div>
                  <div class="actions">
                    {row.state === "downloading" ? (
                      <button type="button" class="button" onClick={() => abort.current?.abort()}>
                        {t("inputs.examples.cancel")}
                      </button>
                    ) : (
                      <>
                        <button
                          type="button"
                          class="button primary"
                          disabled={store === null || busy}
                          onClick={() => (row.state === "ready" ? void run(object) : setConfirm(object))}
                        >
                          {row.state === "ready" ? t("inputs.examples.reconstruct") : t("inputs.examples.downloadAndReconstruct")}
                        </button>
                        {(row.state === "ready" || row.state === "partial") && (
                          <button type="button" class="button" disabled={busy} onClick={() => void remove(object)}>
                            {t("inputs.examples.deleteDownload")}
                          </button>
                        )}
                      </>
                    )}
                  </div>
                  {confirm?.id === object.id && (
                    <div class="example-confirm" role="dialog" aria-label={t("inputs.examples.confirmLabel", { name: object.name })}>
                      <p>
                        {tParts("inputs.examples.confirm", {
                          count: object.photos,
                          name: object.name,
                          size: formatBytes(object.bytes),
                          url: <span class="mono wrap">{manifest.baseUrl}</span>,
                        })}
                      </p>
                      <div class="actions">
                        <button type="button" class="button primary" onClick={() => void run(object)}>
                          {t("inputs.examples.downloadAndReconstruct")}
                        </button>
                        <button type="button" class="button" onClick={() => setConfirm(null)}>
                          {t("inputs.examples.notNow")}
                        </button>
                      </div>
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        </>
      )}
    </section>
  );
}
