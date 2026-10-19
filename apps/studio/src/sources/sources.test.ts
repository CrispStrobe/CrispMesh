import { describe, expect, it } from "vitest";
import type { RunEvent } from "../core/events";
import { FakeTime, fakeFetch, fixtureEvents, fixtureText, json, settle } from "../testing/fixture";
import { BACKOFF_MS, HttpEngine, normaliseEngineUrl, POLL_MS } from "./httpEngine";
import { ReplaySource } from "./replaySource";
import { RunStore } from "./runStore";
import { EngineError, type LinkStatus, type SourceUpdate } from "./types";

function collect(source: { subscribe(listener: (update: SourceUpdate) => void): () => void }) {
  const events: RunEvent[] = [];
  const links: LinkStatus[] = [];
  const resets: number[] = [];
  source.subscribe((update) => {
    if (update.type === "events") events.push(...update.events);
    else if (update.type === "reset") {
      resets.push(update.events.length);
      events.length = 0;
      events.push(...update.events);
    } else links.push(update.link);
  });
  return { events, links, resets };
}

describe("normaliseEngineUrl", () => {
  it("accepts the usual ways of writing an address", () => {
    expect(normaliseEngineUrl("http://127.0.0.1:8765")).toBe("http://127.0.0.1:8765");
    expect(normaliseEngineUrl(" 127.0.0.1:8765/ ")).toBe("http://127.0.0.1:8765");
    expect(normaliseEngineUrl("https://box.example/engine/")).toBe("https://box.example/engine");
  });

  it("refuses credentials in the address and other schemes", () => {
    expect(() => normaliseEngineUrl("http://user:secret@host:1")).toThrow(/token field/);
    expect(() => normaliseEngineUrl("ftp://host")).toThrow();
    expect(() => normaliseEngineUrl("")).toThrow();
  });
});

