import { describe, expect, it } from "vitest";
import { fewPhotos, photosPerTurn } from "./fewPhotos";

describe("photos per turn", () => {
  it("is unknown when the count is, or below three photos", () => {
    expect(photosPerTurn(null)).toBeNull();
    expect(photosPerTurn(2)).toBeNull();
    expect(fewPhotos(null)).toBe(false);
    expect(fewPhotos(2)).toBe(false);
  });

  it("divides by the turns as the engine does, whole division, 1 to 16", () => {
    expect(photosPerTurn(20)).toBe(20);
    expect(photosPerTurn(100, "5")).toBe(20);
    expect(photosPerTurn(101, "4")).toBe(25);
    expect(photosPerTurn(60, "0")).toBe(60);
    expect(photosPerTurn(400, "40")).toBe(25);
    // Not a number of turns: one turn.
    expect(photosPerTurn(30, "two")).toBe(30);
    expect(photosPerTurn(30, true)).toBe(30);
  });

  it("is few below 25 per turn only", () => {
    expect(fewPhotos(3)).toBe(true);
    expect(fewPhotos(20)).toBe(true);
    expect(fewPhotos(24)).toBe(true);
    expect(fewPhotos(25)).toBe(false);
    expect(fewPhotos(36)).toBe(false);
    expect(fewPhotos(120, "5")).toBe(true);
    expect(fewPhotos(125, "5")).toBe(false);
  });
});

it("counts the shortest retained turn rather than an average", () => {
  expect(photosPerTurn(96, "3", "36,24,36")).toBe(24);
  expect(fewPhotos(96, "1", "36,24,36")).toBe(true);
  expect(fewPhotos(107, "1", "36,35,36")).toBe(false);
  expect(photosPerTurn(96, "3", "36,24,35")).toBe(32);
});
