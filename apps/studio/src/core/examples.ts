/**
 * Example objects: real photo sets the app can download on request and reconstruct, described
 * by a manifest (`crispmesh_example_objects_v1`). Nothing of them is bundled; the files come from
 * the public dataset repository the manifest sits in, each checked against its SHA-256.
 * No DOM.
 *
 *   {
 *     "schema": "crispmesh_example_objects_v1",
 *     "source": "<dataset page>", "doi": "...", "license": "CC-BY-4.0",
 *     "attribution": "<text shown before download and kept with every run>",
 *     "calibration_files": [ { "path": "calibration/3dlf-pro.json", "size": 1107, "sha256": "..." } ],
 *     "objects": [
 *       { "name": "dragon", "title": "Dragon", "photos": 73, "bytes": 109148125,
 *         "calibration": "calibration/3dlf-pro.json",
 *         "files": [ { "path": "dragon/rgb/dragon_0_rgb.png", "size": 1450869, "sha256": "..." } ] }
 *     ]
 *   }
 *
 * Paths are relative to the manifest's folder (`base_url` when given).
 */

export const EXAMPLES_SCHEMA = "crispmesh_example_objects_v1";

/** A dataset repository the app offers objects from, with what is said about its objects besides the dataset's own attribution. */
export interface ExampleSource {
  id: string;
  /** Heading of its list. */
  title: string;
  manifest: string;
  /** Shown before download and kept with every run, after the dataset's attribution. */
  purpose: string;
}

/** The sources, in the order they are listed: the freely licensed synthetic renders first. */
export const EXAMPLE_SOURCES: ExampleSource[] = [
  {
    id: "gso",
    title: "Rendered household objects",
    manifest: "https://huggingface.co/datasets/cstr/gso-turntable-photos/resolve/main/manifest.json",
    purpose: "Rendered photos of scanned household objects (synthetic, not photographs). Brands and products shown belong to their owners.",
  },
  {
    id: "3dlf",
    title: "Photographed figures",
    manifest: "https://huggingface.co/datasets/cstr/3dlf-scan-photos/resolve/main/manifest.json",
    purpose:
      "Photographs of 3D prints of models from the Stanford 3D Scanning Repository and similar sources, for evaluation and research. Stanford permits research use; commercial use or inclusion in a product for sale needs Stanford's permission.",
  },
];

export interface ExampleFile {
  /** Path relative to the manifest's folder. */
  path: string;
  size: number;
  sha256: string;
}

export interface ExampleObject {
  /** Short name, also the folder it is stored in. */
  id: string;
  name: string;
  photos: number;
  /** Bytes to download, the calibration included. */
  bytes: number;
  calibration: ExampleFile;
  /** The photos, in capture order. */
  files: ExampleFile[];
}

export interface Attribution {
  source?: string;
  doi?: string;
  license: string;
  /** The text shown before download and kept with every run made from these photos. */
  text: string;
}

export interface ExampleManifest {
  baseUrl: string;
  /** What the source says about its objects (`ExampleSource.purpose`); empty when none. */
  purpose: string;
  attribution: Attribution;
  objects: ExampleObject[];
}

/** Largest download the app accepts for one object. */
export const MAX_OBJECT_BYTES = 1024 * 1024 * 1024;
/** Largest single file. */
export const MAX_FILE_BYTES = 64 * 1024 * 1024;

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() !== "" ? value.trim() : undefined;
}

/** A path that stays a plain relative path: no root, no `..`, no backslashes, a photo or JSON at the end. */
export function safePath(path: string): boolean {
  if (path.startsWith("/") || path.includes("\\") || path.includes("\0")) return false;
  const parts = path.split("/");
  return parts.every((part) => part !== "" && part !== "." && part !== ".." && !part.startsWith(".")) && /\.(png|jpe?g|json)$/i.test(path);
}

function fileEntry(raw: unknown): ExampleFile | null {
  const file = raw as Record<string, unknown> | null;
  const path = text(file?.path);
  const size = typeof file?.size === "number" ? file.size : NaN;
  const sha256 = text(file?.sha256)?.toLowerCase();
  if (path === undefined || !safePath(path) || !(size > 0 && size <= MAX_FILE_BYTES) || sha256 === undefined || !/^[0-9a-f]{64}$/.test(sha256)) return null;
  return { path, size, sha256 };
}

/** The number in a photo's name, for capture order: dragon_10_rgb.png -> 10. */
function captureIndex(path: string): number {
  const match = /(\d+)(?!.*\d)/.exec(path.split("/").pop() ?? "");
  return match === null ? 0 : Number(match[1]);
}

