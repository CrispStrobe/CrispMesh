**CrispMesh {VERSION}, a prerelease. Read this before installing.**

Turntable photos to a closed, printable surface (STL), on your own computer, phone or
browser. No cloud, no upload, and since this release no external program is needed.

### What is new

- **Photos to STL in one command, natively.** `crispmesh-dense run --photos <folder> --calibration <lens.json> --output <run>` makes masks, recovers the cameras, computes depth, builds the closed surface and checks it against the photos. No Python, no AliceVision, no COLMAP. The 73-photo Bunny takes about 93 seconds on an Apple M1.
- **The app reconstructs from photos** on the desktop (macOS, Windows, Linux) and on the phone (iOS, Android builds), with the engine built in. On a phone, photos and the lens calibration come in through the system's picker, and the STL goes out through the share sheet.
- **The web page reconstructs in the browser**: https://crispstrobe.github.io/CrispMesh/ — choose the photos and a lens calibration, or an inputs folder; everything is computed on your device's graphics processor and nothing is uploaded. Where the page can share memory it uses up to four threads (the Bunny from photos in about 6 minutes in Chrome on an M1). Needs a browser with WebGPU (current Chrome or Edge) and up to 2 GiB of memory for 73 photos.
- **Cameras** come from interchangeable providers: `turntable` (our own solver, the default, on every platform), `markers` (a printed marker mat, poses in millimetres), `colmap` and `alicevision` (external programs, desktop only), and `import` (an existing AliceVision or COLMAP solution).
- **Masks**: `threshold` (dark object on a light backdrop, with the contact shadow taken out; the default) or SAM 2.1 (optional: through ONNX, CrispEmbed's ggml engine, or an external Python), or `import`.
- The demo is the native pipeline's own sphere: round, not cut. The replay shows the same final surface at every speed, and the 3D view says which surface it shows.

### How good it is

Four objects of the 3DLF-Scan set (73 photos each, Apple M1, the default command). Scanner F1 at 0.5 % / 1 % / 2 % of the scan's diagonal, against independent structured-light scans used for scoring only:

| Object | Whole run | F1, whole surface | F1, above the support |
| --- | --- | --- | --- |
| Bunny | 93 s | 0.904 / 0.939 / 0.956 | 0.964 / 0.995 / 1.000 |
| Armadillo | 83 s | 0.924 / 0.972 / 0.987 | 0.952 / 0.997 / 1.000 |
| Dragon | 78 s | 0.802 / 0.933 / 0.982 | 0.847 / 0.965 / 0.995 |
| Lucy | 50 s | 0.822 / 0.940 / 0.965 | 0.861 / 0.985 / 0.999 |

### Known limits

- **Unsigned builds.** Nothing here is signed with a developer certificate or notarised. macOS (ad-hoc signature only) will refuse to open the app until you allow it under System Settings > Privacy & Security, or remove the quarantine flag (`xattr -dr com.apple.quarantine "CrispMesh.app"`). Windows SmartScreen will warn about the installer. The Android file is a debug build signed with a throwaway debug key; the iOS file runs in the simulator only (the iOS and macOS apps are in internal TestFlight testing, not in the stores).
- **Turntable captures only**: one turn of ordered photos at one elevation, with a lens calibration. Twelve photos with unknown poses are not enough.
- Thin parts (horn tips, wings) come out short or are lost; surfaces no photo sees follow the silhouettes.
- The meshes have no physical scale, except with the marker mat.
- **Tried on Apple hardware only.** The desktop app on an Apple M1, the iOS app in the simulator, the browser engine in Chrome on the same Mac. Windows and Linux builds compile and pass their tests in CI, which has no GPU; nobody has run them on real hardware. Android has never been installed.
- **License: AGPL-3.0-only** (see `NOTICE` for the permission that covers the project's own store builds). Third-party licenses are in `THIRD-PARTY-NOTICES.txt` and on the app's Licenses screen.

| File | What it is |
| --- | --- |
| `Crisp-3D-Studio_*-macos-arm64.dmg`, `.app.tar.gz` | Desktop app, Apple Silicon |
| `Crisp-3D-Studio_*-windows-x64-setup.exe` | Desktop app, Windows installer (per user) |
| `Crisp-3D-Studio_*-linux-x64.AppImage`, `.deb` | Desktop app, Linux |
| `Crisp-3D-Studio_web-*.zip` | The web app as static files, with the demo and both browser engines; serve it over http(s) from any path |
| `crispmesh-dense-pipeline-*.tar.gz` | The Python reference pipeline (source); needed only for scoring against a reference scan and for comparisons |
| `Crisp-3D-Studio_*-android-arm64-debug.apk` | Android debug build (if present) |
| `Crisp-3D-Studio_*-ios-simulator-arm64.app.zip` | iOS simulator build (if present) |
| `THIRD-PARTY-NOTICES.txt` | The license, the store permission, and the licenses of everything the apps contain |
| `SHA256SUMS.txt` | Checksums of all of the above |
