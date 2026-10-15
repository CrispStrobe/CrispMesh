import { describe, expect, it } from "vitest";
import type { FromWorker, ToWorker, WorkerLike } from "../browser/protocol";
import { optionErrors, optionTokens, parseOptionSpecs } from "../core/providerOptions";
import { initialState, reduce } from "../core/reducer";
import { activeOptions, chosenProviders, importFields, missingFields, parseStartPoints, startBody, startOptionErrors } from "../core/startPoints";
import { event, fixtureEvents, settle } from "../testing/fixture";
import { downloadName } from "../ui/download";
import { servedByEngine } from "../ui/served";
import { BrowserEngine, browserStartPoints, checkCalibration, describeInputs, photoFiles, PREVIEW_LIMIT_MEGAPIXELS, previewAdvice, relativeFiles } from "./browserEngine";
import { RunStore } from "./runStore";

/** A stand-in for the worker: records what it is sent and lets the test answer. */
class FakeWorker implements WorkerLike {
  onmessage: ((event: { data: FromWorker }) => void) | null = null;
  sent: ToWorker[] = [];
  terminated = false;
  constructor(private readonly webgpu = true) {}
  postMessage(message: ToWorker): void {
    this.sent.push(message);
    if (message.type === "load") {
      queueMicrotask(() =>
        this.answer(
          this.webgpu
            ? { type: "loaded", ok: true, settings: { settings: [{ name: "grid", group: "Hull", meaning: "m", kind: "integer", default: 400 }] }, adapter: "apple metal-3" }
            : { type: "loaded", ok: false, message: "This browser has no WebGPU, which the engine computes with." },
        ),
      );
    }
    if (message.type === "file") {
      const bytes = message.path === "mesh/mesh.stl" ? new Uint8Array([1, 2, 3]).buffer : message.path === "mesh/result.json" ? new TextEncoder().encode('{"closed":true}').buffer : null;
      queueMicrotask(() => this.answer({ type: "file", request: message.request, bytes: bytes as ArrayBuffer | null }));
    }
  }
  answer(message: FromWorker): void {
    this.onmessage?.({ data: message });
  }
  terminate(): void {
    this.terminated = true;
  }
}

function engineWith(worker: FakeWorker) {
  const revoked: string[] = [];
  let made = 0;
  const engine = new BrowserEngine({
    module: "https://host/app/engine/crispmesh-dense.js",
    createWorker: () => worker,
    clock: () => 1000,
    objectUrls: { create: () => `blob:${++made}`, revoke: (url) => revoked.push(url) },
  });
  return { engine, revoked };
}

const file = (name: string) => ({ name }) as unknown as File;
const picked = (megapixels = 2 * 24) => ({ name: "sphere", files: [["cameras.json", file("cameras.json")]] as [string, File][], views: 24, megapixels });

