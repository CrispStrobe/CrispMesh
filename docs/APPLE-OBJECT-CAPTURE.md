# Apple Photogrammetry RGB-only oracle

This is an optional **Apple-only comparison**, not the cross-platform CrispMesh
reconstruction backend. The original, small Swift probe in
`scripts/apple_object_capture/Probe.swift` uses RealityKit
`PhotogrammetrySession` to check runtime support and, only when explicitly
requested, submit an ordinary photo folder for a `.preview` USDZ model.
The adapter does not inject depth, LiDAR, reference mesh, external camera
poses, masks, or metric-scale constraints. It accepts JPEG/PNG files, not HEIC
(which can carry embedded depth); it does not inspect or strip all possible
image metadata. Output geometry is untested and cannot yet be counted
as a successful benchmark.

Apple's [photo-to-3D guide](https://developer.apple.com/documentation/realitykit/creating-3d-objects-from-photographs)
describes image-folder input, support checking, and a USDZ model-file request;
the [PhotogrammetrySession API](https://developer.apple.com/documentation/realitykit/photogrammetrysession)
defines `isSupported`, `process(requests:)`, and output events. Apple also
documents the `.preview` request as a lower-detail model than `.full` in its
[detail guide](https://developer.apple.com/documentation/realitykit/photogrammetrysession/request/detail).
Those are API descriptions, not a claim of quality or product shipping terms.
No ObjectScanner or msplat-ios code or assets were copied into this adapter.

## Live SDK and hardware result

On 2026-09-27, this workspace's MacBookAir10,1 (Apple M1, 16 GB), macOS 26.2,
Xcode 26.2 compiled the Swift source using the installed RealityKit SDK. A
direct `--check-support` run and the bounded Python launcher both returned
`{"schema":"apple_photogrammetry_support_v1","supported":true}`. The
compile and its local module cache consumed 84,780 KiB under
`.local-tools/tmp/apple-probe.9SE5u9`; the launcher support report is under
`.local-tools/tmp/apple-probe-support-003`. The final report SHA-256 is
`e9961667cd49730ddc56079f48dfa2014ea9446f3d73d9b983a894625009ff0e`;
the compiled binary SHA-256 is
`f6fe2bbb5677177eeba8b1b19b8a1c49c2e785305b9cfa1f781b67ae10bd277b`.
Combined new probe artifacts were
below the 100 MiB compile/probe cap. Free space remained above 10 GiB. **No
USDZ reconstruction was started.** The runtime support result is evidence of
API availability on this specific Mac, not an image-to-mesh success.

Reproduce the bounded compile/probe with a fresh project-local scratch folder
(the example's shell variable is intentionally task-specific):

```sh
python3 -c 'import shutil,sys;sys.exit(0 if shutil.disk_usage(".").free >= (10<<30)+(100<<20) else 1)'
apple_probe_dir=$(mktemp -d .local-tools/tmp/apple-probe.XXXXXX)
TMPDIR="$apple_probe_dir" xcrun swiftc -parse-as-library \
  -module-cache-path "$apple_probe_dir/modules" \
  scripts/apple_object_capture/Probe.swift \
  -o "$apple_probe_dir/photogrammetry-probe"
du -sk "$apple_probe_dir"  # stop if this exceeds the approved 100 MiB cap
python3 -m scripts.apple_object_capture.launch \
  --probe-bin "$apple_probe_dir/photogrammetry-probe" \
  --run-dir .local-tools/tmp/apple-probe-support-next \
  --check-support --timeout-minutes 1
```

The compile recipe requires an explicit preflight and output-size check; it
is not itself a live byte-limit monitor. The Swift binary also accepts
`--images FOLDER --output FRESH.usdz`, but the
Python launcher is the bounded route for a future approved reconstruction:

```sh
python3 -m scripts.apple_object_capture.launch \
  --probe-bin .local-tools/tmp/apple-probe.9SE5u9/photogrammetry-probe \
  --images /absolute/path/to/rgb-photos \
  --run-dir build-opencv/apple-object-capture-fresh
```

That mode requires 3–500 top-level JPEG/PNG files and rejects extra folder
entries, symlinks, existing output directories, non-macOS platforms, and
insufficient free space. It hashes input photos, the Swift binary, and
the output USDZ; records and size-limits the logs; monitors a 10 GiB disk floor,
output/RSS caps, and a
deadline; and terminates the child process group when a limit is reached. The
default support-check output cap is 100 MiB; actual reconstruction defaults
to 1 GiB and 15 minutes, both configurable only within upper bounds. The
USDZ artifact gate currently proves only that a nonempty file exists; no USDZ
mesh or texture validation or quality score is claimed. In particular, this
M1's current free space is insufficient for the default 1 GiB reconstruction
cap plus reserve and buffer, so even an approved run would fail preflight
unless available space changes.
