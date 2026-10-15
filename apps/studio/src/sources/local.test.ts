import { describe, expect, it } from "vitest";
import type { RunEvent } from "../core/events";
import { CONTRACT_START_POINTS, missingFields, parseStartPoints, startBody } from "../core/startPoints";
import { FakeTime, fixtureBytes, fixtureEvents, settle } from "../testing/fixture";
import { LOCAL_BACKOFF_MS, LOCAL_POLL_MS, LocalEngine, parseMethods, type Bridge } from "./localEngine";
import { RunStore } from "./runStore";
import { EngineError, type LinkStatus, type SourceUpdate } from "./types";

/** A stand-in for the shell: answers commands from a table and records what was asked. */
function shell(answers: Record<string, (args: Record<string, unknown>, headers?: Record<string, string>) => unknown>) {
  const calls: { command: string; args: Record<string, unknown>; bytes?: Uint8Array; headers?: Record<string, string> }[] = [];
  const bridge: Bridge = async <T>(command: string, payload: Record<string, unknown> | Uint8Array = {}, options?: { headers: Record<string, string> }) => {
    const args = payload instanceof Uint8Array ? {} : payload;
    calls.push({ command, args, bytes: payload instanceof Uint8Array ? payload : undefined, headers: options?.headers });
    const answer = answers[command];
    if (answer === undefined) throw `no such command: ${command}`;
    return answer(args, options?.headers) as T;
  };
  return { bridge, calls };
}

function collect(source: { subscribe(listener: (update: SourceUpdate) => void): () => void }) {
  const events: RunEvent[] = [];
  const links: LinkStatus[] = [];
  source.subscribe((update) => {
    if (update.type === "events") events.push(...update.events);
    else if (update.type === "link") links.push(update.link);
  });
  return { events, links };
}

const all = fixtureEvents();
const page = (available: () => number) => (args: Record<string, unknown>) => {
  const since = args.since as number;
  const events = all.slice(since, available());
  return { events, next: events.length > 0 ? events[events.length - 1]!.seq + 1 : Math.min(since, available()) };
};