describe("the engine in this browser", () => {
  it("loads the package from next to the app and reports the adapter and the settings", async () => {
    const worker = new FakeWorker();
    const { engine } = engineWith(worker);
    const health = await engine.health();
    expect(worker.sent[0]).toEqual({ type: "load", module: "https://host/app/engine/crispmesh-dense.js" });
    expect(health).toMatchObject({ device: "WebGPU: apple metal-3 · single-threaded", canStartRuns: true, choosesDevice: false, scoresReference: false });
    expect(health.startPoints?.map((point) => point.id)).toEqual(["inputs", "photos"]);
    expect((await engine.settings())[0]?.name).toBe("grid");
    expect(worker.sent).toHaveLength(1); // loaded once
    expect(engine.kind).toBe("browser");
  });

  it("says so plainly when the browser has no WebGPU", async () => {
    const { engine } = engineWith(new FakeWorker(false));
    await expect(engine.health()).rejects.toThrow(/no WebGPU/);
    await expect(engine.startRun({})).rejects.toThrow(/no WebGPU/);
  });

  it("runs on the picked files, feeds the reducer, serves files from memory and revokes its object URLs", async () => {
    const worker = new FakeWorker();
    const { engine, revoked } = engineWith(worker);
    await expect(engine.startRun({})).rejects.toMatchObject({ status: 400, message: expect.stringContaining("choose the inputs folder") });
    engine.setInputs(picked());
    const id = await engine.startRun({ settings: { grid: 96 }, texture: true, name: "my sphere!" });
    expect(id).toBe("browser-01-my-sphere-");
    const sent = worker.sent.at(-1) as Extract<ToWorker, { type: "run" }>;
    expect(sent).toMatchObject({ type: "run", id, options: { live_previews: true, texture: true, settings: { grid: 96 } } });
    expect(sent.files.map(([path]) => path)).toEqual(["cameras.json"]);
    await expect(engine.startRun({})).rejects.toThrow(/already in progress/);

    const source = engine.openRun(id);
    const store = new RunStore(source);
    source.start();
    const events = fixtureEvents();
    for (const recorded of events.slice(0, 6)) worker.answer({ type: "event", id, event: recorded, wasmBytes: 40_000_000 });
    expect(store.get().run.status).toBe("running");
    expect(store.get().run.meshes.map((mesh) => mesh.label)).toEqual(["Silhouette hull"]);
    expect((await engine.listRuns())[0]).toMatchObject({ id, status: "running", stage: "stereo", events: 6 });

    for (const recorded of events.slice(6)) worker.answer({ type: "event", id, event: recorded, wasmBytes: 47_000_000 });
    worker.answer({ type: "finished", id, ok: true, seconds: 12, peakWasmBytes: 49_000_000 });
    expect(store.get().run.status).toBe("complete");
    expect(store.get().run.eventCount).toBe(events.length);
    expect(store.get().link.state).toBe("ended");
    expect(engine.peakMemory(id)).toBe(49_000_000);
    expect((await engine.listRuns())[0]).toMatchObject({ status: "complete", stage: null });

    expect(new Uint8Array(await source.fetchBytes("mesh/mesh.stl"))).toEqual(new Uint8Array([1, 2, 3]));
    expect(await source.fetchJson("mesh/result.json")).toEqual({ closed: true });
    await expect(source.fetchBytes("nope.png")).rejects.toMatchObject({ status: 404 });
    expect(await source.imageUrl("mesh/mesh.stl")).toBe("blob:1");
    expect(await source.imageUrl("mesh/mesh.stl")).toBe("blob:1");
    source.stop();
    expect(revoked).toEqual(["blob:1"]);

    // A second view of the same run gets every event again, from memory.
    const again = engine.openRun(id);
    const second = new RunStore(again);
    again.start();
    expect(second.get().run.eventCount).toBe(events.length);

    // Removing a finished run frees it in the worker; a new run may then start.
    engine.remove(id);
    expect(worker.sent.at(-1)).toEqual({ type: "dispose", id });
    expect(await engine.listRuns()).toEqual([]);
    const gone = engine.openRun(id);
    const links: string[] = [];
    gone.subscribe((update) => update.type === "link" && links.push(update.link.state));
    gone.start();
    expect(links.at(-1)).toBe("failed");
  });

  it("cancels through the worker and closes a run the engine could not even start", async () => {
    const worker = new FakeWorker();
    const { engine } = engineWith(worker);
    engine.setInputs(picked(), false);
    const id = await engine.startRun({});
    expect((worker.sent.at(-1) as Extract<ToWorker, { type: "run" }>).options).toEqual({ live_previews: false });
    const source = engine.openRun(id);
    const store = new RunStore(source);
    source.start();
    await source.cancel!();
    expect(worker.sent.at(-1)).toEqual({ type: "cancel", id });
    worker.answer({ type: "finished", id, ok: false, message: "out of memory", seconds: 3, peakWasmBytes: 1 });
    await settle();
    expect(store.get().run.status).toBe("failed");
    expect(store.get().run.errors.map((error) => error.message)).toEqual(["out of memory"]);
    // The next run may start now.
    expect(await engine.startRun({})).toBe("browser-02-sphere");
  });

  it("turns intermediate surfaces off above the memory threshold and says why", () => {
    expect(previewAdvice(24, 0.4)).toMatchObject({ on: true });
    expect(previewAdvice(30, PREVIEW_LIMIT_MEGAPIXELS).on).toBe(true);
    const bunny = previewAdvice(73, 146);
    expect(bunny.on).toBe(false);
    expect(bunny.reason).toContain("73 photos, 146 megapixels");
    expect(bunny.reason).toContain("4 GiB");
  });

  it("reads a picked folder: paths inside it, hidden files dropped, size from cameras.json", () => {
    const entries = ["bunny/cameras.json", "bunny/images/b.png", "bunny/images/a.png", "bunny/.DS_Store", "bunny/masks/.hidden/x.png", "bunny"].map((path) => ({ path, file: file(path) }));
    const files = relativeFiles(entries);
    expect(files.map(([path]) => path)).toEqual(["cameras.json", "images/a.png", "images/b.png"]);
    const cameras = JSON.stringify({ views: [{ width: 1600, height: 1250 }, { width: 1600, height: 1250 }, { name: "no size" }] });
    expect(describeInputs("bunny", files, cameras)).toMatchObject({ name: "bunny", views: 3, megapixels: 4 });
    // Photos and masks the cameras name must be in the folder: a browser reads nothing else.
    const named = (image: string) => JSON.stringify({ views: [{ width: 10, height: 10, image, mask: "images/b.png" }] });
    expect(describeInputs("bunny", files, named("./images/a.png")).views).toBe(1);
    expect(() => describeInputs("bunny", files, named("images/missing.png"))).toThrow(/names 1 file that is not in this folder \(the first: images\/missing\.png\)/);
    expect(() => describeInputs("bunny", files, named("/disk/prepared/1.png"))).toThrow(/full paths elsewhere on a disk/);
    expect(() => describeInputs("bunny", files, JSON.stringify({ views: [{ image: "images/a.png", texture_image: "originals/a.png" }] }))).toThrow(/originals\/a\.png/);
    expect(() => describeInputs("x", files, "{}")).toThrow(/not an inputs folder/);
    expect(() => describeInputs("x", files, "not json")).toThrow(/not an inputs folder/);
  });

  it("starts from photos: picked images in capture order, a calibration, threshold and turntable, no previews by default", async () => {
    const worker = new FakeWorker();
    const shipped = JSON.stringify({ schema: "crispmesh_lens_calibration_v1", fx: 776 });
    const engine = new BrowserEngine({
      module: "https://host/app/engine/crispmesh-dense.js",
      createWorker: () => worker,
      clock: () => 1000,
      base: "https://host/app/",
      fetcher: async (url) =>
        url.endsWith("index.json")
          ? new Response(JSON.stringify([{ file: "3dlf-pro.json", label: "3dlf-pro" }]))
          : url.endsWith("calibrations/3dlf-pro.json")
            ? new Response(shipped)
            : new Response("", { status: 404 }),
    });
    const health = await engine.health();
    const photosPoint = health.startPoints!.find((point) => point.id === "photos")!;
    expect(photosPoint.fields.map((field) => field.key)).toEqual(["photos", "calibration"]);
    expect(photosPoint.providers.map((choice) => [choice.module, choice.default, choice.options.filter((o) => o.available).map((o) => o.id)])).toEqual([
      ["masks", "threshold", ["threshold", "background"]],
      ["cameras", "turntable", ["turntable"]],
    ]);
    await expect(engine.startRun({ photos: "x" })).rejects.toThrow(/choose the photos/);
    const named = (name: string) => ({ name, file: file(name) });
    const photos = photoFiles(["capture_10.png", "capture_2.png", ".DS_Store", "notes.txt", "capture_1.jpg", "capture_2.png"].map(named));
    expect(photos.map(([name]) => name)).toEqual(["capture_1.jpg", "capture_2.png", "capture_10.png"]);
    engine.setPhotos({ name: "bunny", files: photos });
    await expect(engine.startRun({ photos: "bunny" })).rejects.toThrow(/lens calibration/);
    expect(await engine.calibrations()).toEqual([{ label: "3dlf-pro", path: "calibrations/3dlf-pro.json" }]);
    await engine.useShippedCalibration("calibrations/3dlf-pro.json");
    await expect(engine.startRun({ photos: "bunny", providers: { masks: "external-sam", cameras: "turntable" } })).rejects.toThrow(/external-sam/);
    const id = await engine.startRun({ photos: "bunny", calibration: "3dlf-pro.json", providers: { masks: "threshold", cameras: "turntable" }, settings: { grid: 320 } });
    const sent = worker.sent.at(-1) as Extract<ToWorker, { type: "run-photos" }>;
    expect(sent).toMatchObject({ type: "run-photos", id, calibration: shipped, options: { live_previews: false, photo_options: ["--masks", "threshold", "--cameras", "turntable"], settings: { grid: 320 } } });
    expect(sent.photos.map(([name]) => name)).toEqual(["capture_1.jpg", "capture_2.png", "capture_10.png"]);
    // A refusal from the engine (photos out of order, say) ends the run with its sentence.
    worker.answer({ type: "finished", id, ok: false, message: "the photos are not in capture order", seconds: 1, peakWasmBytes: 1 });
    await settle();
    const source = engine.openRun(id);
    const store = new RunStore(source);
    source.start();
    expect(store.get().run.status).toBe("failed");
    expect(store.get().run.errors.map((error) => error.message)).toEqual(["the photos are not in capture order"]);
    expect(() => checkCalibration("{}")).toThrow(/not a lens calibration/);
    expect(() => checkCalibration("nope")).toThrow(/not JSON/);
  });

  it("builds its form from the package's describe(): browser providers, path-taking ones explained, no places", () => {
    const setting = (name: string, kind: string, fallback: unknown) => ({ name, flag: `--${name}`, kind, default: fallback, choices: [], repeated: false, meaning: name });
    const described = {
      start_points: [
        { id: "inputs", label: "Inputs", fields: [{ key: "inputs", kind: "inputs" }], providers: [] },
        { id: "scene", label: "Scene", fields: [{ key: "scene", kind: "file" }], providers: [] },
        {
          id: "photos",
          label: "Turntable photos",
          fields: [{ key: "photos", kind: "folder" }, { key: "calibration", kind: "file" }],
          providers: [
            {
              module: "masks",
              default: "threshold",
              settings: [setting("hole-cleanup-budget", "number", 0.02)],
              options: [
                { id: "threshold", platforms: { wasm: true }, available: true, settings: [setting("threshold-level", "text", null)] },
                { id: "import", platforms: { wasm: true }, available: true, settings: [] },
                { id: "external-sam", platforms: { wasm: false }, available: false, settings: [setting("sam-python", "executable", null)] },
              ],
            },
            {
              module: "cameras",
              default: "turntable",
              settings: [],
              options: [
                { id: "turntable", platforms: { wasm: true }, available: true, settings: [setting("turntable-span", "integer", 4), setting("turntable-matches", "path", null)] },
                { id: "markers", platforms: { wasm: true }, available: true, settings: [setting("markers-mat", "path", null), setting("markers-minimum", "integer", 5)] },
              ],
            },
          ],
          option_groups: [
            { id: "tools", label: "Tools", settings: [setting("python", "executable", null)] },
            { id: "gates", label: "Quality gates", settings: [setting("minimum-registered-fraction", "number", 0.8)] },
          ],
        },
      ],
    };
    const points = browserStartPoints(described);
    expect(points.map((point) => point.id)).toEqual(["inputs", "photos"]);
    const photos = points[1]!;
    expect(photos.fields.map((field) => field.key)).toEqual(["photos", "calibration"]);
    const masks = photos.providers[0]!;
    expect(masks.options.map((option) => [option.id, option.available])).toEqual([["threshold", true], ["import", false]]);
    expect(masks.options[1]!.reason).toMatch(/Not in the browser yet/);
    const cameras = photos.providers[1]!;
    expect(cameras.options.map((option) => [option.id, option.available])).toEqual([["turntable", true], ["markers", false]]);
    expect(cameras.options[0]!.settings.map((spec) => spec.name)).toEqual(["turntable-span"]);
    expect(cameras.options[1]!.settings.map((spec) => spec.name)).toEqual(["markers-minimum"]);
    expect(photos.optionGroups.map((group) => group.id)).toEqual(["gates"]);
    expect(browserStartPoints(null)).toEqual([]);
    expect(browserStartPoints({ start_points: "nonsense" })).toEqual([]);
  });

  it("names downloads after the run and what the file is", () => {
    expect(downloadName("browser-01-bunny", "mesh/mesh.stl")).toBe("browser-01-bunny-mesh.stl");
    expect(downloadName("r 1", "stereo/depth-merged.png")).toBe("r-1-stereo-depth-merged.png");
    expect(downloadName("", "input-sheet.png")).toBe("input-sheet.png");
  });
});

