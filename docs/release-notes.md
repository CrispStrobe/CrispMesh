**CrispMesh {VERSION}, a prerelease. Read this before installing.**

Turntable photos to a closed, printable surface (STL), on your own computer, phone or
browser. No cloud, no upload, and since this release no external program is needed.

### What it does

- **Photos to STL in one command, natively.** `crispmesh-dense run --photos <folder> --calibration <lens.json> --output <run>` makes masks, recovers the cameras, computes depth, builds the closed surface and checks it against the photos. No Python, no AliceVision, no COLMAP. The 73-photo Bunny takes about 93 seconds on an Apple M1.
- **The app reconstructs from photos** on the desktop (macOS, Windows, Linux) and on the phone (iOS, Android builds), with the engine built in. On a phone, photos and the lens calibration come in through the system's picker, and the STL goes out through the share sheet.
- **The web page reconstructs in the browser**: https://crispstrobe.github.io/CrispMesh/ — choose the photos and a lens calibration, or an inputs folder; everything is computed on your device's graphics processor and nothing is uploaded. Where the page can share memory it uses up to four threads. Needs a browser with WebGPU (current Chrome or Edge) and up to 2 GiB of memory for 73 photos.
- **Cameras** come from interchangeable providers: `turntable` (our own solver, the default, on every platform), `markers` (a printed marker mat, poses in millimetres), `colmap` and `alicevision` (external programs, desktop only), and `import` (an existing AliceVision or COLMAP solution).
- **Masks**: `threshold` (dark object on a light backdrop, with the contact shadow taken out; the default) or SAM 2.1 (optional: through ONNX, CrispEmbed's ggml engine, or an external Python), or `import`.

### Several camera heights and photo-optimised surfaces

- **Captures from several heights.** `--turntable-rings N` joins N turns of the turntable taken at different camera heights into one set of cameras. With several heights the silhouettes come first and depth only corrects them (`silhouette_first`, automatic), followed by a refinement of the mesh against the photos (`photo_refine`). Single-turn captures are reconstructed exactly as before.
- **Two photo-optimised surfaces, experimental, natively on the GPU** (no Python, no CUDA): a neural signed-distance surface (after NeuS-facto) and 2D Gaussian splats (after gsplat's 2DGS). Each trains from the run's photos, masks and mesh and writes its own mesh beside the run's: `--surface fast|standard|best`, `--splats fast|standard|best`, single settings with `--surface-set` / `--splats-set`; also in the app's "New run" form, on the desktop and in a browser with WebGPU. On the YCB power drill (300 photos, five heights) both match reference runs of the original methods on a Tesla T4 (F1 at 0.5 % of the diagonal: 0.93 neural surface, 0.88 splats, against 0.89 and 0.88); `fast` takes about 20–40 minutes on an M1, `standard` one to one and a half hours.
- The photo resolution these methods use follows the object's size in the photos, not the camera's, so defaults need no adjusting per capture.

### Taking the photos in the app, and handing them to a computer

- **Take photos** with the device's camera (phone, tablet, Mac, or a browser): with *Auto* a photo is taken each
  time the object on the turntable has turned and stands still again; 36 photos for one turn are enough.
- **No lens file needed**: `--calibration auto` (and *Automatic* in the app) finds the focal length from the closed
  turn itself; on five of the six 3DLF objects within 3.3 % of the factory calibration and with practically the
  same mesh.
- **Photos on the phone, computation on a computer**: the desktop app can let other devices use its engine on the
  local network (a QR code pairs the phone); the phone sends its photos there, the run is computed there and
  followed on the phone. Or send the photo folder by AirDrop and start the run on the computer.

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
- **Turntable captures only**: one or more turns of ordered photos (one camera height per turn), 36 photos per turn; about 20 work with the neural surface on objects with enough texture (below 25 the cameras' landmark gate is lower, the ring checks still refuse wrong cameras); 15 are refused. Without a lens file the focal length is found from the turn (distortion is left out; on one object of six it came out 28 % too long).
- **Taking photos and the hand-off to a computer** are new and have been checked in a browser with a video as the camera, not yet on a phone; the connection between devices is plain HTTP with a token, for a network you trust.
- The neural surface and the splats are experimental and slow (tens of minutes to hours per object); printed labels can turn into false relief.
- Thin parts (horn tips, wings) come out short or are lost; surfaces no photo sees follow the silhouettes.
- The meshes have no physical scale, except with the marker mat.
- **Tried on Apple hardware only.** The desktop app on an Apple M1, the iOS app in the simulator, the browser engine in Chrome on the same Mac. Windows and Linux builds compile and pass their tests in CI, which has no GPU; nobody has run them on real hardware. Android has never been installed.
- **License: AGPL-3.0-only** (see `NOTICE` for the permission that covers the project's own store builds). Third-party licenses are in `THIRD-PARTY-NOTICES.txt` and on the app's Licenses screen.

| File | What it is |
| --- | --- |
| `CrispMesh_*-macos-arm64.dmg`, `.app.tar.gz` | Desktop app, Apple Silicon |
| `CrispMesh_*-windows-x64-setup.exe` | Desktop app, Windows installer (per user) |
| `CrispMesh_*-linux-x64.AppImage`, `.deb` | Desktop app, Linux |
| `CrispMesh_web-*.zip` | The web app as static files, with the demo and both browser engines; serve it over http(s) from any path |
| `crispmesh-dense-pipeline-*.tar.gz` | The Python reference pipeline (source); needed only for scoring against a reference scan and for comparisons |
| `CrispMesh_*-android-arm64-debug.apk` | Android debug build (if present) |
| `CrispMesh_*-ios-simulator-arm64.app.zip` | iOS simulator build (if present) |
| `THIRD-PARTY-NOTICES.txt` | The license, the store permission, and the licenses of everything the apps contain |
| `SHA256SUMS.txt` | Checksums of all of the above |