describe("LocalEngine over a mocked command bridge", () => {
  it("copies files picked on a phone into the data folder, one raw command per file", async () => {
    const { bridge, calls } = shell({
      native_health: () => ({ schema: "crispmesh_dense_events_v1", can_start_runs: true, imports: true }),
      native_import: (_args, headers) => ({ path: `${decodeURIComponent(headers!["x-folder"]!)}/${decodeURIComponent(headers!["x-name"]!)}` }),
    });
    const engine = new LocalEngine(bridge);
    await expect(engine.importFiles("x", [])).rejects.toThrow(/no import/);
    expect((await engine.health()).imports).toBe(true);
    // On a phone there is no folder dialog of the app's own to offer.
    expect(engine.pickPath).toBeUndefined();
    const progress: string[] = [];
    const files = [new File([new Uint8Array([1, 2])], "IMG 1.jpg"), new File([new Uint8Array([3])], "Bild_ä.jpg")];
    expect(await engine.importFiles("photos-1", files, (done, total) => progress.push(`${done}/${total}`))).toBe("photos-1/Bild_ä.jpg");
    const sent = calls.filter((call) => call.command === "native_import");
    expect(sent.map((call) => [call.headers, [...call.bytes!]])).toEqual([
      [{ "x-folder": "photos-1", "x-name": "IMG%201.jpg" }, [1, 2]],
      [{ "x-folder": "photos-1", "x-name": "Bild_%C3%A4.jpg" }, [3]],
    ]);
    expect(progress).toEqual(["1/2", "2/2"]);
  });

  it("reads health with the engine's start points and what it does not offer", async () => {
    const { bridge } = shell({
      native_health: () => ({
        schema: "crispmesh_dense_events_v1",
        device: "wgpu",
        can_start_runs: true,
        data_dir: "/Users/me/data",
        start_points: [{ id: "inputs", label: "Inputs folder", fields: [{ key: "inputs", label: "Inputs folder", kind: "inputs", required: true }], providers: [] }],
        later_field: 1,
      }),
    });
    const health = await new LocalEngine(bridge).health();
    expect(health).toMatchObject({ schema: "crispmesh_dense_events_v1", canStartRuns: true, choosesDevice: false, scoresReference: false, dataFolder: "/Users/me/data" });
    expect(health.startPoints?.map((point) => point.id)).toEqual(["inputs"]);
  });

  it("lists runs and settings like the HTTP engine", async () => {
    const { bridge } = shell({
      native_runs: () => ({ runs: [{ id: "r2", status: "running", started: 9, stage: "stereo", stage_fraction: 0.5, events: 7 }, { junk: true }] }),
      native_settings: () => ({ settings: [{ name: "grid", group: "Hull", meaning: "m", kind: "integer", default: 400 }] }),
    });
    const engine = new LocalEngine(bridge);
    expect(await engine.listRuns()).toEqual([{ id: "r2", status: "running", started: 9, stage: "stereo", stageFraction: 0.5, events: 7 }]);
    expect((await engine.settings())[0]).toMatchObject({ name: "grid", default: 400 });
    expect(engine.kind).toBe("local");
  });

  it("starts a run and passes a refusal on as the engine's sentence", async () => {
    const good = shell({ native_start: () => ({ id: "20261006-101500-sphere" }) });
    expect(await new LocalEngine(good.bridge).startRun({ inputs: "sphere", settings: { grid: 96 } })).toBe("20261006-101500-sphere");
    expect(good.calls[0]).toEqual({ command: "native_start", args: { body: { inputs: "sphere", settings: { grid: 96 } } } });

    const bad = shell({
      native_start: () => {
        throw "dense configuration: sizes must increase";
      },
    });
    const failure = await new LocalEngine(bad.bridge).startRun({}).catch((problem: unknown) => problem);
    expect(failure).toBeInstanceOf(EngineError);
    expect(failure).toMatchObject({ status: 400, message: "dense configuration: sizes must increase" });
  });

  it("lists the data folder and opens the native picker", async () => {
    const { bridge, calls } = shell({
      native_data: () => ({ path: "objects", entries: [{ name: "sphere", directory: true, inputs: true }, { name: "scan.ply", directory: false, inputs: false }, 7] }),
      pick_path: () => "/Users/me/Desktop/bunny",
    });
    const engine = new LocalEngine(bridge);
    expect(await engine.listData("objects")).toEqual({
      path: "objects",
      entries: [
        { name: "sphere", directory: true, inputs: true },
        { name: "scan.ply", directory: false, inputs: false },
      ],
    });
    expect(await engine.pickPath!("folder", "Choose the inputs folder")).toBe("/Users/me/Desktop/bunny");
    expect(calls[1]).toEqual({ command: "pick_path", args: { kind: "folder", title: "Choose the inputs folder", start: null } });
  });
});

