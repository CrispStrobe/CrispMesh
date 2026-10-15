import { describe, expect, it } from "vitest";
import { event, fixtureEvents, fixtureText } from "../testing/fixture";
import { parseEventLines } from "./events";
import {
  groupSheets,
  initialState,
  overallFraction,
  reduce,
  reduceAll,
  safeRelativePath,
  stageElapsed,
  stlBytes,
} from "./reducer";

describe("parseEventLines", () => {
  it("reads the fixture log completely and numbers the events by line", () => {
    const { events, consumed } = parseEventLines(fixtureText("events.jsonl"));
    const lines = fixtureText("events.jsonl").trimEnd().split("\n").length;
    expect(lines).toBeGreaterThan(30);
    expect(events).toHaveLength(lines);
    expect(consumed).toBe(lines);
    expect(events[0]).toMatchObject({ type: "run_started", seq: 0, stage: null });
    expect(events.at(-1)).toMatchObject({ type: "run_finished", seq: lines - 1, status: "complete" });
  });

  it("ignores a half-written last line and picks it up once it is complete", () => {
    const first = '{"type":"stage_started","time":1,"stage":"stereo"}\n{"type":"progress","time":2,"stage":"ste';
    const partial = parseEventLines(first);
    expect(partial.events.map((e) => e.type)).toEqual(["stage_started"]);
    expect(partial.consumed).toBe(1);

    // Even a syntactically complete object is not consumed until its newline is there.
    const whole = '{"type":"stage_started","time":1,"stage":"stereo"}\n{"type":"progress","time":2,"stage":"stereo"}';
    expect(parseEventLines(whole).consumed).toBe(1);
    expect(parseEventLines(whole + "\n").consumed).toBe(2);
  });

  it("continues numbering from the reader's position", () => {
    const tail = parseEventLines('{"type":"metric","time":3,"stage":"check","name":"x","value":1}\n', 17);
    expect(tail.events[0]?.seq).toBe(17);
    expect(tail.consumed).toBe(1);
  });

  it("stops at a complete line that is not an event, like the reference reader", () => {
    const result = parseEventLines('{"type":"a","time":1}\nnot json\n{"type":"b","time":2}\n');
    expect(result.events.map((e) => e.type)).toEqual(["a"]);
    expect(result.consumed).toBe(1);
  });

  it("returns nothing for an empty file or an HTML error page", () => {
    expect(parseEventLines("").events).toEqual([]);
    expect(parseEventLines("<!doctype html>\n<html></html>\n").events).toEqual([]);
  });

  it("fills in missing time and stage instead of failing", () => {
    const { events } = parseEventLines('{"type":"later_invention"}\n');
    expect(events[0]).toMatchObject({ type: "later_invention", time: 0, stage: null, seq: 0 });
  });
});

describe("reduce on the recorded sphere run", () => {
  const events = fixtureEvents();
  const final = reduceAll(events);

  it("ends complete with the reported duration", () => {
    expect(final.status).toBe("complete");
    expect(final.seconds).toBe(events.at(-1)!.seconds);
    expect(final.seconds).toBeGreaterThan(0.5);
    expect(final.device).toMatch(/^wgpu/);
    expect(final.schemaUnknown).toBe(false);
    expect(final.eventCount).toBe(events.length);
    expect(final.lastSeq).toBe(events.length - 1);
    expect(final.configuration?.grid).toBe(96);
  });

  it("tracks every stage: inputs done without a start event, evaluate never run", () => {
    const byName = Object.fromEntries(final.stages.map((stage) => [stage.name, stage]));
    expect(final.stages.map((stage) => stage.name)).toEqual(["inputs", "stereo", "mesh", "check", "evaluate"]);
    expect(byName.inputs?.status).toBe("done");
    expect(byName.stereo).toMatchObject({ status: "done", fraction: 1, message: "Depth fused" });
    expect(byName.stereo?.seconds).toBe(events.find((e) => e.type === "stage_finished" && e.stage === "stereo")!.seconds);
    expect(byName.mesh?.status).toBe("done");
    expect(byName.check).toMatchObject({ status: "done", message: "Silhouettes compared" });
    expect(byName.evaluate?.status).toBe("skipped");
  });

  it("lists the surfaces in arrival order with their labels", () => {
    expect(final.meshes.map((mesh) => mesh.label)).toEqual([
      "Silhouette hull",
      "Silhouette hull after mask repair",
      "Surface after level 1",
      "Final surface",
    ]);
    expect(final.meshes.map((mesh) => mesh.kind)).toEqual(["preview_mesh", "preview_mesh", "preview_mesh", "final_mesh"]);
    expect(final.meshes[3]).toMatchObject({ path: "mesh/mesh.stl", triangles: 36168 });
  });

  it("groups sheets by stage and keeps depth sheets in level order", () => {
    const groups = groupSheets(final);
    expect(groups.map((group) => group.stage)).toEqual(["inputs", "stereo", "check"]);
    expect(groups[1]?.sheets.map((sheet) => sheet.path)).toEqual([
      "stereo/mask-repair.png",
      "stereo/hull-vs-mask.png",
      "stereo/depth-level-0.png",
      "stereo/depth-level-1.png",
      "stereo/depth-merged.png",
    ]);
    expect(groups[2]?.sheets.map((sheet) => sheet.kind)).toEqual(["photo_overlay", "preview_render"]);
  });

  it("keeps metrics and the report reference", () => {
    expect(final.metrics.map((metric) => metric.name)).toEqual([
      "mask_added_fraction_median",
      "coverage_level_0",
      "coverage_level_1",
      "silhouette_iou_median",
    ]);
    expect(final.reports.map((report) => [report.path, report.label, report.stage])).toEqual([
      ["check/result.json", "Photo check", "check"],
      ["mesh/result.json", "Mesh report", "mesh"],
    ]);
  });

  it("shows a running stage half way through", () => {
    const middle = reduceAll(events.slice(0, 12));
    expect(middle.status).toBe("running");
    const stereo = middle.stages.find((stage) => stage.name === "stereo")!;
    expect(stereo).toMatchObject({ status: "running", fraction: 0.43, message: "Matching level 1 of 2, view 20 of 24" });
    expect(stageElapsed(stereo, stereo.startedAt! + 30)).toBe(30);
    expect(middle.meshes).toHaveLength(2);
    expect(overallFraction(middle)).toBeGreaterThan(0.2);
    expect(overallFraction(middle)).toBeLessThan(0.5);
  });

  it("is pure: the same events give the same state and inputs are not mutated", () => {
    const before = initialState();
    const frozen = JSON.stringify(before);
    const once = reduceAll(events, before);
    expect(JSON.stringify(before)).toBe(frozen);
    expect(reduceAll(events)).toEqual(once);
  });
});

