/**
 * Where downloaded example objects are kept, and how a run is started from them:
 * the app's data folder (built-in engine), or the browser's Cache Storage (engine in this page).
 */

import type { ExampleFile, ExampleManifest, ExampleObject, ExampleStore } from "../core/examples";
import type { BrowserEngine } from "./browserEngine";
import type { Bridge } from "./localEngine";
import type { Engine } from "./types";
import { t } from "../i18n";

const name = (file: ExampleFile) => file.path.split("/").pop() ?? file.path;

/** `<data>/examples/<id>/photos/<file>` and `<data>/examples/<id>/<calibration>`. */
export class AppExampleStore implements ExampleStore {
  constructor(
    private readonly bridge: Bridge,
    private readonly engine: Engine,
  ) {}

  private folder(object: ExampleObject, file: ExampleFile): string {
    return file === object.calibration ? `examples/${object.id}` : `examples/${object.id}/photos`;
  }

  async have(object: ExampleObject): Promise<Set<string>> {
    const present = new Set<string>();
    for (const [folder, files] of [
      [`examples/${object.id}/photos`, object.files],
      [`examples/${object.id}`, [object.calibration]],
    ] as const) {
      try {
        const listing = await this.engine.listData!(folder);
        const names = new Set(listing.entries.filter((entry) => !entry.directory).map((entry) => entry.name));
        for (const file of files) if (names.has(name(file))) present.add(file.path);
      } catch {
        // Not downloaded yet.
      }
    }
    return present;
  }

  async put(object: ExampleObject, file: ExampleFile, bytes: ArrayBuffer): Promise<void> {
    await this.bridge("native_import", new Uint8Array(bytes), {
      headers: { "x-folder": encodeURIComponent(this.folder(object, file)), "x-name": encodeURIComponent(name(file)) },
    });
  }

  free(): Promise<number | null> {
    return this.bridge<number | null>("native_free_space").catch(() => null);
  }

  remove(object: ExampleObject): Promise<void> {
    return this.bridge<void>("native_delete_example", { id: object.id });
  }

  /** Starts the photos run on the built-in engine; returns the run's id. */
  start(_manifest: ExampleManifest, object: ExampleObject, attribution: string): Promise<string> {
    return this.engine.startRun({
      photos: `examples/${object.id}/photos`,
      calibration: `examples/${object.id}/${name(object.calibration)}`,
      providers: { masks: "threshold", cameras: "turntable" },
      name: object.id,
      attribution,
    });
  }
}

const CACHE = "crispmesh-example-objects";

/** The browser's Cache Storage, keyed by the files' URLs: kept across visits, deletable. */
export class BrowserExampleStore implements ExampleStore {
  constructor(
    private readonly manifest: ExampleManifest,
    private readonly engine: BrowserEngine,
  ) {}

  private url(file: ExampleFile): string {
    return new URL(file.path, this.manifest.baseUrl).href;
  }

  async have(object: ExampleObject): Promise<Set<string>> {
    const present = new Set<string>();
    if (typeof caches === "undefined") return present;
    const cache = await caches.open(CACHE);
    for (const file of [object.calibration, ...object.files]) if ((await cache.match(this.url(file))) !== undefined) present.add(file.path);
    return present;
  }

  async put(_object: ExampleObject, file: ExampleFile, bytes: ArrayBuffer): Promise<void> {
    if (typeof caches === "undefined") throw new Error(t("inputs.examples.error.noCache"));
    const cache = await caches.open(CACHE);
    await cache.put(this.url(file), new Response(bytes, { headers: { "Content-Length": String(bytes.byteLength) } }));
  }

  async free(): Promise<number | null> {
    const estimate = await navigator.storage?.estimate?.().catch(() => undefined);
    return estimate?.quota !== undefined && estimate.usage !== undefined ? estimate.quota - estimate.usage : null;
  }

  async remove(object: ExampleObject): Promise<void> {
    if (typeof caches === "undefined") return;
    const cache = await caches.open(CACHE);
    for (const file of [object.calibration, ...object.files]) await cache.delete(this.url(file));
  }

  /** Hands the cached photos and calibration to the engine in this page and starts the run. */
  async start(_manifest: ExampleManifest, object: ExampleObject, attribution: string): Promise<string> {
    const cache = await caches.open(CACHE);
    const read = async (file: ExampleFile) => {
      const response = await cache.match(this.url(file));
      if (response === undefined) throw new Error(t("inputs.examples.error.notDownloaded", { path: file.path }));
      return response.blob();
    };
    const photos: [string, File][] = [];
    for (const file of object.files) photos.push([name(file), new File([await read(file)], name(file), { type: "image/png" })]);
    this.engine.setPhotos({ name: object.id, files: photos }, null);
    this.engine.setCalibration({ label: name(object.calibration), text: await (await read(object.calibration)).text() });
    return this.engine.startRun({ photos: object.id, calibration: name(object.calibration), providers: { masks: "threshold", cameras: "turntable" }, name: object.id, attribution });
  }
}
