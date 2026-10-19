import { beforeEach, describe, expect, it, vi } from "vitest";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", async importOriginal => ({
  ...await importOriginal<typeof import("@tauri-apps/api/core")>(), invoke,
}));
import { foregroundPreview } from "./shell";
function response(): ArrayBuffer {
  const bytes = new Uint8Array(32), view = new DataView(bytes.buffer);
  [100, 100, 20, 20, 80, 80].forEach((v, i) => view.setUint32(i * 4, v, true));
  bytes.set([137, 80, 78, 71, 13, 10, 26, 10], 24);
  return bytes.buffer;
}
const file = new File([new Uint8Array([1])], "photo.jpg");
const region: [number, number, number, number] = [.1, .1, .9, .9];
beforeEach(() => { invoke.mockReset(); });
describe("foreground recovery across arbitrary photos", () => {
  it("tries the full photo before the previous object's search area", async () => {
    invoke.mockResolvedValue(response());
    expect(await foregroundPreview(file, 1, .5, region)).not.toBeNull();
    expect(invoke.mock.calls[0]![2].headers).not.toHaveProperty("x-region");
    expect(invoke).toHaveBeenCalledTimes(1);
  });
  it("uses the tracked area when full-photo detection fails", async () => {
    invoke.mockRejectedValueOnce("foreground-not-found").mockResolvedValue(response());
    expect(await foregroundPreview(file, 1, .5, region)).not.toBeNull();
    expect(invoke.mock.calls[1]![2].headers["x-region"]).toBe(JSON.stringify(region));
  });
  it("never falls back to the table during strict spike recovery", async () => {
    invoke.mockRejectedValue("foreground-not-found");
    expect(await foregroundPreview(file, 1, .5, region, true)).toBeNull();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke.mock.calls[0]![2].headers["x-region"]).toBe(JSON.stringify(region));
  });
});
