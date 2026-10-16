/**
 * When to take the next photo of a turntable capture by itself: after the object has turned (the picture differs
 * enough from the last photo) and then stands still (consecutive frames agree) for a moment. Works on small grey
 * frames (see `greyThumbnail`); knows nothing about cameras or the DOM, so it is tested with synthetic frames.
 */

export interface AutoCaptureSettings {
  /** Share of pixels changed from the last photo above which the object counts as turned. */
  turned: number;
  /** Share of pixels changed between consecutive frames below which the picture counts as still. */
  still: number;
  /** How long the picture must stay still before the photo is taken, in milliseconds. */
  settleMs: number;
}

/**
 * Measured on the 3DLF Bunny and Dragon photos (96-pixel grey copies, change above `CHANGED_LEVEL`): a 10 degree step
 * changes at least 2.9 % of the pixels (5 degrees: median 2.7 %); strong sensor noise and a brightness shift of 8 grey
 * levels change none. The mean difference does not separate them: the backdrop dilutes it.
 */
export const AUTO_CAPTURE_DEFAULTS: AutoCaptureSettings = { turned: 0.015, still: 0.003, settleMs: 600 };

/** Grey levels by which a pixel of the small copy must change to count. */
export const CHANGED_LEVEL = 12;

export type AutoCaptureState =
  /** Nothing taken yet, or waiting for the object to turn from the last photo. */
  | "waiting"
  /** Turned far enough; waiting for the picture to stand still. */
  | "settling";

export interface AutoCaptureStep {
  state: AutoCaptureState;
  /** True when the photo should be taken now. */
  take: boolean;
  /** Share of pixels changed since the last photo, for display. */
  sinceLast: number;
  /** Share of pixels changed since the previous frame, for display. */
  motion: number;
}

/** Share of the pixels of two grey frames of equal size that differ by more than `CHANGED_LEVEL`. */
export function changedShare(a: Uint8Array, b: Uint8Array): number {
  if (a.length !== b.length || a.length === 0) return Infinity;
  let changed = 0;
  for (let i = 0; i < a.length; i++) if (Math.abs(a[i]! - b[i]!) > CHANGED_LEVEL) changed++;
  return changed / a.length;
}

/**
 * The detector: feed it every frame with its time. The first photo is taken as soon as the picture is still; every
 * later one once the object has turned and come to rest again. `taken()` resets it after any photo, also one taken
 * by hand.
 */
export class AutoCapture {
  private last: Uint8Array | null = null;
  private previous: Uint8Array | null = null;
  private stillSince: number | null = null;
  private state: AutoCaptureState = "waiting";

  constructor(private readonly settings: AutoCaptureSettings = AUTO_CAPTURE_DEFAULTS) {}

  /** Call with the frame that went into a photo. */
  taken(frame: Uint8Array): void {
    this.last = frame;
    this.state = "waiting";
    this.stillSince = null;
  }

  frame(frame: Uint8Array, now: number): AutoCaptureStep {
    const motion = this.previous === null ? Infinity : changedShare(frame, this.previous);
    this.previous = frame;
    const sinceLast = this.last === null ? Infinity : changedShare(frame, this.last);
    if (motion > this.settings.still) this.stillSince = null;
    else if (this.stillSince === null) this.stillSince = now;
    const settled = this.stillSince !== null && now - this.stillSince >= this.settings.settleMs;
    if (this.state === "waiting" && sinceLast > this.settings.turned) this.state = "settling";
    // A still picture that has not moved since the last photo stays waiting: the turntable has not turned yet.
    const take = this.state === "settling" && settled && sinceLast > this.settings.turned;
    return { state: this.state, take, sinceLast: Number.isFinite(sinceLast) ? sinceLast : 0, motion: Number.isFinite(motion) ? motion : 0 };
  }
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
