# Apple Object Capture bunny `.preview` trial

This is an evaluation-only Apple Object Capture arm on the **same 73
gamma/CLAHE processed PNGs** used by the bunny MVE/COLMAP comparison and the
new OpenMVG HIGH run. It has no scanner mesh, depth, supplied poses, or
calibration input. The source folder includes `prepare-manifest.json`, so the
Apple input is a fresh external copy containing only the 73 PNGs.

The input is now staged at
`/Volumes/backups/code/crispmesh-data/apple-bunny73-preview-001-input/images/`.
Its receipt is one directory above at `receipt.json`. The intended fresh
output path is
`/Volumes/backups/code/crispmesh-data/apple-bunny73-preview-001-output/`.
No Apple reconstruction has been run.

The staging preflight verifies every PNG SHA-256 against the frozen prepare
manifest and the prior comparison inventory. It also verifies the numeric
original-photo mapping, probe binary SHA-256
`f6fe2bbb5677177eeba8b1b19b8a1c49c2e785305b9cfa1f781b67ae10bd277b`,
and at least 11 GiB free on both the SSD and internal disk. The input photos
total 121,809,661 bytes. The Apple launcher output cap is 512 MiB; input plus
full output allowance is about 628 MiB, below the 650 MiB total ceiling.

Before any live attempt, run the read-only staged check:

```sh
python3 -m scripts.apple_object_capture.bunny_preview_trial --check-staged
```

After explicit review of the staged input and capacity, the existing bounded
launcher command from the receipt is:

```sh
python3 -m scripts.apple_object_capture.launch \
  --probe-bin .local-tools/tmp/apple-probe.9SE5u9/photogrammetry-probe \
  --images /Volumes/backups/code/crispmesh-data/apple-bunny73-preview-001-input/images \
  --run-dir /Volumes/backups/code/crispmesh-data/apple-bunny73-preview-001-output \
  --max-output-mib 512 --timeout-minutes 15
```

The compiled Swift probe requests RealityKit's `.preview` detail. The launcher
allows only a fresh output, checks exact image hashes before and after the
run, and bounds output bytes, time, logs, child RSS, and disk reserve. Its
active disk watchdog uses a 10 GiB floor, while this trial's staging and
immediate prelaunch checks require 11 GiB. If an 11 GiB *active* floor is
required, adjust that watchdog before launch.

The output USDZ, if produced, remains a research artifact until inspected.
The launcher's success condition is a nonempty USDZ; it does not establish
mesh shape or texture quality. Compare visual object shape, coverage, and
artifact size against the other image-only arms, with scanner-based metrics
kept as post hoc evidence and not as reconstruction inputs.
