import { useEffect, useRef, useState } from "preact/hooks";
import { AUTO_CAPTURE_DEFAULTS, AutoCapture, captureName, greyThumbnail } from "../core/autoCapture";
import { captureCalibration, type CaptureFrame } from "../core/captureLens";
import { cameraFieldOfView } from "../shell/shell";
import type { Engine } from "../sources/types";
import { describe } from "../sources/transport";
import { importFolderName } from "./deviceImport";

interface Props {
  /**
   * The photos taken, in capture order, named capture_0001.jpg and on, and their frame size when every one is a frame
   * of the default camera's live picture (null otherwise: the system camera, or another camera chosen).
   */
  onDone(files: File[], frame: CaptureFrame | null): void;
  onCancel(): void;
}

interface Taken {
  file: File;
  url: string;
  /** Frame size of the default camera's live picture; null for the system camera or a camera chosen in the list. */
  frame: CaptureFrame | null;
}

/** The common frame of all photos, when there is one (see Props.onDone). */
export function commonFrame(photos: { frame: CaptureFrame | null }[]): CaptureFrame | null {
  const first = photos[0]?.frame ?? null;
  if (first === null) return null;
  return photos.every((photo) => photo.frame !== null && photo.frame.width === first.width && photo.frame.height === first.height) ? first : null;
}

/** Photos per turn offered. 20 (18-degree steps) works with the neural surface; the ordinary mesh wants 36. */
const TARGETS = [20, 24, 36, 48, 72];
/** How often the picture is compared, in milliseconds. */
const TICK_MS = 100;

/** Why the camera cannot be used, as a sentence. */
function cameraProblem(error: unknown): string {
  const name = (error as { name?: string } | null)?.name ?? "";
  if (name === "NotAllowedError" || name === "SecurityError") return "The camera was not allowed. Allow it for this app (or site) and try again.";
  if (name === "NotFoundError" || name === "OverconstrainedError") return "No camera was found.";
  if (name === "NotReadableError") return "The camera is in use by another program.";
  return `The camera could not be started${error instanceof Error && error.message ? `: ${error.message}` : "."}`;
}

/**
 * Takes the photos of a turntable capture with this device's camera: by hand (button or space bar) or by itself
 * (Auto: a photo each time the object has turned and stands still again). The camera must not move between photos;
 * the object turns. Photos are full camera frames as JPEG, kept in memory until "Use".
 */
