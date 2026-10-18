import { browserFetch, type FetchLike } from "../sources/transport";
import type { Shell } from "./shell";
import { t } from "../i18n";

/**
 * In the apps: requests to an engine at a plain `http://` address go through the app (`remote_fetch`, the app's
 * own connection), not the web view, whose secure page may block plain HTTP to the local network. Everything else
 * (https) stays with the web view. Same shape as `fetch` for the engine client.
 */
export function remoteFetcher(shell: Pick<Shell, "bridge">): FetchLike {
  return async (input, init) => {
    if (!input.startsWith("http://")) return browserFetch(input, init);
    const signal = init?.signal ?? undefined;
    const aborted = () => new DOMException(t("connection.remoteFetch.aborted"), "AbortError");
    if (signal?.aborted) throw aborted();
    const method = (init?.method ?? "GET").toUpperCase();
    const headers: Record<string, string> = {};
    new Headers(init?.headers).forEach((value, name) => (headers[name] = value));
    const raw = init?.body;
    let body: Uint8Array;
    if (raw === undefined || raw === null) body = new Uint8Array();
    else if (typeof raw === "string") body = new TextEncoder().encode(raw);
    else if (raw instanceof Blob) body = new Uint8Array(await raw.arrayBuffer());
    else if (raw instanceof ArrayBuffer) body = new Uint8Array(raw);
    else if (ArrayBuffer.isView(raw)) body = new Uint8Array(raw.buffer, raw.byteOffset, raw.byteLength);
    else throw new TypeError(t("connection.remoteFetch.body"));
    const framed = await shell.bridge<ArrayBuffer | Uint8Array | number[]>("remote_fetch", body, {
      headers: {
        "x-url": encodeURIComponent(input),
        "x-method": encodeURIComponent(method),
        "x-headers": encodeURIComponent(JSON.stringify(headers)),
      },
    });
    if (signal?.aborted) throw aborted();
    return unframe(framed, method);
  };
}

/** The app's answer (status, header JSON, body; see remote.rs `frame`) as a Response. */
export function unframe(framed: ArrayBuffer | Uint8Array | number[], method = "GET"): Response {
  const bytes = framed instanceof Uint8Array ? framed : framed instanceof ArrayBuffer ? new Uint8Array(framed) : Uint8Array.from(framed);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const status = view.getUint16(0);
  const length = view.getUint32(2);
  const headers = JSON.parse(new TextDecoder().decode(bytes.subarray(6, 6 + length))) as Record<string, string>;
  const body = bytes.slice(6 + length);
  const empty = method === "HEAD" || status === 204 || status === 304;
  return new Response(empty ? null : body, { status, headers });
}
