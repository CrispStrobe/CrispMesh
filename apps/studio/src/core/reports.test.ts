import { describe, expect, it } from "vitest";
import { fixtureText } from "../testing/fixture";
import { formatBytes, formatCount, formatDuration, humanise } from "./format";
import { formatMetricValue, summariseReport } from "./reports";
import { setLanguage } from "../i18n";

const rows = (summary: ReturnType<typeof summariseReport>) =>
  Object.fromEntries(summary.sections.flatMap((section) => section.rows.map((row) => [`${section.title ?? ""}|${row.label}`, row.value])));

describe("summariseReport", () => {
  const cameras = {
    selected: "cameras-hybrid", minimum_registered_fraction: .8, minimum_sparse_mask_support: .85,
    candidates: [
      { path: "cameras", registered: 40, accepted: 78, sparse_mask_support: .912 },
      { path: "cameras-context", registered: 74, accepted: 78, sparse_mask_support: .834 },
      { path: "cameras-hybrid", registered: 74, accepted: 78, sparse_mask_support: .921, removed_weak_views: 1 },
    ],
  };

  it("explains why a high camera count alone does not pass the capture checks", () => {
    const summary = summariseReport(cameras);
    expect(summary.kind).toBe("camera_recovery");
    expect(summary.sections[0]?.rows[0]?.value).toBe("Hybrid feature overlap");
    expect(summary.sections.slice(1).map(section => section.rows.at(-1)?.value)).toEqual(["no", "no", "yes"]);
    expect(summary.sections[3]?.rows[0]?.value).toBe("74 of 78");
    expect(summary.sections[3]?.rows.find(row => row.label === "Automatically excluded weak views")?.value).toBe("1");
    expect(summary.notes[0]).toContain("do not establish geometric accuracy");
  });

  it("rejects a camera fit whose actual hull loses foreground despite sparse support", () => {
    const summary = summariseReport({ ...cameras, minimum_base_mask_iou: .8,
      candidates: [{ ...cameras.candidates[2], base_mask_iou: .773494 }] });
    expect(summary.kind).toBe("camera_recovery");
    expect(summary.sections[1]!.rows.at(-1)!.tone).toBe("bad");
    expect(summary.sections[1]!.rows.some(row => row.label === "Silhouette agreement")).toBe(true);
  });

  it("keeps malformed camera counters in the raw report", () => {
    for (const changes of [{ accepted: 0 }, { registered: 79 }, { sparse_mask_support: NaN }, { sparse_mask_support: 1.1 }]) {
      expect(summariseReport({ ...cameras, candidates: [{ ...cameras.candidates[2], ...changes }] }).kind).toBe("unknown");
    }
  });

  it("shows camera recovery in German", () => {
    setLanguage("de");
    try {
      const summary = summariseReport(cameras);
      expect(summary.sections[0]?.rows[0]?.label).toBe("Gewählter Kamerapfad");
      expect(summary.sections[3]?.rows[0]?.value).toBe("74 von 78");
      expect(summary.sections[3]?.rows.at(-1)?.value).toBe("ja");
    } finally { setLanguage("en"); }
  });

  it("summarises the fixture's mesh report", () => {
    const summary = summariseReport(JSON.parse(fixtureText("mesh/result.json")));
    expect(summary.kind).toBe("mesh");
    expect(rows(summary)).toMatchObject({
      "|Triangles": "36 168",
      "|Closed": "yes",
      "|Genus": "0",
      "|Boundary edges": "0",
      "|Non-manifold edges": "0",
    });
    expect(summary.notes).toContain("The model has no physical scale.");
  });

  it("flags an open mesh", () => {
    const summary = summariseReport({ triangles: 10, closed: false, boundary_edges: 6, nonmanifold_edges: 0, genus: 0 });
    const flat = summary.sections.flatMap((section) => section.rows);
    expect(flat.find((row) => row.label === "Closed")).toMatchObject({ value: "no", tone: "bad" });
    expect(flat.find((row) => row.label === "Boundary edges")).toMatchObject({ value: "6", tone: "bad" });
  });

  it("summarises a photo check", () => {
    const summary = summariseReport({
      triangles: 33800,
      views: 24,
      views_checked: 24,
      silhouette_iou_input_masks: { median: 0.9264, minimum: 0.91, maximum: 0.94, worst_view: "view_007" },
      note: "photo agreement only; not scanner accuracy or physical scale",
    });
    expect(summary.kind).toBe("photo_check");
    expect(rows(summary)).toMatchObject({
      "Silhouette overlap with the input masks|Median": "0.926",
      "Silhouette overlap with the input masks|Weakest view": "view_007",
      "|Views checked": "24 of 24",
    });
  });

  it("summarises a scanner evaluation with F1 at each threshold and the handedness warning", () => {
    const block = (f: number) => ({
      accuracy_mesh_to_reference: { fraction_of_diagonal: { median: 0.003, p90: 0.0133 } },
      completeness_reference_to_mesh: { fraction_of_diagonal: { median: 0.0024, p90: 0.0064 } },
      thresholds: {
        "0.02": { fraction_of_diagonal: 0.02, f1: 0.973 * f },
        "0.005": { fraction_of_diagonal: 0.005, f1: 0.755 * f },
        "0.01": { fraction_of_diagonal: 0.01, f1: 0.911 * f },
      },
    });
    const warning = "CHIRALITY: the mesh only fits the scan as its mirror image.";
    const summary = summariseReport({
      schema: "scan_evaluate_v1",
      warnings: [warning],
      notes: ["Shape-only similarity fit."],
      alignment: { handedness: { scored: "mirrored" } },
      metrics: { all: block(1), above_margin: block(1.01), fit_half_for_comparison_only: block(1) },
    });
    expect(summary.kind).toBe("scan");
    expect(summary.warnings).toEqual([warning]);
    expect(summary.sections.map((section) => section.title)).toEqual(["Whole surface", "Above the support", undefined]);
    expect(summary.sections[0]?.rows.map((row) => [row.label, row.value])).toEqual([
      ["F1 at 0.5%", "0.755"],
      ["F1 at 1%", "0.911"],
      ["F1 at 2%", "0.973"],
      ["Accuracy, median", "0.30% (90th percentile 1.33%)"],
      ["Completeness, median", "0.24% (90th percentile 0.64%)"],
    ]);
    expect(rows(summary)["|Handedness scored"]).toBe("mirrored");
  });

  it("falls back to the raw view for shapes it does not know, never throwing", () => {
    for (const odd of [null, 5, "text", [], { anything: { nested: [1, 2] } }, { metrics: { all: { thresholds: "broken" } } }, { triangles: "many", closed: "perhaps" }]) {
      expect(summariseReport(odd).kind).toBe("unknown");
    }
    expect(summariseReport({ strange: true, warnings: ["Careful"] }).warnings).toEqual(["Careful"]);
  });
});

describe("formatters", () => {
  it("formats durations, sizes and counts", () => {
    expect(formatDuration(2.02)).toBe("2.0 s");
    expect(formatDuration(53.5)).toBe("54 s");
    expect(formatDuration(63.9)).toBe("1 min 04 s");
    expect(formatDuration(3725)).toBe("1 h 02 min");
    expect(formatBytes(1690084)).toBe("1.7 MB");
    expect(formatBytes(84 + 50 * 1_000_000)).toBe("50 MB");
    expect(formatCount(1234567)).toBe("1 234 567");
    expect(humanise("coverage_level_1")).toBe("Coverage level 1");
    expect(formatMetricValue(0.9264583641452899)).toBe("0.9265");
    expect(formatMetricValue(0)).toBe("0");
    expect(formatMetricValue(true)).toBe("yes");
  });
});
