/**
 * The lens of photos taken with *Take photos*. Frames of the camera stream carry no EXIF focal length. Where the app
 * can ask the system for the camera's field of view (the iOS app: AVFoundation), the run uses it as the lens
 * (`fov:DEG`: a centred pinhole lens of that horizontal field of view, no distortion); otherwise the focal length is
 * found from the turn (`auto`). Measured on the 3DLF objects (docs/PHOTOS-TO-INPUTS.md): a field of view within 5 %
 * of the true one does as well as the lens file, and better than `auto` where the turn misleads it (Happy Buddha).
 */

/** Size of the frames taken, when they all came from the live picture of the default camera. */
export interface CaptureFrame {
  width: number;
  height: number;
  /** Full camera frame before a centred digital crop. */
  sourceWidth?: number;
  sourceHeight?: number;
}

/** Asks the device for the horizontal field of view across frames of this size; null when it does not know. */
export type FieldOfViewSource = (width: number, height: number) => Promise<number | null>;

/** The calibration value for photos taken in the app: `fov:DEG` when the device knows its camera, else `auto`. */
export async function captureCalibration(frame: CaptureFrame | null, ask: FieldOfViewSource): Promise<string> {
  if (frame === null || frame.width <= 0 || frame.height <= 0) return "auto";
  const degrees = await ask(frame.sourceWidth ?? frame.width, frame.sourceHeight ?? frame.height).catch(() => null);
  if (typeof degrees !== "number" || !Number.isFinite(degrees) || degrees <= 1 || degrees >= 179) return "auto";
  const share = frame.width / (frame.sourceWidth ?? frame.width);
  const cropped = 2 * Math.atan(Math.tan(degrees * Math.PI / 360) * share) * 180 / Math.PI;
  return `fov:${cropped.toFixed(2)}`;
}

/** The field of view of a `fov:DEG` calibration value, else null. */
export function fieldOfViewOf(calibration: string): number | null {
  const match = /^fov:([0-9]+(?:\.[0-9]+)?)$/.exec(calibration.trim());
  if (match === null) return null;
  const degrees = Number(match[1]);
  return degrees > 1 && degrees < 179 ? degrees : null;
}
