/**
 * Few photos per turn: below 25 the engine lowers its landmark gate (FEW_PHOTOS in
 * crates/dense/src/photos/options.rs), and the ordinary depth-map mesh gets rough; the
 * neural surface copes with about 20 (docs/PHOTOS-TO-INPUTS.md). The "New run" form says so
 * and switches the neural surface on once. No DOM.
 */

/** Below this many photos per turn the ordinary mesh is rough (the engine's FEW_PHOTOS). */
export const FEW_PHOTOS = 25;

/**
 * Photos per turn, as the engine counts them (whole division by the turns, 1 to 16), or null
 * when the number of photos is not known. `turns`: the `turntable-rings` value of the form, if any.
 */
export function photosPerTurn(count: number | null, turns?: string | boolean): number | null {
  if (count === null || !Number.isInteger(count) || count < 3) return null;
  const parsed = typeof turns === "string" && /^\s*\d+\s*$/.test(turns) ? Number(turns) : 1;
  const rings = Math.min(16, Math.max(1, parsed));
  return Math.floor(count / rings);
}

/** Known, at least three photos, and fewer than 25 per turn. */
export function fewPhotos(count: number | null, turns?: string | boolean): boolean {
  const perTurn = photosPerTurn(count, turns);
  return perTurn !== null && perTurn < FEW_PHOTOS;
}

export interface SurfaceChoice {
  /** The automatic choice has been made: it is never made again, so a surface switched off by hand stays off. */
  decided: boolean;
  /** Switch the neural surface on at "fast" now. */
  switchOn: boolean;
}

/**
 * Whether to switch the neural surface on by itself. It happens at most once: the first time
 * there are few photos while the engine offers the surface, and only when it is still off.
 */
export function surfaceAutomatically(input: { few: boolean; offered: boolean; quality: string; decided: boolean }): SurfaceChoice {
  if (input.decided || !input.few || !input.offered) return { decided: input.decided, switchOn: false };
  return { decided: true, switchOn: input.quality === "" };
}
