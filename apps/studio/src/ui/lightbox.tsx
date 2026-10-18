import { useEffect, useRef, useState } from "preact/hooks";
import type { Sheet } from "../core/reducer";
import type { RunSource } from "../sources/types";
import { useImageUrl } from "./gallery";
import { canDownload, downloadFile } from "./download";
import { Icon } from "./icons";
import { formatPercent } from "../core/format";
import { t } from "../i18n";

interface Props {
  source: RunSource;
  sheets: Sheet[];
  index: number;
  onIndex(index: number): void;
  onClose(): void;
}

interface View {
  scale: number;
  x: number;
  y: number;
}

const MAX_SCALE = 16;

/**
 * Full-window image viewer with zoom and pan: wheel or pinch to zoom at the pointer, drag to
 * move, double-click to switch between "fit" and 1:1. Keyboard: + - 0 1, arrows to move,
 * PageUp/PageDown or [ ] for the previous/next sheet, Escape to close.
 */
export function Lightbox({ source, sheets, index, onIndex, onClose }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const image = useRef<HTMLImageElement>(null);
  const view = useRef<View>({ scale: 1, x: 0, y: 0 });
  const natural = useRef({ width: 0, height: 0 });
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const [percent, setPercent] = useState(100);
  const [loaded, setLoaded] = useState(false);
  const sheet = sheets[index]!;
  const { url, failed } = useImageUrl(source, sheet.path);

  const apply = (next: View) => {
    const box = stage.current?.getBoundingClientRect();
    const { width, height } = natural.current;
    if (box !== undefined && width > 0) {
      // Keep the picture from being dragged out of sight: centred when smaller than the stage.
      const w = width * next.scale;
      const h = height * next.scale;
      next.x = w <= box.width ? (box.width - w) / 2 : Math.min(0, Math.max(box.width - w, next.x));
      next.y = h <= box.height ? (box.height - h) / 2 : Math.min(0, Math.max(box.height - h, next.y));
    }
    view.current = next;
    if (image.current !== null) {
      image.current.style.transform = `translate(${next.x}px, ${next.y}px) scale(${next.scale})`;
      image.current.classList.toggle("pixels", next.scale >= 2);
    }
    setPercent(Math.round(next.scale * 100));
  };

  const fitScale = (): number => {
    const box = stage.current?.getBoundingClientRect();
    const { width, height } = natural.current;
    if (box === undefined || width === 0 || height === 0) return 1;
    return Math.min(box.width / width, box.height / height, 1);
  };

  const fit = () => apply({ scale: fitScale(), x: 0, y: 0 });

  const zoomAt = (factor: number, clientX?: number, clientY?: number) => {
    const box = stage.current?.getBoundingClientRect();
    if (box === undefined) return;
    const px = (clientX ?? box.left + box.width / 2) - box.left;
    const py = (clientY ?? box.top + box.height / 2) - box.top;
    const current = view.current;
    const scale = Math.min(MAX_SCALE, Math.max(fitScale() * 0.5, current.scale * factor));
    const ratio = scale / current.scale;
    apply({ scale, x: px - (px - current.x) * ratio, y: py - (py - current.y) * ratio });
  };

  useEffect(() => {
    const element = dialog.current;
    if (element !== null && !element.open) element.showModal();
    return () => element?.close();
  }, []);

  useEffect(() => {
    setLoaded(false);
  }, [sheet.path]);

  useEffect(() => {
    const element = stage.current;
    if (element === null) return;
    const observer = new ResizeObserver(() => fit());
    observer.observe(element);
    // Wheel must be non-passive to keep the page from scrolling behind the dialog.
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      zoomAt(Math.exp(-event.deltaY * (event.ctrlKey ? 0.01 : 0.0015)), event.clientX, event.clientY);
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      observer.disconnect();
      element.removeEventListener("wheel", onWheel);
    };
  }, []);

  const go = (delta: number) => {
    const next = index + delta;
    if (next >= 0 && next < sheets.length) onIndex(next);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    const pan = 60;
    const current = view.current;
    switch (event.key) {
      case "+":
      case "=":
        zoomAt(1.25);
        break;
      case "-":
        zoomAt(0.8);
        break;
      case "0":
        fit();
        break;
      case "1":
        zoomAt(1 / current.scale);
        break;
      case "ArrowLeft":
        apply({ ...current, x: current.x + pan });
        break;
      case "ArrowRight":
        apply({ ...current, x: current.x - pan });
        break;
      case "ArrowUp":
        apply({ ...current, y: current.y + pan });
        break;
      case "ArrowDown":
        apply({ ...current, y: current.y - pan });
        break;
      case "PageUp":
      case "[":
        go(-1);
        break;
      case "PageDown":
      case "]":
        go(1);
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  const onPointerDown = (event: PointerEvent) => {
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
  };

  const onPointerMove = (event: PointerEvent) => {
    const previous = pointers.current.get(event.pointerId);
    if (previous === undefined) return;
    const others = [...pointers.current.entries()].filter(([id]) => id !== event.pointerId);
    const current = view.current;
    if (others.length === 0) {
      apply({ ...current, x: current.x + event.clientX - previous.x, y: current.y + event.clientY - previous.y });
    } else {
      // Pinch: scale by the change in distance to the other finger, around the midpoint.
      const other = others[0]![1];
      const before = Math.hypot(previous.x - other.x, previous.y - other.y);
      const after = Math.hypot(event.clientX - other.x, event.clientY - other.y);
      if (before > 0) zoomAt(after / before, (event.clientX + other.x) / 2, (event.clientY + other.y) / 2);
    }
    pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
  };

  const onPointerEnd = (event: PointerEvent) => {
    pointers.current.delete(event.pointerId);
  };

  return (
    <dialog
      ref={dialog}
      class="lightbox"
      aria-label={sheet.label}
      onClose={onClose}
      onKeyDown={onKeyDown}
      onClick={(event) => {
        if (event.target === dialog.current) onClose();
      }}
    >
      <div class="lightbox-bar">
        <p class="lightbox-title">
          <span class="lightbox-count">
            {index + 1} / {sheets.length}
          </span>{" "}
          {sheet.label}
        </p>
        <div class="lightbox-tools">
          <button type="button" class="button icon-only" onClick={() => go(-1)} disabled={index === 0} aria-label={t("run.lightbox.previous")}>
            <Icon name="prev" />
          </button>
          <button type="button" class="button icon-only" onClick={() => go(1)} disabled={index === sheets.length - 1} aria-label={t("run.lightbox.next")}>
            <Icon name="next" />
          </button>
          <button type="button" class="button icon-only" onClick={() => zoomAt(0.8)} aria-label={t("run.lightbox.zoomOut")}>
            <Icon name="minus" />
          </button>
          <span class="lightbox-zoom" aria-live="polite">
            {formatPercent(percent / 100, 0)}
          </span>
          <button type="button" class="button icon-only" onClick={() => zoomAt(1.25)} aria-label={t("run.lightbox.zoomIn")}>
            <Icon name="plus" />
          </button>
          {canDownload(source) && (
            <button type="button" class="button" onClick={() => void downloadFile(source, sheet.path).catch(() => undefined)}>
              {t("run.lightbox.download")}
            </button>
          )}
          <button type="button" class="button icon-only" onClick={fit} aria-label={t("run.lightbox.fit")}>
            <Icon name="fit" />
          </button>
          <button type="button" class="button icon-only" onClick={onClose} aria-label={t("run.lightbox.close")} autofocus>
            <Icon name="close" />
          </button>
        </div>
      </div>
      <div
        class="lightbox-stage"
        ref={stage}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerEnd}
        onPointerCancel={onPointerEnd}
        onDblClick={(event) => {
          const fitted = fitScale();
          if (view.current.scale > fitted * 1.05) fit();
          else zoomAt(1 / view.current.scale, event.clientX, event.clientY);
        }}
      >
        {url !== undefined && !failed && (
          <img
            ref={image}
            key={sheet.path}
            src={url}
            alt={sheet.label}
            draggable={false}
            class={loaded ? "" : "loading"}
            onLoad={(event) => {
              natural.current = { width: event.currentTarget.naturalWidth, height: event.currentTarget.naturalHeight };
              setLoaded(true);
              fit();
            }}
          />
        )}
        {failed && <p class="viewer-message">{t("run.lightbox.failed")}</p>}
      </div>
    </dialog>
  );
}
