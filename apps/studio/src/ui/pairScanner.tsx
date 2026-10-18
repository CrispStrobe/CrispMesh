import jsQR from "jsqr";
import { useEffect, useRef, useState } from "preact/hooks";
import { parsePairing, type Pairing } from "../core/pairing";
import { t } from "../i18n";

interface Props {
  onFound(pairing: Pairing): void;
  onCancel(): void;
}

/** Scans the computer's pairing code with this device's camera (Connection, Another engine). */
export function PairScanner({ onFound, onCancel }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const [problem, setProblem] = useState("");
  const [seen, setSeen] = useState(false);

  useEffect(() => {
    const element = dialog.current;
    if (element !== null && !element.open) element.showModal();
    let stream: MediaStream | null = null;
    let alive = true;
    const media = typeof navigator === "undefined" ? undefined : navigator.mediaDevices;
    if (media?.getUserMedia === undefined) {
      setProblem(t("connection.scan.noCamera"));
      return;
    }
    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d", { willReadFrequently: true });
    let timer: ReturnType<typeof setInterval> | undefined;
    media
      .getUserMedia({ video: { facingMode: { ideal: "environment" }, width: { ideal: 1280 } }, audio: false })
      .then(async (opened) => {
        if (!alive) {
          opened.getTracks().forEach((track) => track.stop());
          return;
        }
        stream = opened;
        if (video.current !== null) {
          video.current.srcObject = opened;
          await video.current.play().catch(() => undefined);
        }
        timer = setInterval(() => {
          const frame = video.current;
          if (frame === null || context === null || frame.videoWidth === 0) return;
          const scale = Math.min(1, 800 / Math.max(frame.videoWidth, frame.videoHeight));
          canvas.width = Math.round(frame.videoWidth * scale);
          canvas.height = Math.round(frame.videoHeight * scale);
          context.drawImage(frame, 0, 0, canvas.width, canvas.height);
          const code = jsQR(context.getImageData(0, 0, canvas.width, canvas.height).data, canvas.width, canvas.height);
          if (code === null) return;
          const pairing = parsePairing(code.data);
          if (pairing === null) {
            setSeen(true);
            return;
          }
          clearInterval(timer);
          onFound(pairing);
        }, 200);
      })
      .catch(() => alive && setProblem(t("connection.scan.cameraFailed")));
    return () => {
      alive = false;
      if (timer !== undefined) clearInterval(timer);
      stream?.getTracks().forEach((track) => track.stop());
    };
  }, []);

  return (
    <dialog ref={dialog} class="lightbox capture" aria-label={t("connection.scan.label")} onClose={onCancel}>
      <div class="lightbox-bar">
        <p class="lightbox-title">{t("connection.scan.title")}</p>
        <div class="lightbox-tools">
          <button type="button" class="button" onClick={() => dialog.current?.close()}>
            {t("connection.scan.cancel")}
          </button>
        </div>
      </div>
      <div class="capture-stage">
        <video ref={video} class="capture-video" playsInline muted autoPlay />
        {problem !== "" && (
          <p class="capture-problem" role="alert">
            {problem}
          </p>
        )}
      </div>
      <div class="capture-controls">
        <p class="capture-status" role="status">
          {seen
            ? t("connection.scan.notPairing")
            : t("connection.scan.hint")}
        </p>
      </div>
    </dialog>
  );
}
