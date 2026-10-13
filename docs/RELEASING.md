# Releasing Crisp3DS

What a release consists of, how to cut one, what to check before publishing
it, and how builds reach TestFlight.

## What a release contains

`.github/workflows/release.yml` builds, for one version:

| File | Job | What it is |
| --- | --- | --- |
| `Crisp-3D-Studio_web-<v>.zip` | web | Studio as static files (relative URLs; demo recording, third-party notices and the engine for browsers, `engine/`, inside) |
| `Crisp-3D-Studio_<v>-macos-arm64.dmg`, `.app.tar.gz` | desktop | Studio desktop app with the engine built in, Apple Silicon, ad-hoc signed |
| `Crisp-3D-Studio_<v>-windows-x64-setup.exe` | desktop | NSIS installer, per user, unsigned |
| `Crisp-3D-Studio_<v>-linux-x64.AppImage`, `.deb` | desktop | Linux x64 |
| `Crisp-3D-Studio_<v>-android-arm64-debug.apk` | android (non-blocking) | debug build, throwaway debug key |
| `Crisp-3D-Studio_<v>-ios-simulator-arm64.app.zip` | ios (non-blocking) | simulator build, not installable on a device |
| `crisp3ds-dense-pipeline-<v>.tar.gz` | python | `scripts/turntable_mesh`, the modules it imports from elsewhere under `scripts/`, the engine contract, the sphere fixture, `LICENSE` |
| `SHA256SUMS.txt` | publish | checksums of all of the above |

The pipeline archive is tested before it is uploaded: it is unpacked outside
the checkout, its requirements are installed, and the unit tests and the
synthetic end-to-end run are executed from the unpacked copy. That is what
shows it is self-contained.

Every release states, in its notes:

- **Unsigned builds.** No developer certificate, no notarisation.
- **The desktop app reconstructs by itself.** The engine (`crates/dense`) is
  built in; from cameras and masks it needs nothing else, and no Python.
- **From plain photos it needs AliceVision or COLMAP on the computer**
  (set under Folders > Tools). Neither is bundled; SAM masks are optional.
- **The web build reconstructs in a browser with WebGPU** ("This browser"),
  from an inputs folder on the device, within 4 GiB of memory.
- **Tried on one Mac only**; Windows, Linux and the phone builds are untested
  beyond building.
- **No GPU-tested CUDA path.** CI runs on CPU; MPS is used on the development
  machine; `--device cuda` has not been run on a GPU by the project.
- **AGPL-3.0-only.**

## Cutting a release

1. Make sure `main` is green (`dense pipeline`, `studio`, `desktop app`).
2. Rehearse: **Actions > release > Run workflow** with `dry_run` on (the
   default), or `gh workflow run release.yml -f dry_run=true`. The version is
   then `0.0.0-dev+<short sha>`. Download `release-all` and look at it (next
   section). Nothing is released by a dry run.
3. Tag and push:

   ```sh
   git tag -a v0.2.0 -m "Crisp3DS 0.2.0"
   git push origin v0.2.0
   ```

   The tag must be `v` followed by a version like `1.2.3` or `1.2.3-rc.1`.
   The workflow writes that version into `apps/studio/package.json`,
   `tauri.conf.json` and `Cargo.toml` for the build
   (`apps/studio/scripts/set-version.mjs`); it does not commit it. To keep the
   repository's own numbers in step, run
   `node apps/studio/scripts/set-version.mjs 0.2.0` and commit before tagging;
   `--check` tells whether the files agree.
4. When the workflow is done there is a **draft prerelease** named
   `Crisp3DS <version>` with all files attached. It is invisible to the public
   until someone publishes it.

Phone builds get a plain numeric version (`1.2.3`; a dry run uses `0.0.1`),
because Apple and Android accept nothing else.

## Before un-drafting

- `SHA256SUMS.txt` lists every attached file, and each file's checksum
  matches after download (`shasum -a 256 -c SHA256SUMS.txt`).
- The desktop app starts on at least one machine per OS, finds or is told the
  checkout and the interpreters, and completes the synthetic sphere run
  (`apps/studio/README.md`, "Running it from a fresh clone"). Only macOS has
  been tried by the project so far.
- On macOS the app opens after the quarantine step described in the notes.
- The web zip works from a sub-path: unpack, serve the folder one level up,
  open `/<folder>/`, play the demo. In Chrome or Edge, "This browser" on the
  Connection screen is offered and completes a small inputs folder
  (`apps/studio/scripts/browser-engine-check.mjs` does this unattended).
- The desktop app's Folders > Tools: "Check" finds AliceVision or COLMAP
  where one is installed, and a photos run completes.
- The pipeline archive: unpack, install the requirements, run the quick start
  of `scripts/turntable_mesh/README.md`.
- The mobile files are present or deliberately absent (their jobs may fail
  without failing the release).
- The release notes still tell the truth (signing, Python, CUDA, license).
- Then: untick "prerelease" only if that is meant, and publish.

To withdraw a release: delete the draft (or the release) and the tag
(`git push origin :refs/tags/v0.2.0`).

## Signing (not set up)