describe("the photos start", () => {
  // In the shape of `crispmesh-dense run --describe`, as the app hands it on.
  const setting = (name: string, kind: string, fallback: unknown, more: Record<string, unknown> = {}) => ({ name, flag: `--${name}`, kind, default: fallback, choices: [], repeated: false, meaning: `Meaning of ${name}`, ...more });
  const described = [
    {
      id: "photos",
      label: "Turntable photos",
      fields: [{ key: "photos", label: "Photos", kind: "folder", required: true }, { key: "calibration", label: "Lens calibration (.json)", kind: "file", required: true }],
      providers: [
        {
          module: "masks",
          label: "Masks",
          default: "external-sam",
          settings: [setting("hole-cleanup-budget", "number", 0.02)],
          options: [
            { id: "threshold", label: "threshold", available: true, reason: null, version: "0.1.0", external: [], license: "this crate (AGPL-3.0-only)", settings: [setting("threshold-level", "text", null), setting("threshold-envelope", "text", "auto")], inputs: [] },
            { id: "import", label: "import", available: true, external: [], settings: [], inputs: [{ key: "masks_import", label: "Masks folder", kind: "folder", help: "One PNG per photo." }] },
            {
              id: "external-sam",
              label: "external-sam",
              available: false,
              reason: "SAM 2.1 runs in an external Python: missing an interpreter with PyTorch (--sam-python)",
              external: ["Python interpreter with PyTorch"],
              settings: [setting("sam-python", "executable", null), setting("sam-device", "choice", "mps", { choices: ["mps", "cpu", "cuda"] }), setting("sam-multimask", "switch", true)],
              inputs: [],
            },
          ],
        },
        {
          module: "cameras",
          label: "Cameras",
          default: "turntable",
          settings: [setting("clahe-clip", "number", 2.0), setting("open-turn", "switch", false)],
          options: [
            { id: "turntable", external: [], version: "0.1.0", settings: [setting("turntable-features", "integer", 6000), setting("turntable-span", "integer", 4), setting("turntable-matches", "path", null)] },
            { id: "markers", external: [], settings: [setting("markers-minimum", "integer", 5)], inputs: [{ key: "markers_mat", label: "Marker mat description", kind: "file" }] },
            {
              id: "colmap",
              available: true,
              version: "3.9.1",
              external: ["colmap"],
              license: "COLMAP BSD-3-Clause",
              settings: [setting("colmap-matching", "choice", "exhaustive", { choices: ["exhaustive", "sequential", "ring"] }), setting("colmap-overlap", "integer", 10), setting("colmap-mapper-option", "text", null, { repeated: true })],
            },
            { id: "alicevision", available: false, reason: "AliceVision is an external program: give its install prefix", external: ["aliceVision_cameraInit"] },
            { id: "import" },
          ],
        },
      ],
      option_groups: [
        { id: "gates", label: "Quality gates", settings: [setting("minimum-registered-fraction", "number", 0.8)] },
        { id: "nothing", label: "Empty", settings: [setting("python", "executable", null)] },
      ],
    },
  ];
  const point = parseStartPoints(described)[0]!;
  const provider = (module: string, id: string) => point.providers.find((choice) => choice.module === module)!.options.find((option) => option.id === id)!;
  const form = (values: Record<string, string>, providers: Record<string, string>, options: Record<string, string | boolean> = {}) => ({ name: "", device: "", reference: "", values, providers, options });

  it("keeps providers that cannot run, with the reason, and defaults to one that can", () => {
    const [masks, cameras] = point.providers;
    expect(masks!.options.map((option) => [option.id, option.available])).toEqual([["threshold", true], ["import", true], ["external-sam", false]]);
    expect(provider("masks", "external-sam").reason).toContain("SAM 2.1 runs in an external Python");
    // The engine's default cannot run: the first that can.
    expect(masks!.default).toBe("threshold");
    expect(cameras!.default).toBe("turntable");
    expect(provider("cameras", "colmap")).toMatchObject({ external: ["colmap"], license: "COLMAP BSD-3-Clause", available: true, version: "3.9.1" });
    expect(chosenProviders(point, {})).toEqual({ masks: "threshold", cameras: "turntable" });
    expect(chosenProviders(point, { cameras: "import", masks: "nonsense" })).toEqual({ masks: "threshold", cameras: "import" });
  });

  it("takes every option from the engine's description and has no field for a path or a program", () => {
    expect(provider("masks", "threshold").settings).toEqual([
      { name: "threshold-level", label: "Meaning of threshold-level", kind: "text", default: "", choices: [], repeated: false },
      { name: "threshold-envelope", label: "Meaning of threshold-envelope", kind: "text", default: "auto", choices: [], repeated: false },
    ]);
    expect(provider("masks", "external-sam").settings.map((spec) => [spec.name, spec.kind, spec.default])).toEqual([["sam-device", "choice", "mps"], ["sam-multimask", "switch", "on"]]);
    expect(provider("cameras", "turntable").settings.map((spec) => spec.name)).toEqual(["turntable-features", "turntable-span"]);
    expect(point.providers.map((choice) => choice.settings.map((spec) => spec.name))).toEqual([["hole-cleanup-budget"], ["clahe-clip", "open-turn"]]);
    // A group with nothing the form can show is dropped.
    expect(point.optionGroups.map((group) => [group.id, group.label, group.settings.length])).toEqual([["gates", "Quality gates", 1]]);
    expect(activeOptions(point, {}).map((spec) => spec.name)).toEqual(["threshold-level", "threshold-envelope", "turntable-features", "turntable-span", "hole-cleanup-budget", "clahe-clip", "open-turn", "minimum-registered-fraction"]);
    expect(parseOptionSpecs([{ flag: "--x-y", kind: "integer", default: 3 }, { name: "x-y", kind: "integer" }, { name: "odd", kind: "matrix" }, "junk"])).toEqual([{ name: "x-y", label: "x-y", kind: "integer", default: "3", choices: [], repeated: false }]);
  });

  it("builds the request the engine expects: fields, providers, only the options that differ", () => {
    const start = {
      ...form({ photos: "rgb", calibration: "/repo/calibrations/3dlf-pro.json", masks_import: "left over" }, { masks: "threshold", cameras: "colmap" }),
      name: "bunny",
      options: { "threshold-level": "otsu", "threshold-envelope": "auto", "colmap-matching": "ring", "colmap-mapper-option": "--ba_refine_focal_length  0", "turntable-span": "9", "clahe-clip": "2.00", "open-turn": true, "minimum-registered-fraction": " 0.9 " },
    };
    expect(startBody(point, start, { grid: 320 })).toEqual({
      photos: "rgb",
      calibration: "/repo/calibrations/3dlf-pro.json",
      name: "bunny",
      providers: { masks: "threshold", cameras: "colmap" },
      // The provider's own, the module's, the groups'; nothing of a provider that was not chosen, nothing at its default.
      photo_options: ["--threshold-level", "otsu", "--colmap-matching", "ring", "--colmap-mapper-option", "--ba_refine_focal_length", "--colmap-mapper-option", "0", "--open-turn", "--minimum-registered-fraction", "0.9"],
      settings: { grid: 320 },
    });
    expect(startBody(point, { ...start, options: {} }, {})).not.toHaveProperty("photo_options");
    expect(optionTokens(provider("masks", "external-sam").settings, { "sam-multimask": false, "sam-device": "cpu" })).toEqual(["--sam-device", "cpu", "--no-sam-multimask"]);
  });

  it("asks for the paths a provider needs as data of the run", () => {
    const providers = { masks: "import", cameras: "import" };
    // The app says which; an engine that does not is assumed to import from one path per module.
    expect(importFields(point, providers).map((entry) => entry.spec)).toEqual([
      { key: "masks_import", label: "Masks folder", kind: "folder", help: "One PNG per photo." },
      { key: "cameras_import", label: "Existing cameras", kind: "file" },
    ]);
    expect(missingFields(point, { photos: "rgb", calibration: "lens.json" }, providers)).toEqual(["masks_import", "cameras_import"]);
    expect(startBody(point, form({ photos: "rgb", calibration: "lens.json", masks_import: "bunny/masks", cameras_import: "bunny/final.sfm" }, providers), {})).toMatchObject({ masks_import: "bunny/masks", cameras_import: "bunny/final.sfm", providers });
    expect(missingFields(point, { photos: "rgb", calibration: "lens.json" }, {})).toEqual([]);
    // The printed marker mat: its description is a file with a field of its own, never an option word.
    expect(missingFields(point, { photos: "rgb", calibration: "lens.json" }, { cameras: "markers" })).toEqual(["markers_mat"]);
    const sent = startBody(point, form({ photos: "rgb", calibration: "lens.json", markers_mat: "mat.json" }, { cameras: "markers" }, { "markers-minimum": "4" }), {});
    expect(sent).toMatchObject({ markers_mat: "mat.json", providers: { masks: "threshold", cameras: "markers" }, photo_options: ["--markers-minimum", "4"] });
  });

  it("catches typing slips before sending", () => {
    expect(startOptionErrors(point, { cameras: "colmap" }, { "colmap-overlap": "many", "clahe-clip": "x", "colmap-matching": "spiral", "threshold-envelope": "--output", "turntable-span": "nonsense of a provider that is not chosen" })).toEqual({
      "colmap-overlap": "Needs a whole number.",
      "clahe-clip": "Needs a number.",
      "colmap-matching": "Must be one of: exhaustive, sequential, ring.",
      "threshold-envelope": "A value cannot start with two dashes.",
    });
    expect(optionTokens(provider("cameras", "colmap").settings, { "colmap-overlap": "many" })).toEqual([]);
    expect(optionErrors(provider("masks", "threshold").settings, { "threshold-envelope": "" })).toEqual({ "threshold-envelope": "Needs a value." });
  });
});

