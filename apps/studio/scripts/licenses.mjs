// License audit of what the Studio app ships.
//
//   node scripts/licenses.mjs            # write docs/licenses.json and docs/THIRD-PARTY-LICENSES.md
//   node scripts/licenses.mjs --check    # exit 1 if a license is not allowed, or the committed files are stale
//   node scripts/licenses.mjs --notices dist/THIRD-PARTY-NOTICES.txt
//                                        # the license texts of everything shipped, for inclusion in a build
//
// What is looked at
//   npm    package-lock.json. Packages without the lock's "dev" flag are what the web bundle
//          can contain (runtime dependencies and theirs); the rest are build tools.
//   Rust   `cargo metadata --locked --filter-platform <target>` for each platform the shell is
//          built for. A crate is "linked" when it is reachable from the app through normal
//          dependencies without passing through a proc-macro crate; build scripts, proc
//          macros and what only they need are "build only".
//   Other  icons, fonts and the demo recording are listed by hand in ASSETS below.
//
// The output is deterministic (no dates, no paths), so CI can tell when it is out of date.
//   --store  audits only what the builds for Apple's stores link (sandboxed macOS variant, iOS),
//            lists the crates that implement encryption, and fails on a license outside the policy.
//
// Needs `cargo` on PATH and network access to the crates.io index the first time.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const JSON_FILE = join(root, "docs/licenses.json");
const MD_FILE = join(root, "docs/THIRD-PARTY-LICENSES.md");

/** Rust targets the shell is built for. The first two are the App Store platforms. */
export const TARGETS = {
  macos: "aarch64-apple-darwin",
  ios: "aarch64-apple-ios",
  windows: "x86_64-pc-windows-msvc",
  linux: "x86_64-unknown-linux-gnu",
  android: "aarch64-linux-android",
  // The engine for "This browser" in the web bundle: another crate (crates/dense/web) with its own lockfile.
  browser: "wasm32-unknown-unknown",
};

/** The manifest a platform is built from, where it is not the shell. */
const MANIFESTS = { browser: join(root, "../../crates/dense/web/Cargo.toml") };

// ---------------------------------------------------------------------------------------------
// Policy

/** Fine for App Store distribution: attribution only. */
const PERMISSIVE = new Set([
  "MIT", "MIT-0", "BSD-2-Clause", "BSD-3-Clause", "Apache-2.0", "ISC", "Zlib", "Unicode-3.0", "Unicode-DFS-2016",
  "BSL-1.0", "CC0-1.0", "0BSD",
]);
/** Fine, with obligations: keep notices, and make the source of these files available (also when modified). */
const NOTICE = new Set(["MPL-2.0"]);
/** Only acceptable when dynamically linked and replaceable: needs a decision each time. */
const WEAK_COPYLEFT = /^LGPL-/;
/** Exceptions that do not change the category of the license they are attached to. */
const HARMLESS_EXCEPTIONS = new Set(["LLVM-exception"]);

export const CATEGORIES = ["permissive", "notice", "decision", "blocker"];
const rank = (category) => CATEGORIES.indexOf(category);

function classifyId(id) {
  if (PERMISSIVE.has(id)) return "permissive";
  if (NOTICE.has(id)) return "notice";
  if (WEAK_COPYLEFT.test(id)) return "decision";
  // GPL, AGPL, SSPL, non-commercial, proprietary, and anything not recognised.
  return "blocker";
}

/**
 * Classifies an SPDX expression. With OR the most permissive branch may be chosen; with AND
 * every part applies, so the strictest counts. Returns the category and the ids relied on.
 */
