import { describe, expect, it } from "vitest";
import { commonFrame } from "../ui/capture";
import { captureCalibration, fieldOfViewOf } from "./captureLens";

describe("the lens of photos taken in the app", () => {
  it("uses the device's field of view when it knows one", async () => {
    const asked: number[][] = [];
    const ask = async (width: number, height: number) => {
      asked.push([width, height]);
      return 69.4123;
    };
    expect(await captureCalibration({ width: 4032, height: 3024 }, ask)).toBe("fov:69.41");
    expect(asked).toEqual([[4032, 3024]]);
  });

  it("falls back to finding the focal length from the turn", async () => {
    expect(await captureCalibration({ width: 1920, height: 1080 }, async () => null)).toBe("auto");
    expect(await captureCalibration({ width: 1920, height: 1080 }, async () => 0)).toBe("auto");
    expect(await captureCalibration({ width: 1920, height: 1080 }, async () => Number.NaN)).toBe("auto");
    expect(await captureCalibration({ width: 1920, height: 1080 }, () => Promise.reject(new Error("no shell")))).toBe("auto");
    // Not every photo from the live default camera (system camera, another camera chosen): not asked.
    let asked = false;
    expect(
      await captureCalibration(null, async () => {
        asked = true;
        return 60;
      }),
    ).toBe("auto");
    expect(asked).toBe(false);
  });

  it("asks only when every photo is a frame of the same live picture", () => {
    const frame = (width: number, height: number) => ({ frame: { width, height } });
    expect(commonFrame([frame(4032, 3024), frame(4032, 3024)])).toEqual({ width: 4032, height: 3024 });
    expect(commonFrame([frame(4032, 3024), frame(3024, 4032)])).toBeNull();
    expect(commonFrame([frame(4032, 3024), { frame: null }])).toBeNull();
    expect(commonFrame([])).toBeNull();
  });

  it("reads the field of view back from the calibration value", () => {
    expect(fieldOfViewOf("fov:69.41")).toBe(69.41);
    expect(fieldOfViewOf("fov:60")).toBe(60);
    expect(fieldOfViewOf("auto")).toBeNull();
    expect(fieldOfViewOf("calibrations/lens.json")).toBeNull();
    expect(fieldOfViewOf("fov:180")).toBeNull();
    expect(fieldOfViewOf("fov:wide")).toBeNull();
  });
});

it("corrects the field of view for a centred zoom crop", async () => {
  const calls: number[][] = [];
  const value = await captureCalibration({ width: 1632, height: 1224, sourceWidth: 3264, sourceHeight: 2448 }, async (w, h) => { calls.push([w,h]); return 60; });
  expect(calls).toEqual([[3264, 2448]]);
  expect(fieldOfViewOf(value)).toBeCloseTo(32.20, 2);
  expect(commonFrame([{ frame: { width: 100, height: 100, sourceWidth: 200, sourceHeight: 200 } }, { frame: { width: 100, height: 100, sourceWidth: 400, sourceHeight: 400 } }])).toBeNull();
});
