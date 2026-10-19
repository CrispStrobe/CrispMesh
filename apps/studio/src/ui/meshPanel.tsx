import { useEffect, useRef, useState } from "preact/hooks";
import { formatBytes, formatCount } from "../core/format";
import { chooseMesh, LatestOnly } from "../core/meshChoice";
import { stlBytes, type MeshStep, type RunStatus } from "../core/reducer";
import { describe, isAbort } from "../sources/transport";
import type { RunSource } from "../sources/types";
import { MeshLoader, type LoadProgress } from "../viewer/meshLoader";
import type { UpAxis, Viewer } from "../viewer/viewer";
import { canDownload, canShareFiles, downloadFile, shareFile } from "./download";
import { Icon } from "./icons";
import { loadPrefs, savePrefs } from "./prefs";
import { engineText, language, t, tParts } from "../i18n";

/** "1 234 triangles", with the thin grouping of formatCount and the plural of the language. */
function triangles(count: number): string {
  return t("run.mesh.triangles", { count, n: formatCount(count) });
}

interface Props {
  source: RunSource;
  meshes: MeshStep[];
  runStatus: RunStatus;
  themeTick: number;
}

/** Meshes above this size are downloaded only when asked for (the final STL can be 50 MB). */
export const AUTO_LOAD_BYTES = 12 * 1024 * 1024;

const UP_CHOICES: UpAxis[] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];

type Loading = { path: string; progress: LoadProgress } | null;