export function classify(expression) {
  if (typeof expression !== "string" || expression.trim() === "") return { category: "blocker", chosen: "(no license stated)" };
  // "MIT/Apache-2.0" is the old Cargo way of writing OR.
  const tokens = expression.replace(/\//g, " OR ").match(/\(|\)|[^\s()]+/g) ?? [];
  let at = 0;
  const peek = () => tokens[at];
  const better = (a, b) => (rank(a.category) <= rank(b.category) ? a : b);
  const worse = (a, b) =>
    rank(a.category) >= rank(b.category)
      ? { category: a.category, chosen: `${a.chosen} AND ${b.chosen}` }
      : { category: b.category, chosen: `${a.chosen} AND ${b.chosen}` };
  function atom() {
    if (peek() === "(") {
      at += 1;
      const inner = or();
      if (peek() === ")") at += 1;
      return inner;
    }
    const id = tokens[at++] ?? "";
    let result = { category: classifyId(id.replace(/\+$/, "")), chosen: id };
    if (peek()?.toUpperCase() === "WITH") {
      at += 1;
      const exception = tokens[at++] ?? "";
      result = { category: HARMLESS_EXCEPTIONS.has(exception) ? result.category : "blocker", chosen: `${id} WITH ${exception}` };
    }
    return result;
  }
  function and() {
    let left = atom();
    while (peek()?.toUpperCase() === "AND") {
      at += 1;
      left = worse(left, atom());
    }
    return left;
  }
  function or() {
    let left = and();
    while (peek()?.toUpperCase() === "OR") {
      at += 1;
      left = better(left, and());
    }
    return left;
  }
  const result = or();
  return at < tokens.length ? { category: "blocker", chosen: `(cannot read "${expression}")` } : result;
}

// ---------------------------------------------------------------------------------------------
// Things that are not packages

const ASSETS = [
  {
    what: "App icons (src-tauri/icons)",
    origin: "CrispMesh's own artwork, the same set as apps/desktop (drawn from apps/desktop/src-tauri/icons/source.svg)",
    license: "project (AGPL-3.0-only, owner's copyright)",
  },
  {
    what: "Interface icons (src/ui/icons.tsx) and the favicon in index.html",
    origin: "Drawn for Studio as inline SVG paths; no icon set is included",
    license: "project (AGPL-3.0-only, owner's copyright)",
  },
  {
    what: "Fonts",
    origin: "None are shipped. The interface uses the system's fonts (system-ui and ui-monospace stacks)",
    license: "not applicable",
  },
  {
    what: "Demo recording (dist/demo, from tests/fixtures/dense-run-sphere)",
    origin:
      "A synthetic sphere: photos, masks, sheets and meshes were all generated by CrispMesh's own code (scripts/turntable_mesh/synthetic_scene.py and the pipeline). No third-party photos, scans or models",
    license: "project (AGPL-3.0-only, owner's copyright)",
  },
];

// ---------------------------------------------------------------------------------------------
// npm

function npmPackages() {
  const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
  const shipped = [];
  const dev = [];
  for (const [path, entry] of Object.entries(lock.packages)) {
    if (path === "" || entry.link) continue;
    const name = path.slice(path.lastIndexOf("node_modules/") + "node_modules/".length);
    let license = entry.license;
    // The lock usually carries the license; the installed package is the fallback.
    const manifest = join(root, path, "package.json");
    if (license === undefined && existsSync(manifest)) {
      const installed = JSON.parse(readFileSync(manifest, "utf8"));
      license = typeof installed.license === "string" ? installed.license : installed.license?.type;
    }
    const row = { name, version: entry.version, license: license ?? "", ...classify(license), path };
    (entry.dev || entry.devOptional || entry.optional ? dev : shipped).push(row);
  }
  const byName = (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version);
  return { shipped: shipped.sort(byName), dev: dev.sort(byName) };
}

// ---------------------------------------------------------------------------------------------
// Rust

function cargoMetadata(target, manifest = join(root, "src-tauri/Cargo.toml"), features = []) {
  const out = execFileSync(
    "cargo",
    ["metadata", "--format-version", "1", "--locked", "--filter-platform", target, "--manifest-path", manifest, ...features],
    { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] },
  );
  return JSON.parse(out);
}

/** Crates linked into the binary for one target, and crates only used while building. */
function crateSets(metadata) {
  const packages = new Map(metadata.packages.map((p) => [p.id, p]));
  const nodes = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));
  const isProcMacro = (id) => packages.get(id).targets.some((t) => t.kind.includes("proc-macro"));
  const rootId = metadata.resolve.root;
  const linked = new Set();
  const queue = [rootId];
  while (queue.length > 0) {
    const id = queue.pop();
    if (linked.has(id)) continue;
    linked.add(id);
    for (const dep of nodes.get(id).deps) {
      const normal = dep.dep_kinds.some((kind) => kind.kind === null);
      if (normal && !isProcMacro(dep.pkg)) queue.push(dep.pkg);
    }
  }
  // Everything else the resolver lists for this platform is needed only at build time,
  // except dev-dependencies, which are not part of a build at all.
  const reachable = new Set();
  const all = [rootId];
  while (all.length > 0) {
    const id = all.pop();
    if (reachable.has(id)) continue;
    reachable.add(id);
    for (const dep of nodes.get(id).deps) {
      if (dep.dep_kinds.some((kind) => kind.kind !== "dev")) all.push(dep.pkg);
    }
  }
  linked.delete(rootId);
  reachable.delete(rootId);
  return { linked, buildOnly: new Set([...reachable].filter((id) => !linked.has(id))), packages };
}