The workflow contains steps for macOS and Windows signing that are skipped
unless these repository secrets exist. They have **never run with real
secrets**; expect to debug them.

| Secret | For |
| --- | --- |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD` | base64 `.p12` with a Developer ID Application certificate |
| `APPLE_SIGNING_IDENTITY` | the identity's name |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | notarisation of the direct-download `.dmg` |
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | base64 `.pfx` code-signing certificate |

Android release signing (a keystore) and iOS device builds are not wired up.

## Licenses

`apps/studio/docs/THIRD-PARTY-LICENSES.md` and `licenses.json` are generated
by `npm run licenses` in `apps/studio` and checked in the `studio` workflow:
a third-party GPL, AGPL, LGPL, SSPL, non-commercial or unstated license in
what ships fails the build, and so do stale files. Result today: **clean with
obligations**.

- Attribution: release builds carry `THIRD-PARTY-NOTICES.txt` (all license
  texts) and link to it from the app.
- MPL-2.0: `cssparser`, `selectors`, `dtoa-short`, `option-ext`. Used
  unmodified; their source is on crates.io. If one is ever patched, the patch
  must be published.

The audit covers the Studio apps, including the reconstruction engine linked
into them (`crates/dense`, the project's own code, and its dependencies such
as `wgpu`), and the same engine as WebAssembly in the web bundle
(`crates/dense/web`, listed as platform `browser`). They contain no Python,
PyTorch, AliceVision, COLMAP, SAM, OpenMVS or other third-party engine: the
app starts AliceVision or COLMAP as separate programs that the user
installed, and ships none of their code. The Python pipeline's dependencies are installed by the
user and are not redistributed by these releases, except as names in
`requirements-dense.txt`.


## App Store and TestFlight

CrispMesh is in App Store Connect as "CrispMesh",
bundle identifier `com.crispstrobe.crisp3ds`, for iOS and macOS. Builds go to
**internal TestFlight testing only**; nothing has been submitted for review
and there is no store listing beyond the name. How the account signs and
uploads is described in the owner's App Store notes, which are kept outside
this repository on purpose; nothing from them belongs in here.

### What a store build is

- **macOS**: the sandboxed variant
  (`tauri build -- --no-default-features --features native-engine` with
  `src-tauri/tauri.appstore.conf.json`): App Sandbox, user-selected files,
  outgoing connections only (`src-tauri/entitlements.appstore.plist`). No
  child processes: AliceVision, COLMAP, external SAM and the Python launcher
  are absent or listed as unavailable with a sentence saying why.
- **iOS**: the same engine; nothing that starts a program is offered. Runs and
  data live in the app's Documents, which the Files app shows (that is how
  photos get in and the STL gets out).
- Both run photos → masks (`threshold`) → cameras (`turntable`, `markers` or
  `import`) → dense stages → STL entirely inside the app.
- `PrivacyInfo.xcprivacy` (no tracking, no data collected, reasons for file
  times, disk space, boot time and user defaults), added to the generated
  Xcode project by `scripts/ios-prepare.mjs`;
  `ITSAppUsesNonExemptEncryption = false` (no crate in the store builds
  implements encryption or TLS; `licenses.mjs --store` lists them, and the list
  is empty); opaque iOS icons (`scripts/ios-icons-opaque.py`).
- The Licenses screen in the app shows `NOTICE` (AGPL-3.0-only with the
  section 7 permission for store builds published by the copyright holder),
  then every third-party license text and where the MPL-2.0 sources are.

### Licenses of a store build

`npm run licenses -- --store` audits exactly what the two store builds link.
It fails without the store permission in `NOTICE` and on any third-party GPL,
LGPL or AGPL code. Result: **clean with obligations** (third-party code is
permissive apart from four MPL-2.0 crates used unmodified; the project's own
AGPL code is covered by `NOTICE`).

### Version and build number

The marketing version is the app's version (`scripts/set-version.mjs`), the
same in the stores. The build number (`CFBundleVersion`) is the run number of
`testflight.yml`, so each upload has a higher one; a build number cannot be
reused and an upload cannot be undone.

### `testflight.yml`

Manual dispatch only; `dry_run` defaults to true.

- **iOS**: `tauri ios init`, `scripts/ios-prepare.mjs`, an unsigned archive
  (`tauri ios build --no-sign --archive-only`), export with App Store
  distribution signing and the iOS App Store profile, signature check,
  `altool --validate-app`.
- **macOS**: the sandboxed variant built and signed by the Tauri bundler from
  a keychain that lives for the whole job, with the Mac App Store profile
  embedded; `productbuild` wraps it in a signed installer package;
  `altool --validate-app`.
- With `dry_run=false`, each then uploads. Afterwards the build needs its
  encryption answer and an internal tester group in App Store Connect before
  testers see it.

Repository secrets: `ASC_API_KEY_P8_BASE64`, `ASC_KEY_ID`, `ASC_ISSUER_ID`,
`ASC_TEAM_ID`, `ASC_APP_ID`, `DIST_CERT_P12_BASE64`, `DIST_CERT_PASSWORD`,
`ASC_PROFILE_BASE64`, `MAC_PROFILE_BASE64`.
