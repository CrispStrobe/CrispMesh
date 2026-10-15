import { cpSync, createReadStream, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, extname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";
import { defineConfig } from "vitest/config";

const here = dirname(fileURLToPath(import.meta.url));
const fixture = resolve(here, "../../tests/fixtures/dense-run-sphere");
// The engine for "This browser": the WebAssembly package of crates/dense/web, built by
// crates/dense/web/build.sh into pkg/. When it has not been built, the app still builds
// and that mode says it is not included.
const enginePackage = resolve(here, "../../crates/dense/web");
const engineCore = ["crispmesh-dense.js", "pkg/crispmesh_dense_web.js", "pkg/crispmesh_dense_web_bg.wasm"];
const engineBuilt = engineCore.every((name) => existsSync(join(enginePackage, name)));
/** Every file under `folder` (relative to the package), recursively. */
function filesUnder(folder: string): string[] {
  const start = join(enginePackage, folder);
  if (!existsSync(start)) return [];
  return readdirSync(start, { recursive: true })
    .map((name) => join(folder, String(name)).split(sep).join("/"))
    .filter((name) => statSync(join(enginePackage, name)).isFile() && !name.endsWith(".d.ts"));
}
// The threaded package (build.sh threads) runs where the page is cross-origin isolated; on hosts
// that cannot send the headers (GitHub Pages) coi-worker.js isolates the page with a service worker.
const threadFiles = existsSync(join(enginePackage, "pkg-threads/crispmesh_dense_web_bg.wasm")) ? filesUnder("pkg-threads") : [];
const isolation = threadFiles.length > 0 && existsSync(join(enginePackage, "coi-worker.js")) ? ["coi-worker.js"] : [];
const engineFiles = [...engineCore, ...threadFiles];
// Inside the app (Tauri sets TAURI_ENV_PLATFORM for its build) the engine is native: no service worker there.
const webBuild = process.env.TAURI_ENV_PLATFORM === undefined;

const types: Record<string, string> = {
  ".png": "image/png",
  ".json": "application/json",
  ".jsonl": "application/x-ndjson",
  ".stl": "model/stl",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
};

/** Serves files of `folder` under `/<mount>/` in dev, and copies them (or `only`) into `dist/<mount>` on build. */
function mounted(name: string, mount: string, folder: string, only: string[] | null, required: boolean): Plugin {
  return {
    name,
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const path = (request.url ?? "").split("?")[0] ?? "";
        const at = path.indexOf(`/${mount}/`);
        if (at < 0) return next();
        const relative = decodeURIComponent(path.slice(at + mount.length + 2));
        const target = normalize(join(folder, relative));
        if (!target.startsWith(folder + sep) || (only !== null && !only.includes(relative)) || !existsSync(target) || !statSync(target).isFile()) return next();
        response.setHeader("Content-Type", types[extname(target)] ?? "application/octet-stream");
        response.setHeader("Content-Length", statSync(target).size);
        createReadStream(target).pipe(response);
      });
    },
    closeBundle() {
      const out = resolve(here, "dist");
      if (!existsSync(out)) return;
      if (only === null) {
        if (!existsSync(folder)) {
          if (required) throw new Error(`${name}: missing ${folder}`);
          return;
        }
        cpSync(folder, join(out, mount), { recursive: true });
        return;
      }
      for (const file of only) {
        mkdirSync(dirname(join(out, mount, file)), { recursive: true });
        cpSync(join(folder, file), join(out, mount, file));
      }
    },
  };
}

// Lens calibrations that come with the app, offered by the photos start in "This browser".
const calibrations = resolve(here, "../../scripts/turntable_mesh/calibrations");
function calibrationIndex(): string {
  const rows = existsSync(calibrations)
    ? readdirSync(calibrations)
        .filter((name) => name.endsWith(".json"))
        .sort()
        .filter((name) => (JSON.parse(readFileSync(join(calibrations, name), "utf8")) as { schema?: string }).schema === "crispmesh_lens_calibration_v1")
        .map((name) => ({ file: name, label: name.replace(/\.json$/, "") }))
    : [];
  return JSON.stringify(rows, null, 2) + "\n";
}
const calibrationsPlugin: Plugin = {
  name: "crispmesh-calibrations",
  configureServer(server) {
    server.middlewares.use((request, response, next) => {
      const path = (request.url ?? "").split("?")[0] ?? "";
      const at = path.indexOf("/calibrations/");
      if (at < 0) return next();
      const name = decodeURIComponent(path.slice(at + "/calibrations/".length));
      if (name === "index.json") {
        response.setHeader("Content-Type", "application/json");
        return response.end(calibrationIndex());
      }
      const target = join(calibrations, name);
      if (name.includes("/") || !name.endsWith(".json") || !existsSync(target)) return next();
      response.setHeader("Content-Type", "application/json");
      createReadStream(target).pipe(response);
    });
  },
  closeBundle() {
    const out = resolve(here, "dist/calibrations");
    if (!existsSync(resolve(here, "dist"))) return;
    mkdirSync(out, { recursive: true });
    for (const row of JSON.parse(calibrationIndex()) as { file: string }[]) cpSync(join(calibrations, row.file), join(out, row.file));
    writeFileSync(join(out, "index.json"), calibrationIndex());
  },
};

export default defineConfig({
  // Relative asset URLs: the build works from any base path (engine --static, GitHub Pages, Tauri).
  base: "./",
  clearScreen: false,
  define: { __BROWSER_ENGINE__: JSON.stringify(engineBuilt) },
  plugins: [
    // The recorded sphere run as `demo/`.
    mounted("crispmesh-demo-bundle", "demo", fixture, null, true),
    calibrationsPlugin,
    ...(engineBuilt ? [mounted("crispmesh-browser-engine", "engine", enginePackage, engineFiles, false)] : []),
    ...(engineBuilt && webBuild && isolation.length > 0
      ? [
          {
            name: "crispmesh-cross-origin-isolation",
            // First thing on the page: it registers the service worker and reloads once if needed.
            // It sits next to index.html, because a service worker controls only pages at or below
            // its own folder.
            transformIndexHtml: () => [{ tag: "script", attrs: { src: "./coi-worker.js" }, injectTo: "head-prepend" as const }],
            configureServer(server) {
              server.middlewares.use((request, response, next) => {
                if (!(request.url ?? "").split("?")[0]!.endsWith("/coi-worker.js")) return next();
                response.setHeader("Content-Type", "text/javascript");
                createReadStream(join(enginePackage, "coi-worker.js")).pipe(response);
              });
            },
            closeBundle() {
              if (existsSync(resolve(here, "dist"))) cpSync(join(enginePackage, "coi-worker.js"), resolve(here, "dist/coi-worker.js"));
            },
          } satisfies Plugin,
        ]
      : []),
  ],
  server: { host: "127.0.0.1", port: 1430, strictPort: true },
  preview: { host: "127.0.0.1", port: 1431, strictPort: true },
  build: { target: "es2022", chunkSizeWarningLimit: 900 },
  worker: { format: "es" },
  test: { include: ["src/**/*.test.ts"], environment: "node" },
});