function rustCrates() {
  const rows = new Map();
  for (const [platform, target] of Object.entries(TARGETS)) {
    const { linked, buildOnly, packages } = crateSets(cargoMetadata(target, MANIFESTS[platform]));
    for (const [ids, role] of [[linked, "linked"], [buildOnly, "build"]]) {
      for (const id of ids) {
        const p = packages.get(id);
        const key = `${p.name}@${p.version}`;
        const row = rows.get(key) ?? {
          name: p.name,
          version: p.version,
          license: p.license ?? "",
          ...classify(p.license),
          repository: p.repository ?? "",
          linked: [],
          build: [],
          folder: dirname(p.manifest_path),
          // No registry source: a crate of this repository (crates/dense), the project's own code.
          own: p.source === null,
        };
        row[role].push(platform);
        rows.set(key, row);
      }
    }
  }
  const sorted = [...rows.values()].sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
  const third = sorted.filter((row) => !row.own);
  return {
    linked: third.filter((row) => row.linked.length > 0),
    build: third.filter((row) => row.linked.length === 0),
    own: sorted.filter((row) => row.own).map(({ name, version, license, linked }) => ({ name, version, license, linked })),
  };
}

/**
 * What the builds for Apple's stores link, and nothing else: the sandboxed macOS variant
 * (`--no-default-features --features native-engine`) and the iOS app.
 */
const STORE = {
  macos: { target: TARGETS.macos, features: ["--no-default-features", "--features", "native-engine"] },
  ios: { target: TARGETS.ios, features: [] },
};

/** Crates that implement ciphers, key exchange or TLS: what an export-compliance answer has to know about. */
const ENCRYPTION = /^(ring|rustls|rustls-.*|openssl|openssl-sys|native-tls|aws-lc-rs|aws-lc-sys|boring|boring-sys|aes|aes-gcm|chacha20|chacha20poly1305|salsa20|rsa|ed25519.*|x25519.*|curve25519.*|p256|p384|k256|ecdsa|des|blowfish|twofish|cbc|ctr|gcm|sodiumoxide|libsodium-sys|orion|age|pgp|sequoia.*|security-framework|security-framework-sys|schannel|hyper-tls|tokio-rustls|tokio-native-tls|quinn.*|webpki.*)$/;

function storeAudit() {
  const npm = npmPackages();
  const rows = new Map();
  for (const [platform, { target, features }] of Object.entries(STORE)) {
    const { linked, packages } = crateSets(cargoMetadata(target, undefined, features));
    for (const id of linked) {
      const p = packages.get(id);
      const key = `${p.name}@${p.version}`;
      const row = rows.get(key) ?? { name: p.name, version: p.version, license: p.license ?? "", ...classify(p.license), linked: [], own: p.source === null };
      row.linked.push(platform);
      rows.set(key, row);
    }
  }
  const crates = [...rows.values()].sort((a, b) => a.name.localeCompare(b.name));
  const third = [...npm.shipped, ...crates.filter((row) => !row.own)];
  const own = crates.filter((row) => row.own);
  const worst = CATEGORIES[Math.max(0, ...third.map((row) => rank(row.category)))];
  const verdict = { permissive: "clean", notice: "clean with obligations", decision: "needs a decision", blocker: "blocked" }[worst];
  console.log("Builds for Apple's stores: macOS (sandboxed variant, native-engine only) and iOS.");
  console.log(`third-party: ${npm.shipped.length} npm packages, ${crates.length - own.length} crates linked: ${verdict}`);
  console.log("  " + Object.entries(tally(third)).map(([license, count]) => `${license}: ${count}`).join(", "));
  const obligations = third.filter((row) => row.category === "notice");
  if (obligations.length > 0) console.log("  with obligations (notices kept, source offered in the app's license screen): " + obligations.map((row) => `${row.name} ${row.version} [${row.linked?.join("+") ?? "web"}]`).join(", "));
  console.log("the project's own code in these builds: " + own.map((row) => `${row.name} ${row.version} (${row.license})`).join(", "));
  const notice = join(root, "../../NOTICE");
  const permission = existsSync(notice) && /additional permission \(AGPL-3\.0 section 7\)/i.test(readFileSync(notice, "utf8"));
  console.log(permission
    ? "  Basis for the project's own AGPL code in store builds: the section 7 additional permission in NOTICE (store binaries published by the copyright holder)."
    : "  NOTICE with the store permission is missing: the project's own AGPL code has no stated basis for store distribution.");
  if (!permission) process.exitCode = 1;
  const encryption = crates.filter((row) => ENCRYPTION.test(row.name));
  console.log("crates that implement encryption or TLS: " + (encryption.length === 0 ? "none" : encryption.map((row) => `${row.name} ${row.version} [${row.linked.join("+")}]`).join(", ")));
  const bad = third.filter((row) => rank(row.category) >= rank("decision"));
  if (bad.length > 0) {
    console.error("Licenses outside the policy:\n  " + bad.map((row) => `${row.name}@${row.version}: ${row.license || "(none)"} -> ${row.category}`).join("\n  "));
    process.exitCode = 1;
  }
}