describe("LocalRunSource", () => {
  it("reads the event log by line number into the same reducer, and stops at run_finished", async () => {
    let available = 6;
    const time = new FakeTime();
    const { bridge, calls } = shell({ native_events: page(() => available) });
    const source = new LocalEngine(bridge, { timer: time.timer }).openRun("r1");
    const store = new RunStore(source);
    const seen = collect(source);
    source.start();
    await settle();
    expect(store.get().run.status).toBe("running");
    expect(store.get().run.meshes.map((mesh) => mesh.label)).toEqual(["Silhouette hull"]);
    expect(time.pending()).toEqual([LOCAL_POLL_MS]);

    time.advance(LOCAL_POLL_MS);
    await settle();
    expect(seen.events).toHaveLength(6);

    available = all.length;
    time.advance(LOCAL_POLL_MS);
    await settle();
    expect(seen.events.map((event) => event.seq)).toEqual(all.map((event) => event.seq));
    expect(store.get().run.status).toBe("complete");
    expect(store.get().run.reports).toHaveLength(2);
    expect(seen.links.at(-1)).toEqual({ state: "ended" });
    expect(time.pending()).toEqual([]);
    expect(calls.map((call) => call.args.since)).toEqual([0, 6, 6]);
    expect(calls.every((call) => call.args.id === "r1")).toBe(true);
  });

  it("shows a run that failed for want of a GPU with the engine's message", async () => {
    const events = [
      { type: "run_started", time: 1, stage: null, schema: "crispmesh_dense_events_v1", device: "wgpu", seq: 0 },
      { type: "error", time: 2, stage: null, message: "no GPU adapter: No suitable graphics adapter found", seq: 1 },
      { type: "run_finished", time: 2, stage: null, status: "failed", seconds: 1, seq: 2 },
    ];
    const { bridge } = shell({ native_events: (args) => ({ events: events.slice(args.since as number), next: 3 }) });
    const source = new LocalEngine(bridge, { timer: new FakeTime().timer }).openRun("r1");
    const store = new RunStore(source);
    source.start();
    await settle();
    expect(store.get().run.status).toBe("failed");
    expect(store.get().run.errors).toEqual([{ seq: 1, stage: null, message: "no GPU adapter: No suitable graphics adapter found" }]);
    expect(store.get().link.state).toBe("ended");
  });

  it("backs off when the shell fails and gives up on an unknown run", async () => {
    let broken = true;
    const time = new FakeTime();
    const { bridge } = shell({
      native_events: (args) => {
        if (broken) throw "the folder is gone";
        return page(() => 4)(args);
      },
    });
    const source = new LocalEngine(bridge, { timer: time.timer }).openRun("r1");
    const seen = collect(source);
    source.start();
    await settle();
    const waits = [time.pending()[0]!];
    for (let i = 0; i < 4; i++) {
      time.advance(time.pending()[0]!);
      await settle();
      waits.push(time.pending()[0]!);
    }
    expect(waits).toEqual([500, 1000, 2000, 5000, 5000]);
    expect(waits.every((wait) => (LOCAL_BACKOFF_MS as readonly number[]).includes(wait))).toBe(true);
    expect(seen.links.at(-1)).toEqual({ state: "retrying", message: "the folder is gone" });
    broken = false;
    time.advance(5000);
    await settle();
    expect(seen.events).toHaveLength(4);
    expect(seen.links.at(-1)).toEqual({ state: "live" });

    const unknown = shell({
      native_events: () => {
        throw "unknown run";
      },
    });
    const gone = new LocalEngine(unknown.bridge, { timer: time.timer }).openRun("nope");
    const links = collect(gone).links;
    gone.start();
    await settle();
    expect(links.at(-1)?.state).toBe("failed");
    expect(unknown.calls).toHaveLength(1);
  });

  it("fetches run files as bytes through the shell: meshes, reports, and images as object URLs", async () => {
    const stl = fixtureBytes("mesh/mesh.stl");
    const created: Blob[] = [];
    const revoked: string[] = [];
    const { bridge, calls } = shell({
      native_file: (args) => {
        if (args.path === "mesh/mesh.stl") return stl;
        if (args.path === "mesh/result.json") return new TextEncoder().encode('{"triangles": 33800, "closed": true}');
        if (args.path === "input-sheet.png") return [137, 80, 78, 71];
        throw "not found";
      },
      native_file_size: (args) => (args.path === "mesh/mesh.stl" ? stl.byteLength : 0),
      native_cancel: () => ({ id: "r1", cancel_requested: true }),
    });
    const source = new LocalEngine(bridge, {
      objectUrls: {
        create: (blob) => {
          created.push(blob);
          return `blob:${created.length}`;
        },
        revoke: (url) => revoked.push(url),
      },
    }).openRun("r1");

    const progress: number[] = [];
    const bytes = await source.fetchBytes("mesh/mesh.stl", { onProgress: (received) => progress.push(received) });
    expect(bytes.byteLength).toBe(stl.byteLength);
    expect(progress).toEqual([stl.byteLength]);
    expect(await source.fetchJson("mesh/result.json")).toEqual({ triangles: 33800, closed: true });
    expect(await source.fileSize!("mesh/mesh.stl")).toBe(stl.byteLength);
    expect(await source.fileSize!("missing.stl")).toBeUndefined();

    expect(await source.imageUrl("input-sheet.png")).toBe("blob:1");
    expect(await source.imageUrl("input-sheet.png")).toBe("blob:1");
    expect(created[0]?.type).toBe("image/png");
    expect(created[0]?.size).toBe(4);
    expect(calls.filter((call) => call.command === "native_file" && call.args.path === "input-sheet.png")).toHaveLength(1);

    await expect(source.fetchBytes("../other/secret")).rejects.toMatchObject({ status: 404, message: "not found" });
    await source.cancel!();
    expect(calls.at(-1)).toEqual({ command: "native_cancel", args: { id: "r1" } });
    source.stop();
    expect(revoked).toEqual(["blob:1"]);
  });
});