export function CaptureScreen({ onDone, onCancel }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const systemCamera = useRef<HTMLInputElement>(null);
  const stream = useRef<MediaStream | null>(null);
  const detector = useRef(new AutoCapture());
  const lastFrame = useRef<Uint8Array | null>(null);
  const busy = useRef(false);
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [deviceId, setDeviceId] = useState<string>("");
  const [problem, setProblem] = useState("");
  const [ready, setReady] = useState(false);
  const [auto, setAuto] = useState(true);
  const [target, setTarget] = useState(36);
  const [photos, setPhotos] = useState<Taken[]>([]);
  const [status, setStatus] = useState("");
  const [flash, setFlash] = useState(false);
  // The detector's last numbers, on the status line as data attributes (for checks and for tuning).
  const [measure, setMeasure] = useState({ motion: 0, sinceLast: 0 });
  const photosRef = useRef<Taken[]>([]);
  photosRef.current = photos;

  useEffect(() => {
    const element = dialog.current;
    if (element !== null && !element.open) element.showModal();
    return () => {
      for (const photo of photosRef.current) URL.revokeObjectURL(photo.url);
    };
  }, []);

  // The camera: the chosen one, else the one facing away from the screen.
  useEffect(() => {
    let alive = true;
    const media = typeof navigator === "undefined" ? undefined : navigator.mediaDevices;
    if (media?.getUserMedia === undefined) {
      setProblem("This app or browser cannot use a camera here (a camera needs a secure page: https or the app itself).");
      return;
    }
    setReady(false);
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
    if (element === null || busy.current || element.videoWidth === 0) return;
    busy.current = true;
    try {
      const canvas = document.createElement("canvas");
      canvas.width = element.videoWidth;
      canvas.height = element.videoHeight;
      canvas.getContext("2d")!.drawImage(element, 0, 0);
      const blob = await new Promise<Blob | null>((done) => canvas.toBlob(done, "image/jpeg", 0.92));
      if (blob === null) throw new Error("the frame could not be encoded");
      const file = new File([blob], captureName(photosRef.current.length), { type: "image/jpeg" });
      const frame = deviceId === "" ? { width: canvas.width, height: canvas.height } : null;
      setPhotos((current) => [...current, { file, url: URL.createObjectURL(blob), frame }]);
      if (lastFrame.current !== null) detector.current.taken(lastFrame.current);
      setFlash(true);
      setTimeout(() => setFlash(false), 150);
    } catch (error) {
      setStatus(`The photo could not be taken: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      busy.current = false;
    }
  };

  // Compare small grey copies of the picture; in Auto, take a photo when the object has turned and stands still.
  useEffect(() => {
    if (!ready) return;
    const small = document.createElement("canvas");
    const context = small.getContext("2d", { willReadFrequently: true });
    const timer = setInterval(() => {
      const element = video.current;
      if (element === null || context === null || element.videoWidth === 0) return;
      const scale = 96 / Math.max(element.videoWidth, element.videoHeight);
      small.width = Math.max(1, Math.round(element.videoWidth * scale));
      small.height = Math.max(1, Math.round(element.videoHeight * scale));
      context.drawImage(element, 0, 0, small.width, small.height);
      const grey = greyThumbnail(context.getImageData(0, 0, small.width, small.height).data, small.width, small.height, Math.max(small.width, small.height)).data;
      lastFrame.current = grey;
      const step = detector.current.frame(grey, performance.now());
      setMeasure({ motion: step.motion, sinceLast: step.sinceLast });
      if (!auto) return;
      if (photosRef.current.length >= target) {
        setStatus(`${target} photos taken. Use them, or take more by hand.`);
        return;
      }
      setStatus(step.state === "waiting" ? "Turn the object a little, then let it stand still." : step.motion > AUTO_CAPTURE_DEFAULTS.still ? "Moving..." : "Holding still...");
      if (step.take) void take();
    }, TICK_MS);
    return () => clearInterval(timer);
  }, [ready, auto, target]);

  const removeLast = () => {
    setPhotos((current) => {
      const last = current.at(-1);
      if (last !== undefined) URL.revokeObjectURL(last.url);
      return current.slice(0, -1);
    });
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
      aria-label="Take photos"
      onClose={onCancel}
      onKeyDown={(event) => {
        if (event.key === " " && (event.target as HTMLElement).tagName !== "SELECT") {
          event.preventDefault();
          void take();
        }
      }}
    >
      <div class="lightbox-bar">
        <p class="lightbox-title">
          Take photos <span class="lightbox-count">{photos.length} / {target}</span>
        </p>
        <div class="lightbox-tools">
          {devices.length > 1 && (
            <select aria-label="Camera" value={deviceId} onChange={(event) => setDeviceId(event.currentTarget.value)}>
              <option value="">Default camera</option>
              {devices.map((device, index) => (
                <option key={device.deviceId} value={device.deviceId}>
                  {device.label || `Camera ${index + 1}`}
                </option>
              ))}
            </select>
          )}
          <select aria-label="Photos for one turn" value={String(target)} onChange={(event) => setTarget(Number(event.currentTarget.value))}>
            {TARGETS.map((count) => (
              <option key={count} value={String(count)}>
                {count} photos ({Math.round(360 / count)}° steps){count < 25 ? ", with the neural surface" : ""}
              </option>
            ))}
          </select>
          <button type="button" class="button" onClick={close}>
            Cancel
          </button>
        </div>
      </div>
      <div class="capture-stage">
        <video ref={video} class={flash ? "capture-video flash" : "capture-video"} playsInline muted autoPlay />
        {problem !== "" && (
          <div class="capture-problem" role="alert">
            <p>{problem}</p>
            {/* Without a live picture: the system's own camera, one photo per tap (no Auto). */}
            <button type="button" class="button primary" onClick={() => systemCamera.current?.click()}>
              Take a photo with the system camera
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
                setPhotos((current) => [
                  ...current,
                  ...picked.map((file, index) => {
                    const named = new File([file], captureName(current.length + index), { type: file.type || "image/jpeg" });
                    return { file: named, url: URL.createObjectURL(named), frame: null };
                  }),
                ]);
              }}
            />
          </div>
        )}
      </div>
      <div class="capture-controls">
        <label class="capture-auto">
          <input type="checkbox" checked={auto} onChange={(event) => setAuto(event.currentTarget.checked)} /> Auto: a photo each time the object
          has turned and stands still
        </label>
        <p
          class="capture-status"
          role="status"
          aria-live="polite"
          data-motion={measure.motion.toFixed(4)}
          data-since-last={measure.sinceLast.toFixed(4)}
          data-size={`${video.current?.videoWidth ?? 0}x${video.current?.videoHeight ?? 0}`}
        >
          {status !== "" ? status : "Camera on a stand, object in the middle and filling most of the picture, plain light backdrop."}
        </p>
        <div class="capture-buttons">
          <button type="button" class="button primary" onClick={() => void take()} disabled={!ready}>
            Take photo
          </button>
          <button type="button" class="button" onClick={removeLast} disabled={photos.length === 0}>
            Delete last
          </button>
          <button type="button" class="button primary" onClick={() => onDone(photos.map((photo) => photo.file), commonFrame(photos))} disabled={photos.length < 8}>
            Use {photos.length} photos
          </button>
        </div>
        {photos.length > 0 && (
          <div class="capture-strip" aria-label="Photos taken">
            {photos.slice(-12).map((photo) => (
              <img key={photo.url} src={photo.url} alt={photo.file.name} title={photo.file.name} />
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
export function CaptureButton({ engine, onSaved }: { engine: Engine; onSaved(folder: string, count: number, calibration: string): void }) {
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<{ message: string; bad: boolean }>({ message: "", bad: false });
  const save = engine.saveCapture?.bind(engine);
  if (save === undefined) return null;
  return (
    <div class="device-import">
      <button type="button" class="button" onClick={() => setOpen(true)}>
        Take photos
      </button>
      {open && (
        <CaptureScreen
          onCancel={() => setOpen(false)}
          onDone={(files, frame) => {
            setOpen(false);
            // The lens: the camera's field of view where this device knows it (the iOS app), else found from the turn.
            const calibration = captureCalibration(frame, cameraFieldOfView);
            const folder = importFolderName(new Date()).replace("photos-", "capture-");
            setState({ message: `Saving ${files.length} photos...`, bad: false });
            save(folder, files, (done, total) => setState({ message: `Saving ${done} of ${total}...`, bad: false })).then(
              async () => {
                setState({ message: `${files.length} photos saved in ${folder}.`, bad: false });
                onSaved(folder, files.length, await calibration);
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