// ---------------------------------------------------------------------------------------------
// Report

function tally(rows, key = "chosen") {
  const counts = {};
  for (const row of rows) counts[row[key]] = (counts[row[key]] ?? 0) + 1;
  return Object.fromEntries(Object.entries(counts).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])));
}

function audit() {
  const npm = npmPackages();
  const rust = rustCrates();
  const strip = ({ path, folder, own, ...row }) => row;
  const shipped = [...npm.shipped, ...rust.linked];
  const appStore = [...npm.shipped, ...rust.linked.filter((row) => row.linked.includes("macos") || row.linked.includes("ios"))];
  const worst = (rows) => CATEGORIES[Math.max(0, ...rows.map((row) => rank(row.category)))];
  const verdict = (rows) => ({ permissive: "clean", notice: "clean with obligations", decision: "needs a decision", blocker: "blocked" })[worst(rows)];
  const data = {
    about: "Generated by apps/studio/scripts/licenses.mjs. Do not edit; run `npm run licenses`.",
    policy: {
      permissive: [...PERMISSIVE].sort(),
      notice: [...NOTICE].sort(),
      decision: "LGPL-*: only if dynamically linked and replaceable",
      blocker: "GPL, AGPL, SSPL, non-commercial, unknown or unstated, from third parties",
    },
    result: {
      everything_shipped: verdict(shipped),
      app_store_platforms: verdict(appStore),
      not_permissive: shipped.filter((row) => row.category !== "permissive").map((row) => `${row.name}@${row.version}: ${row.license}`),
    },
    counts: {
      npm_shipped: tally(npm.shipped),
      rust_linked_macos_or_ios: tally(rust.linked.filter((row) => row.linked.includes("macos") || row.linked.includes("ios"))),
      rust_linked_any_platform: tally(rust.linked),
      rust_build_only: tally(rust.build),
      npm_dev_only: tally(npm.dev),
    },
    targets: TARGETS,
    npm: { shipped: npm.shipped.map(strip), dev: npm.dev.map(strip) },
    rust: { own: rust.own, linked: rust.linked.map(strip), build: rust.build.map(strip) },
    assets: ASSETS,
  };
  return { data, npm, rust };
}

