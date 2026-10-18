import { afterEach, describe, expect, it } from "vitest";
import { centralRegion, renumberPhotos, turnProblem, turnSizes, MOST_PHOTOS } from "./autoCapture";
import { setLanguage } from "../i18n";

afterEach(() => setLanguage("en"));
describe("reviewing several turns", () => {
  it("retains unequal lengths after deletion, dropping a wholly deleted height", () => {
    expect(turnSizes([{ turn: 0 }, { turn: 0 }, { turn: 2 }])).toEqual([2, 1]);
    expect(turnProblem([36, 35, 36])).toBe("");
    expect(turnProblem([120, 180, 360])).toBe("");
    expect(turnProblem([36, 7])).toContain("Turn 2 has 7");
    expect(turnProblem([MOST_PHOTOS, 8])).toContain(String(MOST_PHOTOS));
    setLanguage("de");
    expect(turnProblem([36, 7])).toContain("Umdrehung 2 hat 7");
  });
  it("renumbers remaining files in capture order without changing their bytes", async () => {
    const files = [new File(["first"], "capture_0001.jpg", { type: "image/jpeg" }), new File(["third"], "capture_0003.jpg", { type: "image/jpeg" })];
    const result = renumberPhotos(files.map((file) => ({ file })));
    expect(result.map((f) => f.name)).toEqual(["capture_0001.jpg", "capture_0002.jpg"]);
    expect(await Promise.all(result.map((f) => f.text()))).toEqual(["first", "third"]);
  });
  it("ignores the outside of the detector thumbnail", () => {
    const data = new Uint8Array(100).fill(1);
    data.fill(2, 40, 60);
    expect(centralRegion(data, 10, 10, 0.2)).toEqual(new Uint8Array([2, 2, 2, 2]));
  });
});
