/** The native preview response: width, height and foreground bounds, followed by a PNG mask. */
export interface ForegroundPreview { width: number; height: number; box: [number, number, number, number]; mask: Blob; }
export interface CropSettings { padding: number; focus: number; threshold: number; }
export const CROP_DEFAULTS: CropSettings = { padding: 0.15, focus: 1, threshold: 0.5 };
export function decodeForegroundPreview(buffer: ArrayBuffer): ForegroundPreview {
  if (buffer.byteLength < 32) throw new Error("invalid-foreground-response");
  const view = new DataView(buffer);
  const [width, height, left, top, right, bottom] = Array.from({ length: 6 }, (_, i) => view.getUint32(i * 4, true)) as [number,number,number,number,number,number];
  if (!width || !height || width > 40000 || height > 40000 || left >= right || top >= bottom || right > width || bottom > height) throw new Error("invalid-foreground-response");
  const magic = new Uint8Array(buffer, 24, 8);
  if (![137,80,78,71,13,10,26,10].every((v,i)=>magic[i]===v)) throw new Error("invalid-foreground-response");
  return { width, height, box: [left, top, right, bottom], mask: new Blob([buffer.slice(24)], { type: "image/png" }) };
}
export function paddedCrop(box: [number,number,number,number], width: number, height: number, padding: number): [number,number,number,number] {
  const margin = Math.ceil(Math.max(box[2]-box[0], box[3]-box[1]) * padding);
  return [Math.max(0,box[0]-margin),Math.max(0,box[1]-margin),Math.min(width,box[2]+margin),Math.min(height,box[3]+margin)];
}
/** Evenly spaced examples, including both ends, so a change of height or distance is visible. */
export function cropSamples<T>(photos: readonly T[]): T[] {
  const indices = [...new Set([0, Math.floor((photos.length-1)/2), photos.length-1])].filter(i=>i>=0 && i<photos.length);
  return indices.map(i=>photos[i]!);
}

/** A detection area inside the saved camera frame; changing it does not crop the photos. */
export type ImageRegion = [number, number, number, number];
export function focusRegion(x: number, y: number, share = 0.35): ImageRegion {
  const left = Math.max(0, Math.min(1-share, x-share/2));
  const top = Math.max(0, Math.min(1-share, y-share/2));
  return [left, top, left+share, top+share];
}

/** Search near the last complete foreground, with room for rotation and modest camera movement. */
export function trackingRegion(previous: ForegroundPreview): ImageRegion {
  const [l,t,r,b] = paddedCrop(previous.box, previous.width, previous.height, 0.7);
  return [l/previous.width, t/previous.height, r/previous.width, b/previous.height];
}

function relativeArea(preview: ForegroundPreview): number {
  const [l,t,r,b] = preview.box;
  return (r-l)*(b-t)/(preview.width*preview.height);
}

/** A single oversized selection can be the support/table at a camera-height change.
 * Require agreement from both neighbours; sustained scale changes are left alone.
 * Retry strictly inside trusted regions, never silently fall back to the whole scene.
 * An unresolved selection is omitted; callers retain its original photo for review.
 */
export async function recoverForegroundSpikes(
  previews: readonly (ForegroundPreview | null)[],
  retry: (index: number, region: ImageRegion) => Promise<ForegroundPreview | null>,
): Promise<(ForegroundPreview | null)[]> {
  const output = [...previews];
  for (let i=1; i<previews.length-1; i++) {
    const before=previews[i-1], current=previews[i], after=previews[i+1];
    if (!before || !current || !after) continue;
    const low=Math.min(relativeArea(before),relativeArea(after));
    const high=Math.max(relativeArea(before),relativeArea(after));
    if (high/low > 2 || relativeArea(current) <= high*4) continue;
    const width=(p: ForegroundPreview)=>(p.box[2]-p.box[0])/p.width;
    const height=(p: ForegroundPreview)=>(p.box[3]-p.box[1])/p.height;
    // A rotating flat object can legitimately change width dramatically while
    // keeping its height. Only flag expansion in both dimensions.
    if (width(current)<=2*Math.max(width(before),width(after)) ||
        height(current)<=2*Math.max(height(before),height(after))) continue;
    output[i]=null;
    // Try the following frame first: it often shares the new camera height.
    for (const anchor of [after,before]) {
      const candidate=await retry(i,trackingRegion(anchor));
      if (candidate && relativeArea(candidate)>=low/2 && relativeArea(candidate)<=high*2) {
        output[i]=candidate;
        break;
      }
    }
  }
  return output;
}