function markdown(data) {
  const table = (head, rows) => [`| ${head.join(" | ")} |`, `| ${head.map(() => "---").join(" | ")} |`, ...rows.map((r) => `| ${r.join(" | ")} |`)].join("\n");
  const counts = (object) => table(["License relied on", "Packages"], Object.entries(object).map(([k, v]) => [k, String(v)]));
  const mpl = data.rust.linked.filter((row) => row.category === "notice");
  const flagged = [...data.npm.shipped, ...data.rust.linked].filter((row) => rank(row.category) >= rank("decision"));
  const lines = [
    "# Third-party licenses in CrispMesh",
    "",
    "Generated by `scripts/licenses.mjs` (`npm run licenses`); the same data is in `licenses.json`. CI fails when a",
    "license outside the policy appears or when these files are out of date. Do not edit by hand.",
    "",
    "## Result",
    "",
    `- Everything shipped, all platforms: **${data.result.everything_shipped}**.`,
    `- The App Store platforms (macOS and iOS): **${data.result.app_store_platforms}**.`,
    "",
    flagged.length === 0
      ? "No third-party GPL, AGPL, LGPL, SSPL, non-commercial or unlicensed code is shipped."
      : `**Flagged:** ${flagged.map((row) => `${row.name} ${row.version} (${row.license})`).join(", ")}.`,
    "",
    "Obligations that come with this result:",
    "",
    "1. **Attribution.** The permissive licenses (MIT, BSD, Apache-2.0, ISC, Zlib, Unicode, BSL-1.0, CC0, 0BSD) require",
    "   their copyright notices and license texts to accompany the app. `npm run licenses -- --notices <file>` collects",
    "   them; release builds put the result into the app as `THIRD-PARTY-NOTICES.txt`.",
    mpl.length > 0
      ? `2. **MPL-2.0** (${mpl.length} crates, listed below): keep their notices, and make the source of those files available to` +
        "\n   recipients, including any modifications to them. Studio uses them unmodified from crates.io; linking them into a" +
        "\n   larger work under other terms is permitted by the license (section 3.3)."
      : "2. No MPL-2.0 code is shipped.",
    "3. **Apache-2.0 NOTICE files**, where a crate has one, are part of the collected notices.",
    "",
    "## Basis for App Store distribution",
    "",
    "CrispMesh's own code is licensed AGPL-3.0-only. The owner is its copyright holder and can therefore also distribute",
    "it through the App Store, whose terms would otherwise conflict with the AGPL for anyone else. **This does not extend",
    "to third-party copyleft code**: no GPL or AGPL code from others may be in a store build. That is what this audit",
    "checks. Contributions by others to CrispMesh's own code would need the same right granted to the owner.",
    "",
    "What the Studio apps contain and do not contain:",
    "",
    "- They contain the web front end (this package's runtime npm dependencies), the Tauri shell, and the project's own",
    "  reconstruction engine (`crates/dense`, Rust, WebGPU through `wgpu`) with the crates it depends on, all listed below.",
    "- They contain **no Python, no PyTorch, no NumPy/SciPy/OpenCV, no AliceVision, no SAM, no OpenMVS, no COLMAP** and no",
    "  other third-party engine. The optional \"External Python engine\" of the desktop app runs software the user installed",
    "  separately; it is not part of any build, and the App Store variant cannot start it at all.",
    "- The system web view (WKWebView, WebView2, WebKitGTK) is part of the operating system, not of the app.",
    "- One exception outside the App Store: the Linux AppImage, as Tauri's bundler builds it, carries the build machine's",
    "  WebKitGTK and GTK libraries (LGPL) as separate shared libraries inside the image, where they can be replaced. The",
    "  `.deb` depends on the system's libraries instead. This audit does not list those system libraries.",
    "",
    "## The project's own crates",
    "",
    "Linked into the app and not third-party: CrispMesh's own code, AGPL-3.0-only, covered by the basis above.",
    "",
    table(["Crate", "Version", "License", "Linked on"], data.rust.own.map((row) => [row.name, row.version, row.license, row.linked.join(", ")])),
    "",
    "## Assets",
    "",
    table(["What", "Where it comes from", "License"], data.assets.map((a) => [a.what, a.origin, a.license])),
    "",
    "## Counts",
    "",
    "### npm packages in the web bundle",
    "",
    counts(data.counts.npm_shipped),
    "",
    "### Rust crates linked into the macOS or iOS app",
    "",
    counts(data.counts.rust_linked_macos_or_ios),
    "",
    "### Rust crates linked on any platform (macOS, iOS, Windows, Linux, Android, and the engine for browsers in the web bundle)",
    "",
    counts(data.counts.rust_linked_any_platform),
    "",
    "Where a crate offers a choice (`MIT OR Apache-2.0`), the first permissive option is the one relied on.",
    "",
    "## npm packages in the web bundle",
    "",
    table(["Package", "Version", "License"], data.npm.shipped.map((row) => [row.name, row.version, row.license])),
    "",
    mpl.length > 0 ? "## MPL-2.0 crates (source availability applies to these)\n" : "",
    mpl.length > 0
      ? table(["Crate", "Version", "Linked on", "Source"], mpl.map((row) => [row.name, row.version, row.linked.join(", "), row.repository || `https://crates.io/crates/${row.name}`]))
      : "",
    "",
    "## Rust crates linked into the app",
    "",
    table(["Crate", "Version", "License", "Linked on"], data.rust.linked.map((row) => [row.name, row.version, row.license || "(none stated)", row.linked.join(", ")])),
    "",
    "## Not shipped: build-time only",
    "",
    `Rust crates used only while compiling (build scripts, proc macros and what only they need): ${data.rust.build.length}.`,
    "",
    counts(data.counts.rust_build_only),
    "",
    `npm development tools (Vite, TypeScript, Vitest, Playwright, the Tauri CLI and their dependencies): ${data.npm.dev.length}.`,
    "",
    counts(data.counts.npm_dev_only),
    "",
    "The full lists are in `licenses.json` (`rust.build`, `npm.dev`). To list transitive dependencies yourself:",
    "`npm ls --all --omit=dev` and `cargo tree --manifest-path src-tauri/Cargo.toml -e normal --target <triple>`.",
    "",
  ];
  return lines.join("\n");
}

