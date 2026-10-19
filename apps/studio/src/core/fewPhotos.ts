/**
 * Few photos per turn: below 25 the engine lowers its landmark gate (FEW_PHOTOS in
 * crates/dense/src/photos/options.rs), and the ordinary depth-map mesh gets rough; the
 * neural surface copes with about 20 (docs/PHOTOS-TO-INPUTS.md). The "New run" form says so
 * and keeps refinement opt-in. No DOM.
 */

/** Below this many photos per turn the ordinary mesh is rough (the engine's FEW_PHOTOS). */
export const FEW_PHOTOS = 25;

/**
 * Photos per turn, as the engine counts them (whole division by the turns, 1 to 16), or null
 * when the number of photos is not known. `turns`: the `turntable-rings` value of the form, if any.
 */
export function photosPerTurn(count: number | null, turns?: string | boolean, sizes?: string | boolean): number | null {
  if (count === null || !Number.isInteger(count) || count < 3) return null;
  if (typeof sizes === "string" && /^\s*\d+(\s*,\s*\d+)*\s*$/.test(sizes)) {
    const lengths = sizes.split(",").map(Number);
    if (lengths.length <= 16 && lengths.every((n) => n >= 8) && lengths.reduce((a, b) => a + b, 0) === count) return Math.min(...lengths);
  }
  const parsed = typeof turns === "string" && /^\s*\d+\s*$/.test(turns) ? Number(turns) : 1;
  const rings = Math.min(16, Math.max(1, parsed));
  return Math.floor(count / rings);
}

/** Known, at least three photos, and fewer than 25 per turn. */
export function fewPhotos(count: number | null, turns?: string | boolean, sizes?: string | boolean): boolean {
  const perTurn = photosPerTurn(count, turns, sizes);
  return perTurn !== null && perTurn < FEW_PHOTOS;
}
