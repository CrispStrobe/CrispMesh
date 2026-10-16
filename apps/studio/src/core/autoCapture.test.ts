import { describe, expect, it } from "vitest";
import { AutoCapture, captureName, changedShare, greyThumbnail } from "./autoCapture";

/** A 32 x 32 grey frame with a bright square at `x` (the "object"), plus a little noise. */
function frame(x: number, noise = 0, seed = 1): Uint8Array {
  const out = new Uint8Array(32 * 32).fill(100);
  let state = seed;
  for (let y = 8; y < 24; y++) for (let i = x; i < x + 8; i++) out[y * 32 + i] = 220;
  for (let i = 0; i < out.length; i++) {
    state = (state * 1103515245 + 12345) % 2147483648;
    out[i] = Math.max(0, Math.min(255, out[i]! + Math.round(((state / 2147483648) * 2 - 1) * noise)));
  }
  return out;
}

/** Feeds frames 100 ms apart; returns the times at which a photo was taken (and tells the detector). */
function run(detector: AutoCapture, frames: Uint8Array[]): number[] {
  const taken: number[] = [];
  frames.forEach((f, i) => {
    const step = detector.frame(f, i * 100);
    if (step.take) {
      taken.push(i * 100);
      detector.taken(f);
    }
  });
  return taken;
}

describe("taking turntable photos by themselves", () => {
  it("takes the first photo once the picture is still, then one per turn after it settles", () => {
    const still = (x: number, count: number) => Array.from({ length: count }, (_, i) => frame(x, 1, i + x * 100));
    const frames = [
      ...still(4, 10), // first position: one photo after 0.6 s of stillness
      frame(6), frame(8), frame(10), // turning
      ...still(12, 10), // second position
      ...still(12, 10), // nothing turns: no further photo
      frame(14), frame(16), // turning again
      ...still(18, 10),
    ];
    const taken = run(new AutoCapture(), frames);
    expect(taken).toHaveLength(3);
    // Each photo waits for 0.6 s of stillness: from the second frame (the first has nothing to compare with), and
    // after a turn from the first frame that matches its predecessor (frame 14).
    expect(taken[0]).toBe(700);
    expect(taken[1]).toBe(2000);
  });

  it("does not take a photo while the picture keeps moving, or after a jitter that comes back", () => {
    const moving = Array.from({ length: 40 }, (_, i) => frame(4 + (i % 2) * 6));
    expect(run(new AutoCapture(), moving)).toEqual([]);
    const detector = new AutoCapture();
    detector.taken(frame(4));
    // A hand passes and the same view returns: it differs from the last photo by nothing.
    const back = [frame(10), frame(16), ...Array.from({ length: 12 }, () => frame(4))];
    expect(run(detector, back)).toEqual([]);
  });

  it("compares small grey copies", () => {
    expect(changedShare(frame(4), frame(4))).toBe(0);
    // The square moved by 8 of its 8 columns: 2 x 8 x 16 of 1024 pixels changed.
    expect(changedShare(frame(4), frame(12))).toBeCloseTo(0.25);
    // Noise and a uniform brightness change below the level change nothing.
    expect(changedShare(frame(4, 3, 1), frame(4, 3, 2))).toBe(0);
    expect(changedShare(frame(4), frame(4).map((v) => v + 8))).toBe(0);
    expect(changedShare(new Uint8Array(3), new Uint8Array(4))).toBe(Infinity);
    // A red 4 x 2 RGBA picture at size 2: two grey pixels of 0.299 * 255.
    const rgba = new Uint8ClampedArray(4 * 2 * 4);
    for (let i = 0; i < rgba.length; i += 4) {
      rgba[i] = 255;
      rgba[i + 3] = 255;
    }
    const small = greyThumbnail(rgba, 4, 2, 2);
    expect([small.width, small.height]).toEqual([2, 1]);
    expect([...small.data]).toEqual([76, 76]);
    expect(captureName(0)).toBe("capture_0001.jpg");
    expect(captureName(71)).toBe("capture_0072.jpg");
  });
});