/** Copyright and license files of everything shipped, as one text file. */
function notices(npm, rust, file) {
  const wanted = /^(licen[sc]e|copying|notice|copyright|unlicense)/i;
  // The project's own license and the store permission (NOTICE at the repository root) come first.
  const own = join(root, "../../NOTICE");
  const parts = [
    ...(existsSync(own) ? [readFileSync(own, "utf8").trim(), "", "=".repeat(100), ""] : []),
    "CrispMesh: third-party notices",
    "",
    "CrispMesh itself is licensed AGPL-3.0-only. It includes the following third-party",
    "software, each under its own license as reproduced below.",
    "",
    "Source code of CrispMesh, with the exact versions of everything listed here",
    "(package-lock.json, Cargo.lock): https://github.com/CrispStrobe/CrispMesh",
    "",
  ];
  const mpl = rust.linked.filter((row) => /MPL-2\.0/.test(row.chosen));
  if (mpl.length > 0) {
    parts.push(
      "Mozilla Public License 2.0. The following parts are under the MPL-2.0 and are used",
      "unmodified. Their source code is available from where they are published:",
      "",
      ...mpl.map((row) => `  ${row.name} ${row.version}    https://crates.io/crates/${row.name}/${row.version}`),
      "",
    );
  }
  const add = (title, license, folder) => {
    parts.push("=".repeat(100), `${title}    (${license})`, "=".repeat(100), "");
    const files = existsSync(folder) ? readdirSync(folder).filter((name) => wanted.test(name)).sort() : [];
    if (files.length === 0) parts.push(`(The package contains no license file. It is licensed: ${license}.)`, "");
    for (const name of files) parts.push(`--- ${name} ---`, readFileSync(join(folder, name), "utf8").trim(), "");
  };
  for (const row of npm.shipped) add(`${row.name} ${row.version} (npm)`, row.license, join(root, row.path));
  for (const row of rust.linked) add(`${row.name} ${row.version} (crates.io)`, row.license, row.folder);
  mkdirSync(dirname(resolve(file)), { recursive: true });
  writeFileSync(resolve(file), parts.join("\n"));
  console.log(`wrote ${file}: ${npm.shipped.length} npm packages, ${rust.linked.length} crates`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url) && process.argv.includes("--store")) {
  storeAudit();
} else if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { data, npm, rust } = audit();
  const json = JSON.stringify(data, null, 2) + "\n";
  const md = markdown(data);
  const args = process.argv.slice(2);
  const bad = [...data.npm.shipped, ...data.rust.linked].filter((row) => rank(row.category) >= rank("decision"));
  console.log(`shipped everywhere: ${data.result.everything_shipped}; macOS and iOS: ${data.result.app_store_platforms}`);
  console.log(`npm in bundle: ${data.npm.shipped.length}; crates linked: ${data.rust.linked.length}; build-only crates: ${data.rust.build.length}; npm dev: ${data.npm.dev.length}`);
  if (args.includes("--notices")) {
    notices(npm, rust, args[args.indexOf("--notices") + 1]);
  } else if (args.includes("--check")) {
    const stale = [];
    if (!existsSync(JSON_FILE) || readFileSync(JSON_FILE, "utf8") !== json) stale.push("docs/licenses.json");
    if (!existsSync(MD_FILE) || readFileSync(MD_FILE, "utf8") !== md) stale.push("docs/THIRD-PARTY-LICENSES.md");
    if (stale.length > 0) {
      console.error(`Out of date: ${stale.join(", ")}. Run \`npm run licenses\` and commit the result.`);
      process.exitCode = 1;
    }
  } else {
    writeFileSync(JSON_FILE, json);
    writeFileSync(MD_FILE, md);
    console.log("wrote docs/licenses.json and docs/THIRD-PARTY-LICENSES.md");
  }
  if (bad.length > 0) {
    console.error("Licenses outside the policy:\n  " + bad.map((row) => `${row.name}@${row.version}: ${row.license || "(none)"} -> ${row.category}`).join("\n  "));
    process.exitCode = 1;
  }
}
