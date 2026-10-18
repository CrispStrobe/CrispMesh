import { t } from "../i18n";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { AutoCapture, captureName, centralRegion, greyThumbnail, renumberPhotos, turnProblem, turnSizes, MOST_PHOTOS } from "../core/autoCapture";
import { captureCalibration, type CaptureFrame } from "../core/captureLens";
import { CROP_DEFAULTS, focusRegion, trackingRegion, type ForegroundPreview, type ImageRegion, type CropSettings } from "../core/foregroundPreview";
import { CropReview } from "./cropReview";
import { cameraFieldOfView, foregroundAvailable, foregroundPreview } from "../shell/shell";
import type { Engine } from "../sources/types";
import { describe } from "../sources/transport";
import { importFolderName } from "./deviceImport";

interface Props {
  /**
   * The photos taken, in capture order, named capture_0001.jpg and on, and their frame size when every one is a frame
   * of the default camera's live picture (null otherwise: the system camera, or another camera chosen).
   */
  onDone(files: File[], frame: CaptureFrame | null, sizes: number[], foreground?: { masks: File[]; padding: number; skipped: number; skippedFiles: File[] }): void;
  onCancel(): void;
  foregroundSupported?: boolean;
}

interface Taken {
  id: string;
  capturedAt: number;
  turn: number;
  file: File;
  url: string;
  /** Frame size of the default camera's live picture; null for the system camera or a camera chosen in the list. */
  frame: CaptureFrame | null;
}

/** The common frame of all photos, when there is one (see Props.onDone). */
export function commonFrame(photos: { frame: CaptureFrame | null }[]): CaptureFrame | null {
  const first = photos[0]?.frame ?? null;
  if (first === null) return null;
  return photos.every((photo) => photo.frame !== null && photo.frame.width === first.width && photo.frame.height === first.height && photo.frame.sourceWidth === first.sourceWidth && photo.frame.sourceHeight === first.sourceHeight) ? first : null;
}

/** Photos per turn offered. 20 (18-degree steps) works with the neural surface; the ordinary mesh wants 36. */
const TARGETS = [20, 24, 36, 48, 72, 120, 180, 360];
/** How often the picture is compared, in milliseconds. */
const TICK_MS = 100;

/** Why the camera cannot be used, as a sentence. */
function cameraProblem(error: unknown): string {
  const name = (error as { name?: string } | null)?.name ?? "";
  if (name === "NotAllowedError" || name === "SecurityError") return t("capture.denied");
  if (name === "NotFoundError" || name === "OverconstrainedError") return t("capture.notFound");
  if (name === "NotReadableError") return t("capture.inUse");
  return t("capture.cameraError", { error: error instanceof Error ? error.message : String(error) });
}

/**
 * Takes the photos of a turntable capture with this device's camera: by hand (button or space bar) or by itself
 * (Auto: a photo each time the object has turned and stands still again). The camera must not move between photos;
 * the object turns. Photos are full camera frames as JPEG, kept in memory until "Use".
 */