export function MeshPanel({ source, meshes, runStatus, themeTick }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const viewer = useRef<Viewer | null>(null);
  const loader = useRef<MeshLoader | null>(null);
  const strip = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(false);
  const [unsupported, setUnsupported] = useState("");
  /** Path chosen by hand, or null to follow the newest surface. */
  const [chosen, setChosen] = useState<string | null>(null);
  const [approved, setApproved] = useState<ReadonlySet<string>>(new Set());
  const [shown, setShown] = useState<{ path: string; triangles: number } | null>(null);
  const [loading, setLoading] = useState<Loading>(null);
  const [error, setError] = useState<{ path: string; message: string } | null>(null);
  const [retry, setRetry] = useState(0);
  const [saving, setSaving] = useState(false);
  const latest = useRef(new LatestOnly());
  /** Sizes learnt by asking the server, by path; null when it could not say. */
  const [probed, setProbed] = useState<Record<string, number | null>>({});
  const [flat, setFlat] = useState(() => loadPrefs().flat);
  const [up, setUp] = useState<UpAxis>(() => {
    const stored = loadPrefs().up as UpAxis;
    return UP_CHOICES.includes(stored) ? stored : "+Y";
  });

  // The viewer (and three.js with it) is loaded on demand, so the rest of the page is up first.
  useEffect(() => {
    let cancelled = false;
    loader.current = new MeshLoader(source);
    void import("../viewer/viewer")
      .then((module) => {
        if (cancelled || canvas.current === null || host.current === null) return;
        if (!module.webglAvailable()) {
          setUnsupported(t("run.mesh.noWebgl"));
          return;
        }
        viewer.current = new module.Viewer(canvas.current, host.current);
        // For automated checks of the apps: whether the surface is actually drawn.
        (window as unknown as { __crispmeshProbe?: () => unknown }).__crispmeshProbe = () => viewer.current?.probe();
        setReady(true);
      })
      .catch((problem) => {
        if (!cancelled) setUnsupported(t("run.mesh.viewFailed", { error: describe(problem) }));
      });
    return () => {
      cancelled = true;
      viewer.current?.dispose();
      viewer.current = null;
      loader.current?.dispose();
      loader.current = null;
    };
  }, [source]);

  useEffect(() => {
    if (!ready) return;
    const style = getComputedStyle(document.documentElement);
    viewer.current?.setColors({
      background: style.getPropertyValue("--viewer-bg").trim() || "#e9ebee",
      surface: style.getPropertyValue("--viewer-surface").trim() || "#b9bec6",
    });
  }, [ready, themeTick]);

  useEffect(() => {
    viewer.current?.setFlatShading(flat);
  }, [ready, flat]);

  useEffect(() => {
    viewer.current?.setUp(up);
  }, [ready, up]);


  // The size of a binary STL follows from its triangle count. When an event does not carry
  // one, the file is asked for its size (HEAD) instead of being downloaded to find out.
  const sizeOf = (step: MeshStep): number | undefined => stlBytes(step.triangles) ?? probed[step.path] ?? undefined;
  const unsized = meshes.filter((step) => step.kind === "final_mesh" && step.triangles === undefined).map((step) => step.path);
  useEffect(() => {
    if (source.fileSize === undefined) return;
    const abort = new AbortController();
    for (const path of unsized) {
      if (path in probed) continue;
      source.fileSize(path, abort.signal).then(
        (size) => setProbed((known) => ({ ...known, [path]: size ?? null })),
        () => undefined,
      );
    }
    return () => abort.abort();
  }, [source, unsized.join("\n")]);

  const needsConsent = (step: MeshStep): boolean => {
    if (step.kind !== "final_mesh" || approved.has(step.path) || loader.current?.has(step.path)) return false;
    const bytes = sizeOf(step);
    return bytes === undefined || bytes > AUTO_LOAD_BYTES;
  };

  // What to put on screen: the selected step, unless it is a big final mesh nobody asked for
  // yet. Then the newest smaller surface stays up (when following) under a "load" button.
  // A function of the steps alone: replay speed or event timing cannot change it.
  const { selected, target, awaitingConsent, following } = chooseMesh(meshes, chosen, needsConsent);
  const targetPath = target?.path;

  useEffect(() => {
    if (!ready || viewer.current === null || loader.current === null) return;
    if (targetPath === undefined) {
      if (meshes.length === 0) {
        viewer.current.clearMesh();
        setShown(null);
      }
      return;
    }
    if (shown?.path === targetPath && viewer.current.hasMesh()) return;
    const abort = new AbortController();
    const ticket = latest.current.begin();
    setError(null);
    setLoading({ path: targetPath, progress: { phase: "download" } });
    loader.current
      .load(targetPath, abort.signal, (progress) => {
        if (!abort.signal.aborted) setLoading({ path: targetPath, progress });
      })
      .then((mesh) => {
        // Only the newest load may put its surface up: one that finishes late is dropped.
        if (abort.signal.aborted || !latest.current.isCurrent(ticket) || viewer.current === null) return;
        // The previous mesh's GPU buffers are released inside setMesh.
        viewer.current.setMesh(mesh);
        setShown({ path: targetPath, triangles: mesh.triangles - mesh.dropped });
        setLoading(null);
      })
      .catch((problem) => {
        if (abort.signal.aborted || isAbort(problem)) return;
        setLoading(null);
        setError({ path: targetPath, message: describe(problem) });
      });
    return () => abort.abort();
    // `shown` is deliberately not a dependency: it is the result of this effect.
  }, [ready, targetPath, meshes.length === 0, retry]);

  // Keep the selected step visible in the strip.
  useEffect(() => {
    const element = strip.current?.querySelector<HTMLElement>("[aria-pressed='true']");
    const container = strip.current;
    if (element === null || element === undefined || container === null) return;
    const left = element.offsetLeft - container.offsetLeft;
    if (left < container.scrollLeft || left + element.offsetWidth > container.scrollLeft + container.clientWidth) {
      container.scrollTo({ left: left - 16, behavior: "auto" });
    }
  }, [selected?.path, meshes.length]);

  const selectedIndex = selected === undefined ? -1 : meshes.indexOf(selected);
  const select = (index: number) => {
    const step = meshes[index];
    if (step !== undefined) setChosen(step.path);
  };
  const shownStep = shown === null ? undefined : meshes.find((step) => step.path === shown.path);
  const consentBytes = selected !== undefined ? sizeOf(selected) : undefined;
  const loadingStep = loading === null ? undefined : meshes.find((step) => step.path === loading.path);

  return (
    <section class="panel mesh-panel" aria-labelledby="mesh-heading">
      <div class="panel-head">
        <h2 id="mesh-heading">{t("run.mesh.heading")}</h2>
        <span class="sub" aria-live="polite">
          {shownStep !== undefined && shown !== null
            ? t("run.mesh.labelTriangles", { label: engineText(shownStep.label), triangles: triangles(shown.triangles) })
            : meshes.length === 0
              ? ""
              : t("run.mesh.noneLoaded")}
        </span>
      </div>

      <div class="viewer" ref={host}>
        <canvas
          ref={canvas}
          tabIndex={0}
          role="img"
          aria-label={
            shownStep !== undefined
              ? t("run.mesh.viewLabel", { label: engineText(shownStep.label) })
              : t("run.mesh.viewEmpty")
          }
          onKeyDown={(event) => {
            const view = viewer.current;
            if (view === null || event.ctrlKey || event.metaKey || event.altKey) return;
            const turn = Math.PI / 24;
            if (event.key === "ArrowLeft") view.orbit(-turn, 0);
            else if (event.key === "ArrowRight") view.orbit(turn, 0);
            else if (event.key === "ArrowUp") view.orbit(0, -turn);
            else if (event.key === "ArrowDown") view.orbit(0, turn);
            else if (event.key === "+" || event.key === "=") view.zoom(0.85);
            else if (event.key === "-") view.zoom(1 / 0.85);
            else if (event.key === "0" || event.key === "Home") view.resetView();
            else return;
            event.preventDefault();
          }}
        />
        {shownStep !== undefined && shown !== null && (
          // Always on the picture: which surface this is, so nobody mistakes a preview for the result.
          <p class={`viewer-label${shownStep.kind === "final_mesh" ? " final" : ""}`} aria-hidden="true">
            {t("run.mesh.labelTriangles", { label: engineText(shownStep.label), triangles: triangles(shown.triangles) })}
            {shownStep.kind !== "final_mesh" && selected?.kind === "final_mesh" && t("run.mesh.finalNotLoaded")}
          </p>
        )}
        {unsupported !== "" && <p class="viewer-message">{unsupported}</p>}
        {unsupported === "" && meshes.length === 0 && (
          <p class="viewer-message">
            {runStatus === "running" || runStatus === "waiting"
              ? t("run.mesh.firstSoon")
              : t("run.mesh.noSurface")}
          </p>
        )}
        {awaitingConsent && selected !== undefined && loading === null && (
          <div class="viewer-overlay">
            <p>
              {tParts("run.mesh.ready", { label: <strong>{engineText(selected.label)}</strong> })}
              {selected.triangles !== undefined && <> · {triangles(selected.triangles)}</>}
            </p>
            <button
              type="button"
              class="button primary"
              onClick={() => setApproved((current) => new Set(current).add(selected.path))}
            >
              {t("run.mesh.load", { label: language() === "en" ? engineText(selected.label).charAt(0).toLowerCase() + engineText(selected.label).slice(1) : engineText(selected.label) })}
              {consentBytes !== undefined && ` (${formatBytes(consentBytes)})`}
            </button>
          </div>
        )}
        {loading !== null && (
          <div class="viewer-status" role="status">
            {loading.progress.phase === "parse"
              ? t("run.mesh.preparing", { label: engineText(loadingStep?.label) ?? t("run.mesh.surface") })
              : loading.progress.received === undefined
                ? t("run.mesh.loadingPlain", { label: engineText(loadingStep?.label) ?? t("run.mesh.surface") })
                : loading.progress.total === undefined
                  ? t("run.mesh.loadingBytes", { label: engineText(loadingStep?.label) ?? t("run.mesh.surface"), received: formatBytes(loading.progress.received) })
                  : t("run.mesh.loadingBytesOf", {
                      label: engineText(loadingStep?.label) ?? t("run.mesh.surface"),
                      received: formatBytes(loading.progress.received),
                      total: formatBytes(loading.progress.total),
                    })}
          </div>
        )}
        {error !== null && (
          <div class="viewer-overlay" role="alert">
            <p>
              <strong>{t("run.mesh.loadFailed")}</strong> {error.message}
            </p>
            <button type="button" class="button" onClick={() => setRetry((count) => count + 1)}>
              {t("run.mesh.tryAgain")}
            </button>
          </div>
        )}
      </div>

      <div class="step-bar">
        <button
          type="button"
          class="button icon-only"
          onClick={() => select(selectedIndex - 1)}
          disabled={selectedIndex <= 0}
          aria-label={t("run.mesh.earlier")}
        >
          <Icon name="prev" />
        </button>
        <div class="step-strip" ref={strip} role="group" aria-label={t("run.mesh.strip")}>
          {meshes.map((step, index) => (
            <button
              key={step.path}
              type="button"
              class="step"
              aria-pressed={step === selected}
              onClick={() => select(index)}
            >
              <span class="step-index">{index + 1}</span>
              <span class="step-label">{engineText(step.label)}</span>
              {step.triangles !== undefined && <span class="step-meta">{triangles(step.triangles)}</span>}
            </button>
          ))}
          {meshes.length === 0 && <span class="sub step-empty">{t("run.mesh.noneYet")}</span>}
        </div>
        <button
          type="button"
          class="button icon-only"
          onClick={() => select(selectedIndex + 1)}
          disabled={selectedIndex < 0 || selectedIndex >= meshes.length - 1}
          aria-label={t("run.mesh.later")}
        >
          <Icon name="next" />
        </button>
      </div>

      <div class="viewer-tools">
        <label class="check">
          <input type="checkbox" checked={following} onChange={(event) => setChosen(event.currentTarget.checked ? null : (selected?.path ?? null))} />
          <span>{t("run.mesh.follow")}</span>
        </label>
        <div class="segmented" role="group" aria-label={t("run.mesh.shading")}>
          <button
            type="button"
            aria-pressed={!flat}
            onClick={() => {
              setFlat(false);
              savePrefs({ flat: false });
            }}
          >
            {t("run.mesh.smooth")}
          </button>
          <button
            type="button"
            aria-pressed={flat}
            onClick={() => {
              setFlat(true);
              savePrefs({ flat: true });
            }}
          >
            {t("run.mesh.flat")}
          </button>
        </div>
        <label class="inline-field">
          <span>{t("run.mesh.up")}</span>
          <select
            value={up}
            onChange={(event) => {
              const axis = event.currentTarget.value as UpAxis;
              setUp(axis);
              savePrefs({ up: axis });
            }}
            aria-label={t("run.mesh.upLabel")}
          >
            {UP_CHOICES.map((axis) => (
              <option key={axis} value={axis}>
                {axis.replace("-", "−")}
              </option>
            ))}
          </select>
        </label>
        {canDownload(source) && shownStep !== undefined && (
          <button
            type="button"
            class="button"
            disabled={saving}
            onClick={() => {
              setSaving(true);
              void downloadFile(source, shownStep.path)
                .catch((problem) => setError({ path: shownStep.path, message: describe(problem) }))
                .finally(() => setSaving(false));
            }}
          >
            {saving
              ? t("run.mesh.saving")
              : stlBytes(shownStep.triangles) !== undefined
                ? t("run.mesh.downloadStlSize", { size: formatBytes(stlBytes(shownStep.triangles)) })
                : t("run.mesh.downloadStl")}
          </button>
        )}
        {!canDownload(source) && shownStep !== undefined && canShareFiles() && (
          <button
            type="button"
            class="button"
            disabled={saving}
            onClick={() => {
              setSaving(true);
              void shareFile(source, shownStep.path)
                .catch((problem) => setError({ path: shownStep.path, message: describe(problem) }))
                .finally(() => setSaving(false));
            }}
          >
            {saving ? t("run.mesh.preparingShare") : t("run.mesh.shareStl")}
          </button>
        )}
        <button type="button" class="button" onClick={() => viewer.current?.resetView()} disabled={shown === null}>
          <Icon name="fit" size={16} /> {t("run.mesh.resetView")}
        </button>
      </div>
      <p class="help viewer-note">{t("run.mesh.note")}</p>
    </section>
  );
}