describe("HttpEngine", () => {
  it("offers cached Gaussian refinement only when the host advertises it", async () => {
    const { fetcher, requests } = fakeFetch((request) => request.url.endsWith("/api/health")
      ? json({ can_start_runs: true, refine_runs: true, base_surfaces: ["silhouette", "stereo"] })
      : json({ id: "refined" }, 201));
    const engine = new HttpEngine("http://e", { fetcher });
    expect(engine.openRun("parent").refine).toBeUndefined();
    expect((await engine.health()).baseSurfaces).toEqual(["silhouette", "stereo"]);
    expect(await engine.openRun("parent").refine!()).toBe("refined");
    expect(requests.at(-1)).toMatchObject({ method: "POST", url: "http://e/api/runs/parent/refine" });
  });

  it("reads health, settings and runs, tolerating fields it does not know", async () => {
    const { fetcher, requests } = fakeFetch((request) => {
      if (request.url.endsWith("/api/health")) return json({ schema: "crispmesh_dense_events_v1", device: "mps", can_start_runs: true, extra: 1 });
      if (request.url.endsWith("/api/settings")) return json({ settings: [{ name: "grid", group: "Hull", meaning: "m", kind: "integer", default: 400 }] });
      return json({ runs: [{ id: "r1", status: "running", started: 5, stage: "stereo", stage_fraction: 0.4, events: 9, colour: "blue" }, { nonsense: true }] });
    });
    const engine = new HttpEngine("http://engine:8765/", { fetcher });
    expect(await engine.health()).toEqual({ schema: "crispmesh_dense_events_v1", device: "mps", canStartRuns: true });
    expect((await engine.settings())[0]?.name).toBe("grid");
    expect(await engine.listRuns()).toEqual([{ id: "r1", status: "running", started: 5, stage: "stereo", stageFraction: 0.4, events: 9 }]);
    expect(requests.map((request) => request.url)).toEqual([
      "http://engine:8765/api/health",
      "http://engine:8765/api/settings",
      "http://engine:8765/api/runs",
    ]);
  });

  it("sends the token only as a bearer header, never in a URL", async () => {
    const { fetcher, requests } = fakeFetch(() => json({ runs: [] }));
    await new HttpEngine("http://engine:8765", { fetcher, token: "s3cret" }).listRuns();
    expect(requests[0]?.headers.Authorization).toBe("Bearer s3cret");
    expect(requests[0]?.url).not.toContain("s3cret");
  });

  it("starts a run and returns its id", async () => {
    const { fetcher, requests } = fakeFetch(() => json({ id: "20261005-120000-sphere" }, 201));
    const id = await new HttpEngine("http://e", { fetcher }).startRun({ inputs: "sphere", settings: { grid: 96 } });
    expect(id).toBe("20261005-120000-sphere");
    expect(requests[0]).toMatchObject({ method: "POST", url: "http://e/api/runs" });
    expect(JSON.parse(requests[0]!.body!)).toEqual({ inputs: "sphere", settings: { grid: 96 } });
  });

  it("passes the engine's validation sentence on as a 400", async () => {
    const { fetcher } = fakeFetch(() => json({ error: "dense configuration: sizes must increase" }, 400));
    const failure = await new HttpEngine("http://e", { fetcher }).startRun({}).catch((problem: unknown) => problem);
    expect(failure).toBeInstanceOf(EngineError);
    expect(failure).toMatchObject({ status: 400, message: "dense configuration: sizes must increase" });
  });

  it("says so when nothing answers, without leaking the token", async () => {
    const fetcher = () => Promise.reject(new TypeError("Failed to fetch"));
    const failure = await new HttpEngine("http://e", { fetcher, token: "s3cret" }).health().catch((problem: unknown) => problem);
    expect(failure).toMatchObject({ status: 0 });
    expect(String((failure as Error).message)).not.toContain("s3cret");
  });

  it("sends captured photos to an engine that takes uploads, one request per photo, with the token", async () => {
    const { fetcher, requests } = fakeFetch((request) => {
      if (request.url.endsWith("/api/health")) return json({ schema: "crispmesh_dense_events_v1", can_start_runs: true, uploads: true });
      const name = new URL(request.url).searchParams.get("name");
      return json({ path: `capture-1/${name}` }, 201);
    });
    const engine = new HttpEngine("http://computer:8766", { fetcher, token: "s3cret" });
    expect(engine.saveCapture).toBeUndefined();
    await engine.health();
    const photos = ["capture_0001.jpg", "capture_0002.jpg"].map((name) => new File([new Uint8Array([1, 2, 3])], name, { type: "image/jpeg" }));
    const progress: number[] = [];
    expect(await engine.saveCapture!("capture-1", photos, (done) => progress.push(done))).toBe("capture-1/capture_0002.jpg");
    expect(progress).toEqual([1, 2]);
    const uploads = requests.filter((request) => request.url.includes("/api/uploads"));
    expect(uploads.map((request) => [request.method, request.url, request.headers.Authorization])).toEqual([
      ["POST", "http://computer:8766/api/uploads?folder=capture-1&name=capture_0001.jpg", "Bearer s3cret"],
      ["POST", "http://computer:8766/api/uploads?folder=capture-1&name=capture_0002.jpg", "Bearer s3cret"],
    ]);
    // An engine that does not say so gets no Take photos.
    const { fetcher: plain } = fakeFetch(() => json({ schema: "crispmesh_dense_events_v1", can_start_runs: true }));
    const other = new HttpEngine("http://e", { fetcher: plain });
    await other.health();
    expect(other.saveCapture).toBeUndefined();
  });

  it("does not mistake another web server for an engine", async () => {
    const { fetcher } = fakeFetch(() => json({ hello: "world" }));
    await expect(new HttpEngine("http://e", { fetcher }).health()).rejects.toThrow(/not a CrispMesh engine/);
  });
});