export function CaptureScreen({ onDone, onCancel, foregroundSupported = false }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const watchRegionRef = useRef<ImageRegion | null>(null);
  const [watchRegion, setWatchRegion] = useState<ImageRegion | null>(null);
  const [watchBox, setWatchBox] = useState<{ left: number; top: number; width: number; height: number } | null>(null);
  const systemCamera = useRef<HTMLInputElement>(null);
  const stream = useRef<MediaStream | null>(null);
  const detector = useRef(new AutoCapture());
  const lastFrame = useRef<Uint8Array | null>(null);
  const busy = useRef(false);
  const nextPhoto = useRef(0);
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [deviceId, setDeviceId] = useState<string>("");
  const [problem, setProblem] = useState("");
  const [ready, setReady] = useState(false);
  const [auto, setAuto] = useState(true);
  const [cropAvailable, setCropAvailable] = useState(false);
  const [autoCrop, setAutoCrop] = useState(false);
  const [cropSettings, setCropSettings] = useState<CropSettings>(CROP_DEFAULTS);
  const [cropBusy, setCropBusy] = useState(false);
  const cropCache = useRef<{ key: string; results: Map<File, ForegroundPreview | null> }>({ key: "", results: new Map() });
  const [finishError, setFinishError] = useState("");
  const [saving, setSaving] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [paused, setPaused] = useState(false);
  const [turn, setTurn] = useState(0);
  const [heights, setHeights] = useState(1);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [target, setTarget] = useState(36);
  const [photos, setPhotos] = useState<Taken[]>([]);
  const previewFiles = useMemo(() => photos.map(photo => photo.file), [photos]);
  useEffect(() => { let alive = true; void foregroundAvailable().then(value => { if (alive) setCropAvailable(foregroundSupported && value); }); return () => { alive = false; }; }, [foregroundSupported]);
  const [status, setStatus] = useState("");
  const [flash, setFlash] = useState(false);
  // The detector's last numbers, on the status line as data attributes (for checks and for tuning).
  const [measure, setMeasure] = useState({ motion: 0, sinceLast: 0 });
  const active = useRef(true);
  const mounted = useRef(true);
  active.current = !paused;
  const photosRef = useRef<Taken[]>([]);
  photosRef.current = photos;

  useEffect(() => {
    mounted.current = true;
    const element = dialog.current;
    if (element !== null && !element.open) element.showModal();
    return () => {
      mounted.current = false;
      for (const photo of photosRef.current) URL.revokeObjectURL(photo.url);
    };
  }, []);

  // The camera: the chosen one, else the one facing away from the screen.
  useEffect(() => {
    let alive = true;
    const media = typeof navigator === "undefined" ? undefined : navigator.mediaDevices;
    if (media?.getUserMedia === undefined) {
      setProblem(t("capture.secure"));
      return;
    }
    setReady(false);
    detector.current.reset();
    watchRegionRef.current = null;setWatchRegion(null);
    lastFrame.current = null;
    const wanted: MediaTrackConstraints = {
      width: { ideal: 4096 },
      height: { ideal: 3072 },
      ...(deviceId !== "" ? { deviceId: { exact: deviceId } } : { facingMode: { ideal: "environment" } }),
    };
    media
      .getUserMedia({ video: wanted, audio: false })
      .then(async (opened) => {
        if (!alive) {
          opened.getTracks().forEach((track) => track.stop());
          return;
        }
        stream.current?.getTracks().forEach((track) => track.stop());
        stream.current = opened;
        if (video.current !== null) {
          video.current.srcObject = opened;
          await video.current.play().catch(() => undefined);
        }
        setProblem("");
        setReady(true);
        const list = await media.enumerateDevices().catch(() => []);
        if (alive) setDevices(list.filter((device) => device.kind === "videoinput"));
      })
      .catch((error) => alive && setProblem(cameraProblem(error)));
    return () => {
      alive = false;
    };
  }, [deviceId]);

  useEffect(
    () => () => {
      stream.current?.getTracks().forEach((track) => track.stop());
      stream.current = null;
    },
    [],
  );

  const take = async () => {
    const element = video.current;
    if (element === null || busy.current || !active.current || photosRef.current.length >= MOST_PHOTOS || element.videoWidth === 0) return;
    busy.current = true;
    try {
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.round(element.videoWidth / zoom));
      canvas.height = Math.max(1, Math.round(element.videoHeight / zoom));
      const left = (element.videoWidth - canvas.width) / 2;
      const top = (element.videoHeight - canvas.height) / 2;
      canvas.getContext("2d")!.drawImage(element, left, top, canvas.width, canvas.height, 0, 0, canvas.width, canvas.height);
      const capturedAt = performance.now();
      const capturedFrame = detectorFrame(canvas, 1, watchRegionRef.current);
      const blob = await new Promise<Blob | null>((done) => canvas.toBlob(done, "image/jpeg", 0.92));
      if (!mounted.current) return;
      if (blob === null) throw new Error(t("capture.encodeError"));
      const number = nextPhoto.current++;
      const file = new File([blob], captureName(number), { type: "image/jpeg" });
      const frame = deviceId === "" ? { width: canvas.width, height: canvas.height, sourceWidth: element.videoWidth, sourceHeight: element.videoHeight } : null;
      setPhotos((current) => [...current, { id: String(number), capturedAt, file, url: previewUrl(canvas), frame, turn }]);
      detector.current.taken(capturedFrame, capturedAt);
      setFlash(true);
      setTimeout(() => setFlash(false), 150);
    } catch (error) {
      setStatus(t("capture.photoError", { error: error instanceof Error ? error.message : String(error) }));
    } finally {
      busy.current = false;
    }
  };

  // Compare small grey copies of the picture; in Auto, take a photo when the object has turned and stands still.
  useEffect(() => {
    if (!ready) return;
    const timer = setInterval(() => {
      const element = video.current;
      if (element === null || element.videoWidth === 0 || !active.current || busy.current) return;
      const grey = detectorFrame(element, zoom, watchRegionRef.current);
      lastFrame.current = grey;
      const step = detector.current.frame(grey, performance.now());
      setMeasure({ motion: step.motion, sinceLast: step.sinceLast });
      if (!auto) return;
      if (photosRef.current.filter((photo) => photo.turn === turn).length >= target || photosRef.current.length >= MOST_PHOTOS) {
        setStatus(t("capture.targetReached", { count: target }));
        return;
      }
      setStatus(t(`capture.look.${step.look}`));
      if (step.take) void take();
    }, TICK_MS);
    return () => clearInterval(timer);
  }, [ready, auto, target, turn, zoom]);

  // Place the detector over the object instead of a large static background. Re-anchor to the
  // saved last photo in the new area so selecting an area cannot itself create another shot.
  const focusAuto = async (region: ImageRegion | null) => {
    if (busy.current || saving) return;
    busy.current = true;
    try {
      const last = photosRef.current.at(-1);
      if (last?.turn === turn) {
        const image = new Image();const url = URL.createObjectURL(last.file);
        try {
          image.src = url;await image.decode();
          if (mounted.current) {
            detector.current.taken(detectorFrame(image, 1, region), last.capturedAt);
            watchRegionRef.current = region;setWatchRegion(region);
          }
        } finally {URL.revokeObjectURL(url);}
      } else {
        detector.current.reset();watchRegionRef.current = region;setWatchRegion(region);
      }
    } catch (error) {setStatus(t("capture.autoAreaFailed", {error:describe(error)}));}
    finally { busy.current = false; }
  };
  useEffect(() => {
    const element = video.current, parent = stage.current;
    if (!element || !parent || !watchRegion) { setWatchBox(null);return; }
    const update = () => {
      const rect = element.getBoundingClientRect(), container = parent.getBoundingClientRect();
      const [l,t,r,b] = watchRegion;
      // The transformed video rectangle includes digital zoom. The detection area is in the saved frame.
      setWatchBox({ left: rect.left-container.left+(0.5+(l-0.5)/zoom)*rect.width,
        top: rect.top-container.top+(0.5+(t-0.5)/zoom)*rect.height,
        width: (r-l)/zoom*rect.width, height: (b-t)/zoom*rect.height });
    };
    const observer = new ResizeObserver(update);observer.observe(element);observer.observe(parent);update();
    return () => observer.disconnect();
  }, [watchRegion, zoom, ready]);

  const removeLast = () => {
    setPhotos((current) => {
      const last = current.at(-1);
      if (last !== undefined) URL.revokeObjectURL(last.url);
      return current.slice(0, -1);
    });
  };

  const sizes = turnSizes(photos);
  const problemWithTurns = turnProblem(sizes);
  const turnCount = photos.filter((photo) => photo.turn === turn).length;
  const togglePause = () => {
    detector.current.resume();
    setStatus("");
    setPaused(!paused);
  };
  const nextTurn = () => {
    setPaused(true);
    setTurn(turn + 1);
    watchRegionRef.current = null;setWatchRegion(null);
    detector.current.reset();
    setStatus("");
    setSelected(new Set());
  };
  const removeSelected = () => {
    setPhotos((current) => current.filter((photo) => {
      if (!selected.has(photo.id)) return true;
      URL.revokeObjectURL(photo.url);
      return false;
    }));
    setSelected(new Set());
    detector.current.resume();
  };

  const finish = async () => {
    if (busy.current || saving || (autoCrop && cropBusy)) return;
    if (!autoCrop) {active.current = false;onDone(renumberPhotos(photos), commonFrame(photos), sizes);return;}
    busy.current = true;setFinishError("");setSaving(true);setPaused(true);
    try {
      const valid: Taken[] = [];const masks: Blob[] = [];
      let previous: ForegroundPreview | null = null;
      const cached = cropCache.current.key === `${cropSettings.focus}/${cropSettings.threshold}` ? cropCache.current.results : new Map<File,ForegroundPreview|null>();
      for (const [index, photo] of photos.entries()) {
        if (!mounted.current) return;
        setStatus(t("capture.crop.progress", { done: index + 1, total: photos.length }));
        const result: ForegroundPreview | null = cached.has(photo.file) ? cached.get(photo.file)! : await foregroundPreview(photo.file, cropSettings.focus, cropSettings.threshold, previous ? trackingRegion(previous) : undefined);
        if (result !== null) {previous=result;valid.push(photo);masks.push(result.mask);}
      }
      if (!mounted.current) return;
      const keptSizes = turnSizes(valid);
      if (valid.length < 8 || turnProblem(keptSizes)) {setFinishError(t("capture.crop.tooFew"));return;}
      const files = renumberPhotos(valid);
      const maskFiles = masks.map((mask, i) => new File([mask], files[i]!.name + ".png", { type: "image/png" }));
      active.current = false;
      onDone(files, commonFrame(valid), keptSizes, { masks: maskFiles, padding: cropSettings.padding, skipped: photos.length - valid.length, skippedFiles: photos.filter(photo => !valid.includes(photo)).map(photo => photo.file) });
    } catch {setFinishError(t("capture.crop.failed"));}
    finally {busy.current = false;setSaving(false);}
  };

  const close = () => {
    stream.current?.getTracks().forEach((track) => track.stop());
    stream.current = null;
    dialog.current?.close();
  };

  return (
    <dialog
      ref={dialog}
      class="lightbox capture"
      aria-label={t("capture.title")}
      onClose={onCancel}
      onKeyDown={(event) => {
        if (event.key === " " && event.target === event.currentTarget) {
          event.preventDefault();
          void take();
        }
      }}
    >
      <div class="lightbox-bar">
        <p class="lightbox-title">
          {t("capture.title")} <span class="lightbox-count">{photos.length} / {target * heights}</span>
        </p>
        <div class="lightbox-tools">
          {devices.length > 1 && (
            <select disabled={photos.length > 0 || saving} aria-label={t("capture.camera")} value={deviceId} onChange={(event) => setDeviceId(event.currentTarget.value)}>
              <option value="">{t("capture.defaultCamera")}</option>
              {devices.map((device, index) => (
                <option key={device.deviceId} value={device.deviceId}>
                  {device.label || t("capture.cameraNumber", { count: index + 1 })}
                </option>
              ))}
            </select>
          )}
          <select aria-label={t("capture.target")} value={String(target)} onChange={(event) => setTarget(Number(event.currentTarget.value))}>
            {TARGETS.map((count) => (
              <option key={count} value={String(count)} disabled={count * heights > MOST_PHOTOS}>
                {t("capture.steps", { count, degrees: Math.round(360 / count) })}{count < 25 ? t("capture.neural") : ""}
              </option>
            ))}
          </select>
          <label>{t("capture.heights")}{" "}<select aria-label={t("capture.heights")} value={heights} disabled={photos.length > 0} onChange={(event) => setHeights(Number(event.currentTarget.value))}>
            {Array.from({ length: 16 }, (_, i) => <option value={i + 1} disabled={(i + 1) * target > MOST_PHOTOS}>{i + 1}</option>)}
          </select></label>
          <button type="button" class="button" onClick={close}>
            {t("capture.cancel")}
          </button>
        </div>
      </div>
      <div class="capture-stage" ref={stage}>
        <video ref={video} style={{ transform: `scale(${zoom})` }} class={flash ? "capture-video flash" : "capture-video"} playsInline muted autoPlay
          onClick={event => {
            const rect = event.currentTarget.getBoundingClientRect();
            if (!rect.width || !rect.height) return;
            const x = ((event.clientX-rect.left)/rect.width-0.5)*zoom+0.5;
            const y = ((event.clientY-rect.top)/rect.height-0.5)*zoom+0.5;
            void focusAuto(focusRegion(x,y));
          }} />
        {watchBox && <span class="capture-watch-box" role="img" aria-label={t("capture.autoArea")} style={{left:watchBox.left,top:watchBox.top,width:watchBox.width,height:watchBox.height}} />}
        {problem !== "" && (
          <div class="capture-problem" role="alert">
            <p>{problem}</p>
            {/* Without a live picture: the system's own camera, one photo per tap (no Auto). */}
            <button type="button" class="button primary" disabled={paused || photos.length >= MOST_PHOTOS} onClick={() => systemCamera.current?.click()}>
              {t("capture.systemCamera")}
            </button>
            <input
              ref={systemCamera}
              class="visually-hidden"
              type="file"
              accept="image/*"
              capture="environment"
              tabIndex={-1}
              aria-hidden="true"
              onChange={(event) => {
                const picked = [...(event.currentTarget.files ?? [])];
                event.currentTarget.value = "";
                if (busy.current || saving) return;
                busy.current = true;
                void (async () => {
                  try {
                    for (const source of picked.slice(0, MOST_PHOTOS - photosRef.current.length)) {
                      // Re-encode the actual image: renaming an HEIC or PNG to .jpg is not a conversion.
                      const url = URL.createObjectURL(source);
                      try {
                        const image = new Image();image.src = url;await image.decode();
                        const canvas = document.createElement("canvas");canvas.width = image.naturalWidth;canvas.height = image.naturalHeight;
                        canvas.getContext("2d")!.drawImage(image, 0, 0);
                        const blob = await new Promise<Blob | null>(done => canvas.toBlob(done, "image/jpeg", 0.92));
                        if (!mounted.current) return;
                        if (blob === null) throw new Error(t("capture.encodeError"));
                        const number = nextPhoto.current++;
                        const file = new File([blob], captureName(number), { type: "image/jpeg" });
                        const taken = { id: String(number), capturedAt: performance.now(), file, url: previewUrl(canvas), frame: null, turn };
                        setPhotos(current => [...current, taken]);
                      } finally { URL.revokeObjectURL(url); }
                    }
                  } catch (error) { setStatus(t("capture.photoError", { error: describe(error) })); }
                  finally { busy.current = false; }
                })();
              }}
            />
          </div>
        )}
      </div>
      <div class="capture-controls">
        <label class="capture-zoom">{t("capture.zoom", { zoom })}
          <input type="range" min="1" max="4" step="0.1" value={zoom} disabled={photos.length > 0} aria-label={t("capture.zoomLabel")}
            onInput={(event) => { setZoom(Number(event.currentTarget.value)); watchRegionRef.current=null;setWatchRegion(null);detector.current.reset(); }} />
        </label>
        <span class="help">{t("capture.zoomHelp")}</span>
        <span class="help">{t("capture.autoAreaHelp")}</span>
        {watchRegion && <button class="button" type="button" disabled={saving} onClick={()=>void focusAuto(null)}>{t("capture.autoAreaReset")}</button>}
        <p>{t("capture.turn", { turn: turn + 1, heights, count: turnCount })}</p>
        <label class="capture-auto">
          <input type="checkbox" checked={auto} onChange={(event) => { detector.current.resume(); setAuto(event.currentTarget.checked); }} /> {t("capture.auto")}
        </label>
        <p
          class="capture-status"
          role="status"
          aria-live="polite"
          data-motion={measure.motion.toFixed(4)}
          data-since-last={measure.sinceLast.toFixed(4)}
          data-size={`${video.current?.videoWidth ?? 0}x${video.current?.videoHeight ?? 0}`}
        >
          {saving ? status : paused ? t("capture.paused") : status !== "" ? status : t("capture.advice")}
        </p>
        <div class="capture-buttons">
          <button type="button" class="button primary" onClick={() => void take()} disabled={!ready || paused || photos.length >= MOST_PHOTOS}>
            {t("capture.take")}
          </button>
          <button type="button" class="button" onClick={removeLast} disabled={saving || photos.length === 0}>
            {t("capture.deleteLast")}
          </button>
          <button type="button" class="button" onClick={togglePause} disabled={saving}>{paused ? t("capture.continue") : t("capture.pause")}</button>
          {turn + 1 < heights && <button type="button" class="button" disabled={saving || turnCount < 8} onClick={nextTurn}>{t("capture.nextHeight")}</button>}
          <button type="button" class="button" disabled={saving || selected.size === 0} onClick={removeSelected}>{t("capture.deleteSelected", { count: selected.size })}</button>
          <button type="button" class="button primary" onClick={() => void finish()} disabled={photos.length < 8 || problemWithTurns !== "" || busy.current || saving || (autoCrop && cropBusy)}>
            {t("capture.use", { count: photos.length })}
          </button>
        </div>
        {cropAvailable && <label class="check"><input type="checkbox" checked={autoCrop} disabled={saving} onChange={event => { setAutoCrop(event.currentTarget.checked);setPaused(true); }} />{t("capture.crop.enable")}</label>}
        {autoCrop && paused && photos.length > 0 && <CropReview files={previewFiles} settings={cropSettings} onSettings={setCropSettings} disabled={saving}
          onBusy={setCropBusy} onResults={(rows, focus, threshold) => {cropCache.current = {key:`${focus}/${threshold}`,results:new Map(rows.map(row=>[row.file,row.result]))};}} />}
        {autoCrop && !paused && <span class="help">{t("capture.crop.pausePreview")}</span>}
        {finishError && <p class="field-error" role="alert">{finishError}</p>}
        {problemWithTurns && <p class="field-error" role="alert">{problemWithTurns}</p>}
        {photos.length > 0 && (
          <div class="capture-strip" aria-label={t("capture.photos")}>
            {photos.map((photo, index) => (
              <label key={photo.id} class="capture-thumb">
                <input type="checkbox" disabled={saving} checked={selected.has(photo.id)} aria-label={t("capture.select", { count: index + 1, turn: photo.turn + 1 })}
                  onChange={(event) => { const checked = event.currentTarget.checked; setPaused(true); setSelected((current) => { const next = new Set(current); if (checked) next.add(photo.id); else next.delete(photo.id); return next; }); }} />
                <img src={photo.url} loading="lazy" alt={t("capture.photo", { count: index + 1, turn: photo.turn + 1 })} />
                <span>{index + 1} · {t("capture.height", { count: photo.turn + 1 })}</span>
              </label>
            ))}
          </div>
        )}
      </div>
    </dialog>
  );
}