describe("reduce on things the contract allows to change", () => {
  it("ignores unknown event types, artifact kinds and fields", () => {
    const started = reduce(initialState(), event("run_started", { schema: "crisp3ds_dense_events_v1", brand_new_field: [1, 2] }));
    let state = started;
    state = reduce(state, event("telemetry_burst", { stage: "stereo", payload: { deep: true } }, 1));
    state = reduce(state, event("artifact", { stage: "stereo", kind: "preview_volume", path: "stereo/preview/00-hull.npz", label: "x" }, 2));
    state = reduce(state, event("artifact", { stage: "stereo", kind: "glb_preview", path: "stereo/p.glb", label: "Compact preview" }, 3));
    state = reduce(state, event("artifact", { stage: "stereo", kind: 7, path: "a.png" }, 4));
    state = reduce(state, event("artifact", { stage: "stereo", kind: "depth_sheet" }, 5));
    state = reduce(state, event("metric", { stage: "stereo", name: "odd", value: { nested: 1 } }, 6));
    state = reduce(state, event("progress", { stage: "stereo", fraction: "soon", message: 12 }, 7));
    expect(state.meshes).toEqual([]);
    expect(state.sheets).toEqual([]);
    expect(state.metrics).toEqual([]);
    expect(state.errors).toEqual([]);
    expect(state.status).toBe("running");
    expect(state.eventCount).toBe(8);
    expect(state.ignored).toBe(6);
    expect(state.stages.find((stage) => stage.name === "stereo")).toMatchObject({ status: "running", fraction: 0, message: "" });
  });

  it("accepts extra fields on known events", () => {
    const state = reduce(
      initialState(),
      event("artifact", { stage: "stereo", kind: "depth_sheet", path: "stereo/d.png", label: "Depth", level: 2, views: [1, 2, 3], colour_map: "turbo" }),
    );
    expect(state.sheets[0]).toMatchObject({ kind: "depth_sheet", level: 2, label: "Depth" });
  });

  it("flags an unknown schema but carries on", () => {
    const state = reduce(initialState(), event("run_started", { schema: "crisp3ds_dense_events_v2" }));
    expect(state.schemaUnknown).toBe(true);
    expect(state.status).toBe("running");
  });

  it("adds a stage it has never heard of", () => {
    let state = reduce(initialState(), event("stage_started", { stage: "texture" }));
    state = reduce(state, event("progress", { stage: "texture", fraction: 0.5, message: "Baking" }, 1));
    expect(state.stages.map((stage) => stage.name)).toEqual(["inputs", "stereo", "mesh", "check", "evaluate", "texture"]);
    expect(state.stages[5]).toMatchObject({ status: "running", fraction: 0.5, message: "Baking" });
  });

  it("clamps progress to 0..1", () => {
    let state = reduce(initialState(), event("progress", { stage: "stereo", fraction: 7 }));
    expect(state.stages[1]?.fraction).toBe(1);
    state = reduce(state, event("progress", { stage: "stereo", fraction: -3 }, 1));
    expect(state.stages[1]?.fraction).toBe(0);
  });

  it("lets the latest value of a metric win and keeps first-seen order", () => {
    let state = reduce(initialState(), event("metric", { stage: "stereo", name: "a", value: 1 }));
    state = reduce(state, event("metric", { stage: "stereo", name: "b", value: 2 }, 1));
    state = reduce(state, event("metric", { stage: "check", name: "a", value: 3 }, 2));
    expect(state.metrics.map((metric) => [metric.name, metric.value])).toEqual([
      ["a", 3],
      ["b", 2],
    ]);
  });

  it("orders surfaces by seq even when a side process delivers one late", () => {
    let state = reduce(initialState(), event("artifact", { kind: "preview_mesh", path: "p/10/mesh.stl", label: "Level 1", stage: "stereo" }, 9));
    state = reduce(state, event("artifact", { kind: "preview_mesh", path: "p/00/mesh.stl", label: "Hull", stage: "stereo" }, 4));
    expect(state.meshes.map((mesh) => mesh.label)).toEqual(["Hull", "Level 1"]);
  });

  it("replaces an artifact announced again for the same path", () => {
    let state = reduce(initialState(), event("artifact", { kind: "final_mesh", path: "mesh/mesh.stl", label: "Final", triangles: 10 }, 1));
    state = reduce(state, event("artifact", { kind: "final_mesh", path: "mesh/mesh.stl", label: "Final", triangles: 20 }, 2));
    expect(state.meshes).toHaveLength(1);
    expect(state.meshes[0]?.triangles).toBe(20);
  });

  it("refuses artifact paths that leave the run", () => {
    for (const path of ["../secret.png", "/etc/passwd", "a/../../b.png", "http://elsewhere/x.png", "a\\b.png", "a//b.png"]) {
      expect(safeRelativePath(path)).toBe(false);
      const state = reduce(initialState(), event("artifact", { kind: "input_sheet", path, label: "x" }));
      expect(state.sheets).toEqual([]);
    }
    expect(safeRelativePath("stereo/preview/00-hull/mesh.stl")).toBe(true);
  });

  it("records a failed stage with its message", () => {
    let state = reduce(initialState(), event("run_started"));
    state = reduce(state, event("stage_started", { stage: "stereo" }, 1));
    state = reduce(state, event("error", { stage: "stereo", message: "exit code 1: MPS is not available" }, 2));
    state = reduce(state, event("run_finished", { status: "failed", seconds: 4 }, 3));
    expect(state.status).toBe("failed");
    expect(state.errors).toEqual([{ seq: 2, stage: "stereo", message: "exit code 1: MPS is not available" }]);
    expect(state.stages.map((stage) => stage.status)).toEqual(["skipped", "failed", "skipped", "skipped", "skipped"]);
  });

  it("shows a cancelled run as cancelled, not failed", () => {
    let state = reduce(initialState(), event("run_started"));
    state = reduce(state, event("stage_started", { stage: "stereo" }, 1));
    state = reduce(state, event("error", { stage: "stereo", message: "cancelled: ..." }, 2));
    state = reduce(state, event("run_finished", { status: "cancelled", seconds: 4 }, 3));
    expect(state.status).toBe("cancelled");
    expect(state.stages[1]?.status).toBe("cancelled");
  });

  it("treats an unknown final status as failed rather than complete", () => {
    const state = reduce(reduce(initialState(), event("run_started")), event("run_finished", { status: "exploded" }, 1));
    expect(state.status).toBe("failed");
  });
});

