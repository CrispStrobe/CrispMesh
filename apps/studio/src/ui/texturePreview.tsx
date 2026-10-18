import { useEffect, useRef, useState } from "preact/hooks";
import { checkPhotoGlb } from "../core/photoGlb";
import type { RunSource } from "../sources/types";
import { describe, isAbort } from "../sources/transport";
import type { Viewer, UpAxis } from "../viewer/viewer";
import { t } from "../i18n";

/** Download on request; close/unmount releases the texture, geometry and WebGL context. */
export function TexturePreview({ source, path, themeTick }: { source: RunSource; path: string; themeTick: number }) {
  const [open, setOpen] = useState(false);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState("");
  const [photo, setPhoto] = useState(true);
  const [up, setUp] = useState<UpAxis>("+Y");
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const viewer = useRef<Viewer | null>(null);
  useEffect(() => {
    if (!open) return;
    const abort = new AbortController();
    setReady(false); setError("");
    void (async () => {
      const [{ Viewer, webglAvailable }, { GLTFLoader }, { Mesh, MeshStandardMaterial }] = await Promise.all([
        import("../viewer/viewer"), import("three/addons/loaders/GLTFLoader.js"), import("three"),
      ]);
      abort.signal.throwIfAborted();
      if (!webglAvailable()) throw Error(t("run.texture.no3d"));
      const bytes = await source.fetchBytes(path, { signal: abort.signal });
      abort.signal.throwIfAborted();
      checkPhotoGlb(bytes);
      const model = await new GLTFLoader().parseAsync(bytes, "");
      // CrispMesh exports one static mesh, with one embedded photo atlas.
      const meshes: InstanceType<typeof Mesh>[] = [];
      model.scene.traverse((object) => { if (object instanceof Mesh) meshes.push(object); });
      const dispose = () => { for (const mesh of meshes) {
        mesh.geometry.dispose();
        for (const material of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) {
          const map = (material as InstanceType<typeof MeshStandardMaterial>).map;
          map?.dispose(); (map?.image as { close?: () => void } | undefined)?.close?.(); material.dispose();
        }
      } };
      if (abort.signal.aborted) { dispose(); return; }
      const mesh = meshes[0];
      const material = mesh?.material;
      if (meshes.length !== 1 || mesh === undefined || !(material instanceof MeshStandardMaterial) || !material.map) {
        dispose(); throw Error(t("run.texture.notPhotoGlb"));
      }
      if (!canvas.current || !host.current) { dispose(); return; }
      let view: Viewer | undefined;
      try {
        view = new Viewer(canvas.current, host.current);
        view.setPhotoMesh(mesh.geometry, material.map);
      } catch (problem) {
        view?.dispose(); dispose(); throw problem;
      }
      material.dispose();
      viewer.current = view;
      setReady(true);
    })().catch((problem) => { if (!abort.signal.aborted && !isAbort(problem)) setError(describe(problem)); });
    return () => { abort.abort(); viewer.current?.dispose(); viewer.current = null; };
  }, [open, source, path]);
  useEffect(() => { viewer.current?.setPhotoAppearance(photo); }, [photo, ready]);
  useEffect(() => { viewer.current?.setUp(up); }, [up, ready]);
  useEffect(() => {
    const style = getComputedStyle(document.documentElement);
    viewer.current?.setColors({ background: style.getPropertyValue("--viewer-bg").trim() || "#f6f7f9", surface: "#b9bec6" });
  }, [themeTick, ready]);
  return <>
    <button class="button" type="button" onClick={() => setOpen(!open)}>{open ? t("run.texture.close") : t("run.texture.open")}</button>
    {open && <>
      <div class="viewer" ref={host}><canvas ref={canvas} tabIndex={0} role="img" aria-label={t("run.texture.viewLabel")} onKeyDown={(event) => {
        const view = viewer.current;
        if (!view || event.ctrlKey || event.metaKey || event.altKey) return;
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
      }} />
        {!ready && !error && <p class="viewer-message" role="status">{t("run.texture.loading")}</p>}
        {error && <p class="viewer-message" role="alert">{error}</p>}
      </div>
      <div class="viewer-tools">
        <label class="check"><input type="checkbox" checked={photo} disabled={!ready} onChange={(e) => setPhoto(e.currentTarget.checked)} /> {t("run.texture.photo")}</label>
        <label class="inline-field"><span>{t("run.texture.up")}</span><select aria-label={t("run.texture.upLabel")} value={up} onChange={(e) => setUp(e.currentTarget.value as UpAxis)}>
          {["+X", "-X", "+Y", "-Y", "+Z", "-Z"].map((axis) => <option value={axis}>{axis}</option>)}
        </select></label>
        <button class="button" type="button" disabled={!ready} onClick={() => viewer.current?.resetView()}>{t("run.texture.resetView")}</button>
      </div>
      <p class="help">{t("run.texture.help")}</p>
    </>}
  </>;
}