/**
 * "Take photos" for an engine that keeps its data in a folder (the apps): the photos are saved into a new folder of
 * the data folder, which then becomes the run's photos.
 */
export function CaptureButton({ engine, onSaved }: { engine: Engine; onSaved(folder: string, count: number, calibration: string, sizes: number[], foreground?: { folder: string; padding: number; skipped: number }): void }) {
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<{ message: string; bad: boolean }>({ message: "", bad: false });
  const save = engine.saveCapture?.bind(engine);
  if (save === undefined) return null;
  return (
    <div class="device-import">
      <button type="button" class="button" onClick={() => setOpen(true)}>
        {t("capture.title")}
      </button>
      {open && (
        <CaptureScreen
          foregroundSupported
          onCancel={() => setOpen(false)}
          onDone={(files, frame, sizes, foreground) => {
            setOpen(false);
            // The lens: the camera's field of view where this device knows it (the iOS app), else found from the turn.
            const calibration = captureCalibration(frame, cameraFieldOfView);
            const folder = importFolderName(new Date()).replace("photos-", "capture-");
            setState({ message: t("capture.saving", { count: files.length }), bad: false });
            save(folder, files, (done, total) => setState({ message: t("capture.savingProgress", { done, total }), bad: false })).then(
              async () => {
                try {
                  const masksFolder = folder + "-masks";
                  if (foreground) {
                    await save(masksFolder, foreground.masks, (done,total) => setState({ message: t("capture.crop.savingMasks", { done, total }), bad: false }));
                    if (foreground.skippedFiles.length) await save(folder + "-unmatched", foreground.skippedFiles);
                  }
                  setState({ message: t("capture.saved", { count: files.length, folder }) + (foreground?.skipped ? " " + t("capture.crop.skipped", { count: foreground.skipped, folder: folder + "-unmatched" }) : ""), bad: false });
                  onSaved(folder, files.length, await calibration, sizes,
                    foreground ? { folder: masksFolder, padding: foreground.padding, skipped: foreground.skipped } : undefined);
                } catch (problem) {setState({ message: describe(problem), bad: true });}
              },
              (problem) => setState({ message: describe(problem), bad: true }),
            );
          }}
        />
      )}
      {state.message !== "" && (
        <span class={state.bad ? "field-error" : "help"} role="status">
          {state.message}
        </span>
      )}
    </div>
  );
}

