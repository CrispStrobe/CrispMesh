// Sets one version in every place Studio states it, so a release cannot ship with
// package.json, tauri.conf.json and Cargo.toml disagreeing.
//
//   node scripts/set-version.mjs 1.2.3        # write
//   node scripts/set-version.mjs --check      # exit 1 if the files disagree
//
// Files: package.json, package-lock.json, src-tauri/tauri.conf.json,
// src-tauri/Cargo.toml, src-tauri/Cargo.lock (the crispmesh-studio entry).

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const file = (name) => resolve(root, name);
const read = (name) => readFileSync(file(name), "utf8");
const SEMVER = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/;

const CARGO_TOML = /(\[package\][^[]*?\r?\nversion = ")([^"]+)(")/;
const CARGO_LOCK = /(name = "crispmesh-studio"\r?\nversion = ")([^"]+)(")/;

function current() {
  return {
    "package.json": JSON.parse(read("package.json")).version,
    "package-lock.json": JSON.parse(read("package-lock.json")).version,
    "package-lock.json (root package)": JSON.parse(read("package-lock.json")).packages[""].version,
    "src-tauri/tauri.conf.json": JSON.parse(read("src-tauri/tauri.conf.json")).version,
    "src-tauri/Cargo.toml": CARGO_TOML.exec(read("src-tauri/Cargo.toml"))?.[2],
    "src-tauri/Cargo.lock": CARGO_LOCK.exec(read("src-tauri/Cargo.lock"))?.[2],
  };
}

const argument = process.argv[2];
if (argument === "--check" || argument === undefined) {
  const versions = current();
  const distinct = new Set(Object.values(versions));
  for (const [name, version] of Object.entries(versions)) console.log(`${version}  ${name}`);
  if (distinct.size !== 1 || distinct.has(undefined)) {
    console.error("The versions disagree.");
    process.exit(1);
  }
  process.exit(0);
}

if (!SEMVER.test(argument)) {
  console.error(`"${argument}" is not a version like 1.2.3, 1.2.3-rc.1 or 0.0.0-dev+abc1234.`);
  process.exit(1);
}

function setJson(name, change) {
  const data = JSON.parse(read(name));
  change(data);
  writeFileSync(file(name), JSON.stringify(data, null, 2) + "\n");
}

function setText(name, pattern) {
  const text = read(name);
  if (!pattern.test(text)) {
    console.error(`${name}: the version line was not found.`);
    process.exit(1);
  }
  writeFileSync(file(name), text.replace(pattern, `$1${argument}$3`));
}

setJson("package.json", (data) => {
  data.version = argument;
});
setJson("package-lock.json", (data) => {
  data.version = argument;
  data.packages[""].version = argument;
});
setJson("src-tauri/tauri.conf.json", (data) => {
  data.version = argument;
});
setText("src-tauri/Cargo.toml", CARGO_TOML);
setText("src-tauri/Cargo.lock", CARGO_LOCK);
console.log(`Studio is now version ${argument} in:`);
for (const name of Object.keys(current())) console.log(`  ${name}`);
