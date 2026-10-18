import { t } from "../i18n";

/**
 * When to take the next photo of a turntable capture by itself: after the object has turned (the picture differs
 * enough from the last photo) and then stands still (consecutive frames agree) for a moment. Works on small grey
 * frames (see `greyThumbnail`, `centralRegion`); knows nothing about cameras or the DOM, so it is tested with
 * synthetic frames.
 */

export interface AutoCaptureSettings {
  /** Share of pixels changed from the last photo above which the object counts as turned. */
  turned: number;
  /** Share of pixels changed between consecutive frames below which the picture counts as still. */
  still: number;
  /** How long the picture must stay still before the photo is taken, in milliseconds. */
  settleMs: number;
  /** Shortest time between two photos, in milliseconds (an object that rocks to rest is not taken twice). */
  gapMs: number;
}

/**
 * Measured on the 3DLF Bunny and Dragon photos (96-pixel grey copies, change above `CHANGED_LEVEL`): a 10 degree step
 * changes at least 2.9 % of the pixels (5 degrees: median 2.7 %); strong sensor noise and a brightness shift of 8 grey
 * levels change none. The mean difference does not separate them: the backdrop dilutes it.
 */
export const AUTO_CAPTURE_DEFAULTS: AutoCaptureSettings = { turned: 0.015, still: 0.003, settleMs: 600, gapMs: 1000 };

/** Grey levels by which a pixel of the small copy must change to count. */
export const CHANGED_LEVEL = 12;

export type AutoCaptureState =
  /** Nothing taken yet, or waiting for the object to turn from the last photo. */
  | "waiting"
  /** Turned far enough; waiting for the picture to stand still. */
  | "settling";

/**
 * What Auto sees, for the live indicator: the picture moves; it stands still but has not turned from the last photo;
 * it has turned and is settling; a photo is taken now.
 */
export type AutoCaptureLook = "moving" | "still" | "settling" | "taking";

export interface AutoCaptureStep {
  state: AutoCaptureState;
  look: AutoCaptureLook;
  /** True when the photo should be taken now. */
  take: boolean;
  /** Share of pixels changed since the last photo, for display. */
  sinceLast: number;
  /** Share of pixels changed since the previous frame, for display. */
  motion: number;
}

/** The median grey level, from a histogram. */
function median(frame: Uint8Array): number {
  const counts = new Uint32Array(256);
  for (let i = 0; i < frame.length; i++) counts[frame[i]!]!++;
  let seen = 0;
  for (let level = 0; level < 256; level++) {
    seen += counts[level]!;
    if (seen * 2 >= frame.length) return level;
  }
  return 255;
}

/**
 * Share of pixels differing by more than `CHANGED_LEVEL` after both frames are brought to a common exposure.
 * Gains are capped at two and comparison is symmetric: moderate global exposure changes do not count as motion. The median is the backdrop's when the backdrop fills most of the picture,
 * so a turning object hardly moves it.
 */
export function changedShare(a: Uint8Array, b: Uint8Array): number {
  if (a.length !== b.length || a.length === 0) return Infinity;
  // Compare in a common exposure: symmetric under swapping frames, with an absolute
  // threshold in the brighter frame's units. Avoid amplifying a nearly black image.
  const ma = median(a);
  const mb = median(b);
  const common = Math.max(ma, mb, 16);
  const ga = Math.min(2, common / Math.max(ma, 16));
  const gb = Math.min(2, common / Math.max(mb, 16));
  let changed = 0;
  for (let i = 0; i < a.length; i++) if (Math.abs(a[i]! * ga - b[i]! * gb) > CHANGED_LEVEL) changed++;
  return changed / a.length;
}

/**
 * The detector: feed it every frame with its time. The first photo is taken as soon as the picture is still; every
 * later one once the object has turned and come to rest again. Still means: consecutive frames agree, and the picture
 * has not drifted since the stillness began either (a slowly turning table changes little from frame to frame).
 * `taken()` resets it after any photo, also one taken by hand; `reset()` forgets the last photo (the camera moved).
 */
export class AutoCapture {
  private last: Uint8Array | null = null;
  private lastAt = -Infinity;
  private previous: Uint8Array | null = null;
  private stillSince: number | null = null;
  /** The frame at the start of the still time. */
  private anchor: Uint8Array | null = null;
  private state: AutoCaptureState = "waiting";

  constructor(private readonly settings: AutoCaptureSettings = AUTO_CAPTURE_DEFAULTS) {}

  /** Call with the frame that went into a photo, and its time (for the shortest time between photos). */
  taken(frame: Uint8Array, now = -Infinity): void {
    this.last = frame.slice();
    this.lastAt = now;
    this.state = "waiting";
    this.stillSince = null;
    this.anchor = null;
  }

  /** Restart a settling interval after pausing without photographing the same view again. */
  resume(): void {
    this.previous = null;
    this.stillSince = null;
    this.anchor = null;
    this.state = "waiting";
  }

  /** Forgets the last photo: the next one is taken as soon as the picture is still (at a new camera height). */
  reset(): void {
    this.last = null;
    this.previous = null;
    this.lastAt = -Infinity;
    this.state = "waiting";
    this.stillSince = null;
    this.anchor = null;
  }