/** Same central thumbnail for detection and the exact frame encoded by the shutter. */
function detectorFrame(source: HTMLCanvasElement | HTMLVideoElement | HTMLImageElement, zoom = 1, region: ImageRegion | null = null): Uint8Array {
  const width = source instanceof HTMLVideoElement ? source.videoWidth : source instanceof HTMLImageElement ? source.naturalWidth : source.width;
  const height = source instanceof HTMLVideoElement ? source.videoHeight : source instanceof HTMLImageElement ? source.naturalHeight : source.height;
  const small = document.createElement("canvas");
  const [left,top,right,bottom] = region ?? [0,0,1,1];
  const scale = 96 / Math.max(width*(right-left), height*(bottom-top));
  small.width = Math.max(1, Math.round(width*(right-left)*scale));
  small.height = Math.max(1, Math.round(height*(bottom-top)*scale));
  const context = small.getContext("2d", { willReadFrequently: true })!;
  const cropWidth = width / zoom, cropHeight = height / zoom;
  context.drawImage(source, (width-cropWidth)/2+left*cropWidth, (height-cropHeight)/2+top*cropHeight, (right-left)*cropWidth, (bottom-top)*cropHeight, 0, 0, small.width, small.height);
  const grey = greyThumbnail(context.getImageData(0, 0, small.width, small.height).data, small.width, small.height);
  return region ? grey.data : centralRegion(grey.data, grey.width, grey.height);
}

/** A small preview keeps hundreds of full-resolution photos out of the image decoder cache. */
function previewUrl(source: HTMLCanvasElement): string {
  const canvas = document.createElement("canvas");
  canvas.width = 192;
  canvas.height = Math.max(1, Math.round(192 * source.height / source.width));
  canvas.getContext("2d")!.drawImage(source, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL("image/jpeg", 0.7);
}