describe("HttpRunSource polling", () => {
  const all = fixtureEvents();

  function serve(available: () => number, fail: () => Response | null = () => null) {
    return fakeFetch((request) => {
      const failure = fail();
      if (failure !== null) return failure;
      const since = Number(new URL(request.url).searchParams.get("since"));
      const events = all.slice(since, available());
      return json({ events, next: events.length > 0 ? events[events.length - 1]!.seq + 1 : since });
    });
  }

  it("polls about once a second with a moving cursor and stops at run_finished", async () => {
    let available = 5;
    const time = new FakeTime();
    const { fetcher, requests } = serve(() => available);
    const source = new HttpEngine("http://e", { fetcher, timer: time.timer }).openRun("r1");
    const seen = collect(source);
    source.start();
    await settle();
    expect(seen.events).toHaveLength(5);
    expect(time.pending()).toEqual([POLL_MS]);

    time.advance(POLL_MS);
    await settle();
    expect(seen.events).toHaveLength(5); // nothing new: the cursor stays

    available = 20;
    time.advance(POLL_MS);
    await settle();
    expect(seen.events).toHaveLength(20);

    available = all.length;
    time.advance(POLL_MS);
    await settle();
    expect(seen.events.map((event) => event.seq)).toEqual(all.map((event) => event.seq));
    expect(requests.map((request) => new URL(request.url).search)).toEqual(["?since=0", "?since=5", "?since=5", "?since=20"]);
    expect(seen.links.at(-1)).toEqual({ state: "ended" });
    expect(time.pending()).toEqual([]);

    time.advance(60_000);
    await settle();
    expect(requests).toHaveLength(4);
  });

  it("backs off on errors, says why, and recovers without losing or repeating events", async () => {
    let available = 3;
    let broken = false;
    const time = new FakeTime();
    const { fetcher } = serve(
      () => available,
      () => (broken ? new Response("bad gateway", { status: 502 }) : null),
    );
    const source = new HttpEngine("http://e", { fetcher, timer: time.timer }).openRun("r1");
    const seen = collect(source);
    source.start();
    await settle();
    expect(seen.events).toHaveLength(3);

    broken = true;
    const waits: number[] = [];
    for (let i = 0; i < 7; i++) {
      time.advance(time.pending()[0]!);
      await settle();
      waits.push(time.pending()[0]!);
    }
    expect(waits).toEqual([1000, 2000, 4000, 8000, 15000, 15000, 15000]);
    expect(waits.every((wait) => (BACKOFF_MS as readonly number[]).includes(wait))).toBe(true);
    expect(seen.links.at(-1)).toMatchObject({ state: "retrying", message: expect.stringContaining("502") });

    broken = false;
    available = 10;
    time.advance(15000);
    await settle();
    expect(seen.events.map((event) => event.seq)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(seen.links.at(-1)).toEqual({ state: "live" });
    expect(time.pending()).toEqual([POLL_MS]);
  });

  it("gives up on a refused token or an unknown run instead of hammering the engine", async () => {
    for (const [status, pattern] of [
      [401, /token/],
      [404, /does not know this run/],
    ] as const) {
      const time = new FakeTime();
      const { fetcher, requests } = fakeFetch(() => json({ error: status === 401 ? "unauthorised" : "unknown run" }, status));
      const source = new HttpEngine("http://e", { fetcher, timer: time.timer }).openRun("r1");
      const seen = collect(source);
      source.start();
      await settle();
      expect(seen.links.at(-1)?.state).toBe("failed");
      expect(seen.links.at(-1)?.message).toMatch(pattern);
      time.advance(120_000);
      await settle();
      expect(requests).toHaveLength(1);
    }
  });

  it("never applies an event twice when the engine repeats one", async () => {
    const time = new FakeTime();
    let call = 0;
    const { fetcher } = fakeFetch(() => {
      call += 1;
      // A misbehaving engine: the second answer starts one event too early.
      return call === 1 ? json({ events: all.slice(0, 4), next: 4 }) : json({ events: all.slice(3, 6), next: 6 });
    });
    const source = new HttpEngine("http://e", { fetcher, timer: time.timer }).openRun("r1");
    const seen = collect(source);
    source.start();
    await settle();
    time.advance(POLL_MS);
    await settle();
    source.stop();
    expect(seen.events.map((event) => event.seq)).toEqual([0, 1, 2, 3, 4, 5]);
  });

  it("stops polling when stopped", async () => {
    const time = new FakeTime();
    const { fetcher, requests } = serve(() => 3);
    const source = new HttpEngine("http://e", { fetcher, timer: time.timer }).openRun("r1");
    source.start();
    await settle();
    source.stop();
    time.advance(60_000);
    await settle();
    expect(requests).toHaveLength(1);
  });

  it("serves files: plain URLs without a token, fetched object URLs with one", async () => {
    const open = new HttpEngine("http://e", { fetcher: fakeFetch(() => json({})).fetcher }).openRun("run 1");
    expect(await open.imageUrl("stereo/depth level.png")).toBe("http://e/api/runs/run%201/files/stereo/depth%20level.png");

    const created: string[] = [];
    const revoked: string[] = [];
    const { fetcher, requests } = fakeFetch(() => new Response(new Blob(["png"])));
    const guarded = new HttpEngine("http://e", {
      fetcher,
      token: "s3cret",
      objectUrls: {
        create: () => {
          created.push(`blob:${created.length}`);
          return created.at(-1)!;
        },
        revoke: (url) => revoked.push(url),
      },
    }).openRun("r1");
    expect(await guarded.imageUrl("input-sheet.png")).toBe("blob:0");
    expect(await guarded.imageUrl("input-sheet.png")).toBe("blob:0"); // cached, fetched once
    expect(requests).toHaveLength(1);
    expect(requests[0]).toMatchObject({ url: "http://e/api/runs/r1/files/input-sheet.png", headers: { Authorization: "Bearer s3cret" } });
    guarded.stop();
    expect(revoked).toEqual(["blob:0"]);
  });

  it("asks the engine to cancel", async () => {
    const { fetcher, requests } = fakeFetch(() => json({ id: "r1", cancel_requested: true }));
    await new HttpEngine("http://e", { fetcher }).openRun("r1").cancel!();
    expect(requests[0]).toMatchObject({ method: "POST", url: "http://e/api/runs/r1/cancel" });
  });
});

describe("ReplaySource", () => {
  function bundle(body = fixtureText("events.jsonl")) {
    return fakeFetch((request) =>
      request.url.endsWith("/events.jsonl") ? new Response(body) : new Response("missing", { status: 404 }),
    );
  }

  it("plays the bundle's events with their recorded timing into the reducer", async () => {
    const time = new FakeTime();
    const { fetcher, requests } = bundle();
    const source = new ReplaySource("https://host/bundles/sphere", { fetcher, clock: time.clock });
    const store = new RunStore(source);
    source.start();
    await settle();
    expect(requests[0]?.url).toBe("https://host/bundles/sphere/events.jsonl");
    expect(store.get().run.status).toBe("running");
    expect(store.get().run.eventCount).toBe(1);

    const recorded = fixtureEvents();
    const hull = recorded.find((event) => event.kind === "preview_mesh")!;
    time.advance((hull.time - recorded[0]!.time) * 1000 + 1);
    expect(store.get().run.meshes.map((mesh) => mesh.label)).toEqual(["Silhouette hull"]);
    time.advance((recorded.at(-1)!.time - hull.time) * 1000);
    expect(store.get().run.status).toBe("complete");
    expect(store.get().run.eventCount).toBe(recorded.length);
    expect(store.get().link.state).toBe("ended");
  });

  it("scrubbing backwards resets the state to exactly the events before that point", async () => {
    const time = new FakeTime();
    const source = new ReplaySource("https://host/b/", { fetcher: bundle().fetcher, clock: time.clock, speed: Infinity });
    const store = new RunStore(source);
    const seen = collect(source);
    source.start();
    await settle();
    expect(store.get().run.meshes).toHaveLength(4);

    source.replay.seek(6);
    expect(seen.resets).toEqual([6]);
    expect(store.get().run.eventCount).toBe(6);
    expect(store.get().run.status).toBe("running");
    expect(store.get().run.meshes.map((mesh) => mesh.label)).toEqual(["Silhouette hull"]);
    expect(store.get().run.sheets.map((sheet) => sheet.kind)).toEqual(["input_sheet", "mask_repair_sheet"]);

    source.replay.seek(0);
    expect(store.get().run.status).toBe("waiting");
    source.replay.seek(fixtureEvents().length);
    expect(store.get().run.status).toBe("complete");
    expect(store.get().run.reports).toHaveLength(2);
  });

  it("can start paused and step through", async () => {
    const time = new FakeTime();
    const source = new ReplaySource("https://host/b/", { fetcher: bundle().fetcher, clock: time.clock, autoplay: false });
    const store = new RunStore(source);
    source.start();
    await settle();
    expect(store.get().run.eventCount).toBe(0);
    source.replay.step(1);
    source.replay.step(1);
    expect(store.get().run.eventCount).toBe(2);
    expect(source.replay.snapshot()).toMatchObject({ position: 2, total: fixtureEvents().length, playing: false });
  });

  it("ignores a half-written last line in a bundle", async () => {
    const time = new FakeTime();
    const text = fixtureText("events.jsonl");
    const cut = text.slice(0, text.length - 40);
    const source = new ReplaySource("https://host/b/", { fetcher: bundle(cut).fetcher, clock: time.clock, speed: Infinity });
    const store = new RunStore(source);
    source.start();
    await settle();
    expect(store.get().run.eventCount).toBe(fixtureEvents().length - 1);
    expect(store.get().run.status).toBe("running");
  });

  it("resolves files relative to the bundle and reports the virtual time", async () => {
    const time = new FakeTime();
    const source = new ReplaySource("https://host/b", { fetcher: bundle().fetcher, clock: time.clock });
    expect(await source.imageUrl("stereo/depth-level-0.png")).toBe("https://host/b/stereo/depth-level-0.png");
    source.start();
    await settle();
    const first = fixtureEvents()[0]!.time;
    time.advance(500);
    expect(source.now()).toBeCloseTo(first + 0.5, 3);
  });

  it("fails clearly when there is no event log", async () => {
    for (const fetcher of [
      fakeFetch(() => new Response("nope", { status: 404 })).fetcher,
      fakeFetch(() => new Response("<!doctype html><html></html>\n")).fetcher,
    ]) {
      const source = new ReplaySource("https://host/b/", { fetcher, clock: new FakeTime().clock });
      const seen = collect(source);
      source.start();
      await settle();
      expect(seen.links.at(-1)?.state).toBe("failed");
      expect(seen.events).toEqual([]);
    }
  });
});