  frame(frame: Uint8Array, now: number): AutoCaptureStep {
    const motion = this.previous === null ? Infinity : changedShare(this.previous, frame);
    this.previous = frame.slice();
    const sinceLast = this.last === null ? Infinity : changedShare(this.last, frame);
    if (motion > this.settings.still) {
      this.stillSince = null;
      this.anchor = null;
    } else if (this.anchor === null || changedShare(this.anchor, frame) > this.settings.still) {
      // Still from frame to frame, but drifted from where the stillness began: it begins again here.
      this.stillSince = now;
      this.anchor = frame.slice();
    }
    const settled = this.stillSince !== null && now - this.stillSince >= this.settings.settleMs;
    if (this.state === "waiting" && sinceLast > this.settings.turned) this.state = "settling";
    // A still picture that has not moved since the last photo stays waiting: the turntable has not turned yet.
    const turned = this.state === "settling" && sinceLast > this.settings.turned;
    const take = turned && settled && now - this.lastAt >= this.settings.gapMs;
    const look: AutoCaptureLook = this.stillSince === null ? "moving" : take ? "taking" : turned ? "settling" : "still";
    return { state: this.state, look, take, sinceLast: Number.isFinite(sinceLast) ? sinceLast : 0, motion: Number.isFinite(motion) ? motion : 0 };
  }
}

/**
 * The middle of a grey frame (`width` x `height`): `share` of each side, centred. Auto compares only this part, where
 * the object stands: a person passing at the edge, a hand reaching in, or the edge of the turntable do not count.
 */
export function centralRegion(data: Uint8Array, width: number, height: number, share = 0.8): Uint8Array {
  const w = Math.max(1, Math.round(width * share));
  const h = Math.max(1, Math.round(height * share));
  const x0 = Math.floor((width - w) / 2);
  const y0 = Math.floor((height - h) / 2);
  const out = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) out.set(data.subarray((y0 + y) * width + x0, (y0 + y) * width + x0 + w), y * w);
  return out;
}

/**
 * A small grey copy of RGBA pixels (`width` x `height`), `size` pixels across the longer side, by box averaging:
 * what the detector compares. Small frames ignore sensor noise and cost little on a phone.
 */
export function greyThumbnail(rgba: Uint8ClampedArray, width: number, height: number, size = 96): { data: Uint8Array; width: number; height: number } {
  const scale = Math.max(width, height) / size;
  const w = Math.max(1, Math.round(width / scale));
  const h = Math.max(1, Math.round(height / scale));
  const data = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    const y0 = Math.floor((y * height) / h);
    const y1 = Math.max(y0 + 1, Math.floor(((y + 1) * height) / h));
    for (let x = 0; x < w; x++) {
      const x0 = Math.floor((x * width) / w);
      const x1 = Math.max(x0 + 1, Math.floor(((x + 1) * width) / w));
      let sum = 0;
      let count = 0;
      for (let yy = y0; yy < y1; yy++) {
        for (let xx = x0; xx < x1; xx++) {
          const i = (yy * width + xx) * 4;
          sum += 0.299 * rgba[i]! + 0.587 * rgba[i + 1]! + 0.114 * rgba[i + 2]!;
          count++;
        }
      }
      data[y * w + x] = Math.round(sum / count);
    }
  }
  return { data, width: w, height: h };
}

/** `capture_0007.jpg`: names that sort in capture order. */
export function captureName(index: number): string {
  return `capture_${String(index + 1).padStart(4, "0")}.jpg`;
}

/** The photos of a capture in several turns (camera heights): which turn each belongs to, in capture order. */
export interface TurnPhoto {
  turn: number;
}

/**
 * The number of photos of every turn that still has photos, in order: what the engine's `--turntable-turn-sizes`
 * takes. Photos are in capture order, so the turns follow one another; a turn emptied by deleting is left out.
 */
export function turnSizes(photos: TurnPhoto[]): number[] {
  const sizes: number[] = [];
  let current: number | null = null;
  for (const photo of photos) {
    if (photo.turn !== current) {
      sizes.push(0);
      current = photo.turn;
    }
    sizes[sizes.length - 1]!++;
  }
  return sizes;
}

/** Fewest photos per turn the engine accepts with several turns (crates/dense/src/photos/options.rs). */
export const FEWEST_PER_TURN = 8;
/** Most photos per turn the engine accepts. */
export const MOST_PHOTOS = 1536;
export const MOST_PER_TURN = MOST_PHOTOS;

/** What keeps these turns from a run, as a sentence, or "" when they are fine. Only several turns are limited here. */
export function turnProblem(sizes: number[]): string {
  if (sizes.reduce((sum, n) => sum + n, 0) > MOST_PHOTOS) return t("capture.tooMany", { max: MOST_PHOTOS });
  const index = sizes.findIndex((n) => n < FEWEST_PER_TURN || n > MOST_PER_TURN);
  if (index < 0) return "";
  return t("capture.turnProblem", { turn: index + 1, count: sizes[index]!, min: FEWEST_PER_TURN, max: MOST_PER_TURN });
}

/** Copy only the file wrapper, preserving pixels and capture order after deletions. */
export function renumberPhotos(photos: { file: File }[]): File[] {
  return photos.map(({ file }, index) => new File([file], captureName(index), { type: file.type, lastModified: file.lastModified }));
}