describe("start points", () => {
  const providersJson = [
    {
      id: "photos",
      label: "Photos",
      fields: [{ key: "photos", label: "Photos folder", kind: "folder" }, { key: "lens", label: "Lens calibration", kind: "file", required: false }],
      providers: [
        { module: "masks", label: "Masks", default: "threshold", options: [{ id: "threshold", label: "Threshold" }, { id: "sam", label: "SAM 2.1", meaning: "needs PyTorch" }, { nonsense: 1 }] },
        { module: "cameras", label: "Cameras", default: "gone", options: [{ id: "alicevision" }, { id: "colmap", label: "COLMAP" }] },
        { module: "empty", options: [] },
      ],
    },
    { id: "broken" },
    "junk",
  ];

  it("reads a provider list given as JSON and drops what it cannot use", () => {
    const points = parseStartPoints(providersJson);
    expect(points).toHaveLength(1);
    expect(points[0]?.fields).toEqual([
      { key: "photos", label: "Photos folder", kind: "folder", required: true, help: undefined },
      { key: "lens", label: "Lens calibration", kind: "file", required: false, help: undefined },
    ]);
    expect(points[0]?.providers.map((choice) => [choice.module, choice.default, choice.options.map((option) => option.id)])).toEqual([
      ["masks", "threshold", ["threshold", "sam"]],
      ["cameras", "alicevision", ["alicevision", "colmap"]],
    ]);
    expect(parseStartPoints(null)).toEqual([]);
    expect(parseStartPoints({ not: "a list" })).toEqual([]);
  });

  it("builds the contract's start bodies", () => {
    const [inputs, scene] = CONTRACT_START_POINTS;
    const form = { name: "", device: "", reference: "", values: { inputs: " sphere ", scene: "left over from the other tab" }, providers: {} };
    expect(startBody(inputs!, form, {})).toEqual({ inputs: "sphere" });
    expect(missingFields(inputs!, { inputs: "  " })).toEqual(["inputs"]);
    const values = { scene: "bunny/final.sfm", prepared: "bunny/pngs", raw_masks: "bunny/masks", inputs: "ignored here" };
    expect(startBody(scene!, { name: "bunny", device: "mps", reference: "", values, providers: {} }, { grid: 320 })).toEqual({
      scene: "bunny/final.sfm",
      prepared: "bunny/pngs",
      raw_masks: "bunny/masks",
      name: "bunny",
      device: "mps",
      settings: { grid: 320 },
    });
    expect(missingFields(scene!, { scene: "x" })).toEqual(["prepared", "raw_masks"]);
  });

  it("sends the chosen providers for a start point that has a choice, defaults otherwise", () => {
    const photos = parseStartPoints(providersJson)[0]!;
    const form = { name: "", device: "", reference: "", values: { photos: "bunny/photos" }, providers: { masks: "sam", cameras: "not-offered" } };
    expect(startBody(photos, form, {})).toEqual({ photos: "bunny/photos", providers: { masks: "sam", cameras: "alicevision" } });
    expect(missingFields(photos, { photos: "x" })).toEqual([]);
  });
});

describe("optional surfaces in the health answer", () => {
  it("keeps well-formed methods and drops malformed ones", () => {
    const methods = parseMethods([
      {
        id: "surface",
        label: "Neural surface",
        meaning: "m",
        available: true,
        qualities: [{ id: "fast", label: "Fast", meaning: "f" }, { label: "no id" }],
        settings: [{ name: "steps", kind: "integer", meaning: "s" }, {}],
      },
      { label: "no id", qualities: [] },
      "text",
    ]);
    expect(methods).toEqual([
      {
        id: "surface",
        label: "Neural surface",
        meaning: "m",
        available: true,
        qualities: [{ id: "fast", label: "Fast", meaning: "f" }],
        settings: [{ name: "steps", kind: "integer", meaning: "s" }],
      },
    ]);
    expect(parseMethods(undefined)).toBeUndefined();
  });
});
