/** Shared fetch helpers for the sources. `fetch` is injected so tests need no network. */

import { t } from "../i18n";
import { EngineError, type FetchOptions } from "./types";

export type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

export const browserFetch: FetchLike = (input, init) => fetch(input, init);

/** `a/b c/d.png` -> `a/b%20c/d.png`. */
export function encodePath(path: string): string {
  return path.split("/").map(encodeURIComponent).join("/");
}

export function withTrailingSlash(url: string): string {
  return url.endsWith("/") ? url : url + "/";
}

/** Reads a response body, reporting progress when the body can be streamed. */
export async function readBytes(response: Response, options: FetchOptions = {}): Promise<ArrayBuffer> {
  const header = response.headers.get("Content-Length");
  const declared = header === null ? NaN : Number(header);
  // With Content-Encoding the declared length is the compressed one; do not present it as the total.
  const total = Number.isFinite(declared) && !response.headers.get("Content-Encoding") ? declared : undefined;
  if (options.onProgress === undefined || response.body === null) return response.arrayBuffer();
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let received = 0;
  let reported = 0;
  let yielded = Date.now();
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    received += value.byteLength;
    const now = Date.now();
    // A fast source (local disk, cache) answers every read at once; without a pause this
    // loop would run as one long chain and freeze the page for the whole download.
    if (now - yielded > 30) {
      await new Promise((resume) => setTimeout(resume, 0));
      yielded = Date.now();
    }
    if (now - reported > 100) {
      reported = now;
      options.onProgress(received, total);
    }
  }
  options.onProgress(received, total);
  const joined = new Uint8Array(received);
  let offset = 0;
  for (const chunk of chunks) {
    joined.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return joined.buffer;
}

/** Fetch that turns "could not connect" and non-2xx answers into `EngineError`s with a readable sentence. */
export async function request(fetcher: FetchLike, url: string, init?: RequestInit): Promise<Response> {
  let response: Response;
  try {
    response = await fetcher(url, init);
  } catch (problem) {
    if (problem instanceof DOMException && problem.name === "AbortError") throw problem;
    throw new EngineError(t("engine.unreachable"), 0);
  }
  if (!response.ok) throw new EngineError(await errorSentence(response), response.status);
  return response;
}

async function errorSentence(response: Response): Promise<string> {
  let detail = "";
  try {
    const body: unknown = await response.clone().json();
    const message = (body as { error?: unknown } | null)?.error;
    if (typeof message === "string") detail = message;
  } catch {
    // not JSON; the status has to do
  }
  if (response.status === 401) return t("engine.tokenRefused");
  if (response.status === 404) return detail === "" || detail === "not found" ? t("engine.notFound") : capitalise(detail) + ".";
  return detail !== "" ? detail : t("engine.requestFailed", { status: String(response.status) });
}

function capitalise(value: string): string {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

/** Asks for a file's size with HEAD. Undefined when the server does not say or does not answer HEAD. */
export async function headSize(fetcher: FetchLike, url: string, init: RequestInit = {}): Promise<number | undefined> {
  try {
    const response = await fetcher(url, { ...init, method: "HEAD" });
    if (!response.ok || response.headers.get("Content-Encoding")) return undefined;
    const header = response.headers.get("Content-Length");
    const size = header === null ? NaN : Number(header);
    return Number.isFinite(size) && size > 0 ? size : undefined;
  } catch (problem) {
    if (isAbort(problem)) throw problem;
    return undefined;
  }
}

export function isAbort(problem: unknown): boolean {
  return problem instanceof DOMException && problem.name === "AbortError";
}

export function describe(problem: unknown): string {
  return problem instanceof Error ? problem.message : String(problem);
}
