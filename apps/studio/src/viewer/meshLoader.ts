/**
 * Downloads and parses meshes without blocking the page: parsing happens in a worker, or
 * in slices on the main thread if a worker cannot be started. Parsed meshes are kept in a
 * small least-recently-used cache (CPU memory only; GPU buffers belong to the viewer and
 * exist only for the mesh on screen).
 */

import { meshBytes, parseBinaryStlChunked, StlError, type ParsedMesh } from "../core/stl";
import type { RunSource } from "../sources/types";
import type { StlRequest, StlResponse } from "./stl.worker";
import { t } from "../i18n";

export interface LoadProgress {
  phase: "download" | "parse";
  received?: number;
  total?: number;
}

const CACHE_BUDGET = 192 * 1024 * 1024;

export class MeshLoader {
  private worker: Worker | null | undefined;
  private nextId = 1;
  private waiting = new Map<number, { resolve: (mesh: ParsedMesh) => void; reject: (problem: Error) => void }>();
  private cache = new Map<string, ParsedMesh>();
  private cached = 0;

  constructor(private readonly source: RunSource) {}

  has(path: string): boolean {
    return this.cache.has(path);
  }

  async load(path: string, signal: AbortSignal, onProgress?: (progress: LoadProgress) => void): Promise<ParsedMesh> {
    const hit = this.cache.get(path);
    if (hit !== undefined) {
      // Refresh its place in the eviction order.
      this.cache.delete(path);
      this.cache.set(path, hit);
      return hit;
    }
    onProgress?.({ phase: "download" });
    const buffer = await this.source.fetchBytes(path, {
      signal,
      onProgress: (received, total) => onProgress?.({ phase: "download", received, total }),
    });
    signal.throwIfAborted();
    onProgress?.({ phase: "parse" });
    const mesh = await this.parse(buffer);
    signal.throwIfAborted();
    this.remember(path, mesh);
    return mesh;
  }

  dispose(): void {
    this.worker?.terminate();
    this.worker = null;
    for (const { reject } of this.waiting.values()) reject(new DOMException("Loader disposed", "AbortError"));
    this.waiting.clear();
    this.cache.clear();
    this.cached = 0;
  }

  private remember(path: string, mesh: ParsedMesh): void {
    const size = meshBytes(mesh);
    if (size > CACHE_BUDGET) return;
    this.cache.set(path, mesh);
    this.cached += size;
    for (const [key, old] of this.cache) {
      if (this.cached <= CACHE_BUDGET) break;
      if (key === path) continue;
      this.cache.delete(key);
      this.cached -= meshBytes(old);
    }
  }

  private parse(buffer: ArrayBuffer): Promise<ParsedMesh> {
    const worker = this.startWorker();
    if (worker === null) return this.parseHere(buffer);
    return new Promise<ParsedMesh>((resolve, reject) => {
      const id = this.nextId++;
      this.waiting.set(id, { resolve, reject });
      const message: StlRequest = { id, buffer };
      worker.postMessage(message, [buffer]);
    });
  }

  private parseHere(buffer: ArrayBuffer): Promise<ParsedMesh> {
    return parseBinaryStlChunked(buffer, () => new Promise((resolve) => setTimeout(resolve, 0)));
  }

  private startWorker(): Worker | null {
    if (this.worker !== undefined) return this.worker;
    try {
      const worker = new Worker(new URL("./stl.worker.ts", import.meta.url), { type: "module" });
      worker.onmessage = (message: MessageEvent<StlResponse>) => {
        const answer = message.data;
        const waiting = this.waiting.get(answer.id);
        if (waiting === undefined) return;
        this.waiting.delete(answer.id);
        if (answer.ok) waiting.resolve(answer.mesh);
        else waiting.reject(new StlError(answer.error));
      };
      worker.onerror = () => {
        // The worker itself failed (blocked by policy, script not loadable): fail what is
        // waiting and parse on the main thread from now on.
        this.worker = null;
        worker.terminate();
        for (const { reject } of this.waiting.values()) {
          reject(new StlError(t("run.loader.background")));
        }
        this.waiting.clear();
      };
      this.worker = worker;
    } catch {
      this.worker = null;
    }
    return this.worker;
  }
}
