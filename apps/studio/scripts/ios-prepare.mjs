// Finishes the Xcode project that `tauri ios init` generates (src-tauri/gen/apple, not in
// git), for everything Tauri's configuration has no setting for:
//
//   - the privacy manifest (src-tauri/PrivacyInfo.xcprivacy) becomes a resource of the app;
//   - with --team <id>: the development team and automatic signing, which the generated
//     project does not state and `tauri ios build` cannot be told on its command line.
//
//   npx tauri ios init --ci && node scripts/ios-prepare.mjs [--team TEAMID]
//
// Then it regenerates the project with XcodeGen, so the result is the same however often
// this runs. Needs `xcodegen` on PATH (it is what `tauri ios init` itself uses).

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const apple = join(root, "src-tauri/gen/apple");
if (!existsSync(join(apple, "project.yml"))) {
  console.error("src-tauri/gen/apple/project.yml is missing: run `npx tauri ios init --ci` first.");
  process.exit(1);
}
const target = readdirSync(apple).find((name) => name.endsWith("_iOS"));
if (target === undefined) {
  console.error("No *_iOS folder in src-tauri/gen/apple.");
  process.exit(1);
}

// Every file of the target's folder is a source of the target; XcodeGen makes this one a resource.
copyFileSync(join(root, "src-tauri/PrivacyInfo.xcprivacy"), join(apple, target, "PrivacyInfo.xcprivacy"));

// Localized OS permission prompts belong in the bundle, alongside the privacy manifest.
for (const language of ["en", "de"]) {
  const folder = join(apple, target, `${language}.lproj`);
  mkdirSync(folder, { recursive: true });
  copyFileSync(join(root, "src-tauri/localizations", `${language}.lproj`, "InfoPlist.strings"), join(folder, "InfoPlist.strings"));
}

const at = process.argv.indexOf("--team");
const team = at > 0 ? process.argv[at + 1] : undefined;
if (team !== undefined) {
  if (!/^[A-Z0-9]{10}$/.test(team)) {
    console.error("--team needs a ten-character team identifier.");
    process.exit(1);
  }
  const file = join(apple, "project.yml");
  let text = readFileSync(file, "utf8");
  const anchor = "      PRODUCT_BUNDLE_IDENTIFIER:";
  if (!text.includes(anchor)) {
    console.error("project.yml has no PRODUCT_BUNDLE_IDENTIFIER line to put the team next to.");
    process.exit(1);
  }
  text = text.replace(/^      (DEVELOPMENT_TEAM|CODE_SIGN_STYLE):.*\n/gm, "");
  text = text.replace(anchor, `      DEVELOPMENT_TEAM: ${team}\n      CODE_SIGN_STYLE: Automatic\n${anchor}`);
  writeFileSync(file, text);
}

execFileSync("xcodegen", ["generate", "--spec", "project.yml"], { cwd: apple, stdio: "inherit" });
const project = readFileSync(join(apple, readdirSync(apple).find((name) => name.endsWith(".xcodeproj")), "project.pbxproj"), "utf8");
if (!/PrivacyInfo\.xcprivacy in Resources/.test(project)) {
  console.error("The privacy manifest did not become a resource of the app.");
  process.exit(1);
}
if (!/InfoPlist\.strings in Resources/.test(project)) {
  console.error("Localized permission prompts did not become a resource of the app.");
  process.exit(1);
}
console.log(`${target}: privacy manifest is a resource${team !== undefined ? `, team ${team}, automatic signing` : ""}.`);
