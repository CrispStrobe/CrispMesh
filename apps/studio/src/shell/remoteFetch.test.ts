import { describe, expect, it } from "vitest";
import { remoteFetcher, unframe } from "./remoteFetch";

/** What remote.rs `frame` writes. */
function frame(status: number, headers: Record<string, string>, body: Uint8Array): Uint8Array {
  const json = new TextEncoder().encode(JSON.stringify(headers));
  const out = new Uint8Array(6 + json.length + body.length);
  const view = new DataView(out.buffer);
  view.setUint16(0, status);
  view.setUint32(2, json.length);
  out.set(json, 6);
  out.set(body, 6 + json.length);
  return out;
}

describe("requests to an http engine through the app", () => {
  it("sends method, address, headers and body to the app and reads its answer", async () => {
    const calls: { command: string; args: unknown; headers: Record<string, string> }[] = [];
    const shell = {
      bridge: async <T,>(command: string, args?: unknown, options?: { headers: Record<string, string> }) => {
        calls.push({ command, args, headers: options?.headers ?? {} });
        return frame(201, { "content-type": "application/json" }, new TextEncoder().encode('{"path":"c/1.jpg"}')) as T;
      },
    };
    const fetcher = remoteFetcher(shell);
    const photo = new File([new Uint8Array([1, 2, 3])], "1.jpg");
    const response = await fetcher("http://192.168.1.20:8766/api/uploads?folder=c&name=1.jpg", {
      method: "POST",
      headers: { Authorization: "Bearer t0k3n" },
      body: photo,
    });
    expect(response.status).toBe(201);
    expect(await response.json()).toEqual({ path: "c/1.jpg" });
    expect(calls[0]!.command).toBe("remote_fetch");
    expect([...(calls[0]!.args as Uint8Array)]).toEqual([1, 2, 3]);
    expect(decodeURIComponent(calls[0]!.headers["x-url"]!)).toBe("http://192.168.1.20:8766/api/uploads?folder=c&name=1.jpg");
    expect(decodeURIComponent(calls[0]!.headers["x-method"]!)).toBe("POST");
    expect(JSON.parse(decodeURIComponent(calls[0]!.headers["x-headers"]!))).toEqual({ authorization: "Bearer t0k3n" });
  });

  it("leaves https to the web view and answers HEAD and 204 without a body", async () => {
    const shell = { bridge: async <T,>() => frame(200, {}, new Uint8Array()) as T };
    const original = globalThis.fetch;
    let seen = "";
    globalThis.fetch = (async (input: string) => {
      seen = input;
      return new Response("ok");
    }) as typeof fetch;
    try {
      await remoteFetcher(shell)("https://engine.example.org/api/health");
    } finally {
      globalThis.fetch = original;
    }
    expect(seen).toBe("https://engine.example.org/api/health");
    expect(unframe(frame(204, {}, new Uint8Array()), "POST").status).toBe(204);
    const head = unframe(frame(200, { "content-length": "52000000" }, new Uint8Array()), "HEAD");
    expect(head.headers.get("content-length")).toBe("52000000");
  });
});