describe("a run that started from photos, in the reducer", () => {
  it("puts masks and cameras first, in that order, whenever they turn up", () => {
    let state = reduce(initialState(), event("run_started"));
    state = reduce(state, event("stage_started", { stage: "cameras", provider: "alicevision" }, 1));
    state = reduce(state, event("stage_started", { stage: "masks", provider: "threshold" }, 2));
    state = reduce(state, event("stage_started", { stage: "texture" }, 3));
    expect(state.stages.map((stage) => stage.name)).toEqual(["masks", "cameras", "inputs", "stereo", "mesh", "check", "evaluate", "texture"]);
    // A run from an inputs folder has neither.
    expect(initialState().stages.map((stage) => stage.name)).toEqual(["inputs", "stereo", "mesh", "check", "evaluate"]);
  });

  it("shows the mask sheet, the sparse overlay and the stage's metrics", () => {
    let state = reduce(initialState(), event("run_started"));
    state = reduce(state, event("stage_started", { stage: "masks", provider: "threshold" }, 1));
    state = reduce(state, event("metric", { stage: "masks", name: "mask_area_median_fraction", value: 0.15893 }, 2));
    state = reduce(state, event("artifact", { stage: "masks", kind: "mask_sheet", path: "frontend/mask-contact-sheet.png", label: "Masks on photos (red: dark pixels left out)" }, 3));
    state = reduce(state, event("stage_finished", { stage: "masks", seconds: 21.4 }, 4));
    state = reduce(state, event("stage_started", { stage: "cameras", provider: "alicevision" }, 5));
    state = reduce(state, event("artifact", { stage: "cameras", kind: "sparse_overlay", path: "frontend/sparse-overlay.png", label: "Sparse points on undistorted photos and masks" }, 6));
    expect(state.sheets.map((sheet) => [sheet.kind, sheet.stage])).toEqual([["mask_sheet", "masks"], ["sparse_overlay", "cameras"]]);
    expect(state.metrics).toEqual([{ name: "mask_area_median_fraction", value: 0.15893, stage: "masks", seq: 2 }]);
    expect(state.stages.slice(0, 2).map((stage) => [stage.name, stage.status])).toEqual([["masks", "done"], ["cameras", "running"]]);
    expect(state.ignored).toBe(0);
  });
});

describe("asking the page's own origin for an engine", () => {
  it("only where an engine could be serving the page", () => {
    const at = (url: string) => {
      const parsed = new URL(url);
      return servedByEngine({ protocol: parsed.protocol, hostname: parsed.hostname, pathname: parsed.pathname });
    };
    expect(at("http://127.0.0.1:8765/")).toBe(true);
    expect(at("http://192.168.1.20:8765/index.html")).toBe(true);
    expect(at("https://crispstrobe.github.io/CrispMesh/")).toBe(false);
    expect(at("https://someone.github.io/")).toBe(false);
    expect(at("https://example.org/studio/")).toBe(false);
    expect(at("tauri://localhost/")).toBe(false);
  });
});