describe("selectors", () => {
  it("knows the size of a binary STL from its triangle count", () => {
    expect(stlBytes(33800)).toBe(1690084);
    expect(stlBytes(undefined)).toBeUndefined();
  });

  it("freezes the elapsed time of a stage that stopped without finishing", () => {
    let state = reduce(initialState(), { type: "stage_started", seq: 0, time: 100, stage: "stereo" });
    state = reduce(state, { type: "error", seq: 1, time: 130, stage: "stereo", message: "x" });
    expect(stageElapsed(state.stages[1]!, 9999)).toBe(30);
  });
});


describe("textured export", () => {
  it("offers GLB separately from STL geometry and rejects escaping paths", () => {
    let state = reduce(initialState(), event("artifact", { stage: "texture", kind: "textured_mesh", path: "texture/mesh.glb", label: "Textured mesh" }));
    expect(state.downloads.map((file) => file.path)).toEqual(["texture/mesh.glb"]);
    expect(state.meshes).toEqual([]);
    state = reduce(state, event("artifact", { stage: "texture", kind: "textured_mesh", path: "../escape.glb" }, 1));
    expect(state.downloads).toHaveLength(1);
  });
});

describe("surfaces optimised against the photos", () => {
  it("shows the neural surface and the splats as further finished meshes with their labels", () => {
    let state = reduce(initialState(), event("artifact", { stage: "surface", kind: "neural_surface", path: "surface/mesh.stl", label: "Neural surface (experimental)" }));
    state = reduce(state, event("artifact", { stage: "splats", kind: "splats", path: "splats/mesh.stl", label: "2D splats (experimental)" }, 1));
    expect(state.meshes.map((mesh) => [mesh.kind, mesh.path, mesh.label])).toEqual([
      ["final_mesh", "surface/mesh.stl", "Neural surface (experimental)"],
      ["final_mesh", "splats/mesh.stl", "2D splats (experimental)"],
    ]);
  });
});