/** Reads a manifest. Throws a sentence when it cannot be used; drops objects that are not safe to fetch. */
export function parseExampleManifest(json: unknown, manifestUrl: string, purpose = ""): ExampleManifest {
  const root = json as Record<string, unknown> | null;
  if (root === null || typeof root !== "object" || root.schema !== EXAMPLES_SCHEMA) throw new Error("This is not a list of example objects.");
  const base = new URL(text(root.base_url) ?? "./", manifestUrl);
  if (base.protocol !== "https:" && base.hostname !== "127.0.0.1" && base.hostname !== "localhost") throw new Error("Example objects are only downloaded over HTTPS.");
  const credit = text(root.attribution);
  const license = text(root.license);
  if (credit === undefined || license === undefined) throw new Error("The list of example objects says nothing about where the photos come from.");
  const attribution: Attribution = { source: text(root.source), doi: text(root.doi), license, text: credit };
  const calibrations = new Map<string, ExampleFile>();
  for (const raw of Array.isArray(root.calibration_files) ? root.calibration_files : []) {
    const file = fileEntry(raw);
    if (file !== null && file.path.endsWith(".json")) calibrations.set(file.path, file);
  }
  const objects: ExampleObject[] = [];
  for (const raw of Array.isArray(root.objects) ? root.objects : []) {
    const row = raw as Record<string, unknown>;
    const id = text(row.id) ?? text(row.name);
    if (id === undefined || !/^[A-Za-z0-9_-]{1,64}$/.test(id)) continue;
    const calibration = calibrations.get(text(row.calibration) ?? "");
    const files = (Array.isArray(row.files) ? row.files : []).map(fileEntry);
    if (calibration === undefined || files.length < 3 || files.some((file) => file === null || !/\.(png|jpe?g)$/i.test(file.path))) continue;
    const photos = (files as ExampleFile[]).sort((a, b) => captureIndex(a.path) - captureIndex(b.path) || a.path.localeCompare(b.path));
    const bytes = photos.reduce((sum, file) => sum + file.size, 0) + calibration.size;
    if (bytes > MAX_OBJECT_BYTES) continue;
    objects.push({ id, name: text(row.title) ?? id, photos: photos.length, bytes, calibration, files: photos });
  }
  return { baseUrl: base.href, purpose, attribution, objects };
}

/** What is kept with a run made from an example object. */
export function runAttribution(manifest: ExampleManifest, object: ExampleObject): string {
  return [
    `Photos: ${object.name}, ${object.photos} photos, downloaded from ${manifest.baseUrl}`,
    "",
    manifest.attribution.text,
    manifest.attribution.source !== undefined ? `Original source: ${manifest.attribution.source}` : "",
    "",
    manifest.purpose,
  ]
    .filter((line, index, all) => line !== "" || all[index - 1] !== "")
    .join("\n");
}

/** Hex SHA-256 of bytes, with the platform's crypto. */
export async function sha256(bytes: ArrayBuffer): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

export interface DownloadProgress {
  done: number;
  total: number;
  bytes: number;
  totalBytes: number;
}

/** Where downloaded files go: the app's data folder, or the browser's storage. */
export interface ExampleStore {
  /** Files of the object already stored and intact, by path. */
  have(object: ExampleObject): Promise<Set<string>>;
  put(object: ExampleObject, file: ExampleFile, bytes: ArrayBuffer): Promise<void>;
  /** Free bytes where the files go, or null when unknown. */
  free(): Promise<number | null>;
  remove(object: ExampleObject): Promise<void>;
}

/**
 * Downloads what is missing of an object, file by file, each checked against its size and
 * SHA-256 before it is stored. Cancels with `signal`. Throws a sentence on any mismatch.
 */
export async function downloadExample(
  manifest: ExampleManifest,
  object: ExampleObject,
  store: ExampleStore,
  options: { signal?: AbortSignal; onProgress?(progress: DownloadProgress): void; fetcher?: (url: string, init: RequestInit) => Promise<Response> } = {},
): Promise<void> {
  const fetcher = options.fetcher ?? ((url: string, init: RequestInit) => fetch(url, init));
  const have = await store.have(object);
  const missing = [object.calibration, ...object.files].filter((file) => !have.has(file.path));
  const missingBytes = missing.reduce((sum, file) => sum + file.size, 0);
  const free = await store.free();
  // Room for the photos and for the run made from them (about twice their size).
  if (free !== null && free < missingBytes * 3 + 256 * 1024 * 1024) {
    throw new Error(`Not enough free space: the photos need ${Math.round(missingBytes / 1e6)} MB and the run about twice that; ${Math.round(free / 1e6)} MB are free.`);
  }
  const total = object.files.length + 1;
  let done = total - missing.length;
  let bytes = object.bytes - missingBytes;
  options.onProgress?.({ done, total, bytes, totalBytes: object.bytes });
  for (const file of missing) {
    options.signal?.throwIfAborted();
    const response = await fetcher(new URL(file.path, manifest.baseUrl).href, { signal: options.signal });
    if (!response.ok) throw new Error(`${file.path}: the server answered ${response.status}.`);
    const body = await response.arrayBuffer();
    if (body.byteLength !== file.size) throw new Error(`${file.path}: ${body.byteLength} bytes arrived, ${file.size} were expected.`);
    if ((await sha256(body)) !== file.sha256) throw new Error(`${file.path}: the file is not the one the list describes (SHA-256 differs).`);
    await store.put(object, file, body);
    done += 1;
    bytes += file.size;
    options.onProgress?.({ done, total, bytes, totalBytes: object.bytes });
  }
}
