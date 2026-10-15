# P1 cracker-box reference-first experiment

Frozen before execution, 2026-09-27. This continues
[REFERENCE-FIRST-PLAN.md](REFERENCE-FIRST-PLAN.md); no new acquisition is needed.

## Inputs and arms

All arms use the same 60 original Berkeley NP3 JPEGs (1280×1024), angles
000 through 354 at six-degree intervals. The source manifest SHA-256 is
`d3d1945828a34413e2205ff7617850a36a99b4db462585087a8cecade5de9c64`.
Reconstruction receives no supplied cameras, calibration, depth or reference mesh.

1. **Stock COLMAP raw:** PyCOLMAP 3.11.1, shared SINGLE/SIMPLE_RADIAL camera
   (one physical NP3 camera), self-calibration, exhaustive matching and automatic
   initialization. Native numerical defaults, CPU two threads, seed 20260927.
2. **Stock COLMAP feature-masked:** identical options, with the existing 60
   photo-derived foreground feature masks. Mask manifest SHA-256:
   `0b78469039d11516d0ac96b642553099267cd97c9a6439a1ec4b2fcb133615bd`.
   These are coarse feature support, not ground-truth silhouettes; only four
   sample angles had human QA. Masks apply to feature extraction, not automatically
   to OpenMVS depth reconstruction. Both arms are declared now, not tuned after results.
3. **Apple Object Capture preview:** original RGB folder, default RealityKit
   configuration, preview USDZ. No explicit masks or supplied cameras. Internal
   processing is opaque: this is a product-workflow comparison, not an isolated
   mask-controlled algorithm comparison. Existing probe binary SHA-256:
   `f6fe2bbb5677177eeba8b1b19b8a1c49c2e785305b9cfa1f781b67ae10bd277b`.

## Resources and outputs

Run native jobs serially on M1, without simultaneous test/build workloads.
Each arm has a ten-minute deadline, 512 MiB output cap and a 10 GiB free-space
floor on both volumes. COLMAP sampled child RSS cap is 4 GiB; Apple is 8 GiB.
Apple's direct-child measurement does not cover all system/XPC service memory.
Scratch and artifacts remain on the external SSD, under
`/Volumes/backups/code/crispmesh-data/` in fresh directories:

- `cracker-stock-raw-001`
- `cracker-stock-masked-001`
- `apple-crackerbox60-preview-001`

Preserve failed/partial results. No automatic retries, pair-graph edits, manual
initial pair, track repair, or geometry cleanup. Freeze executable source before
each run and record input, executable, option and output hashes.

## Interpretation and continuation

Report all sparse components, selecting by registered-image count, point count,
then component index, never by reference agreement. At least 54/60 views and
valid finite reciprocal tracks make a component structurally eligible only.
Repeated image IDs in a track are not automatically invalid (see Sceaux results).
Independent post-hoc rig-camera review must precede a dense continuation.
The rig convention/board-offset limitations remain; it is a camera diagnostic,
not certified physical ground truth. Dense continuation needs its own frozen
image/camera/mask handoff and budget; old camera-specific warped masks are not reused.

Validate an Apple USDZ structurally and inspect bare geometry before calling
its mesh useful. A nonempty file alone is not quality evidence. The separate
Google scanner mesh has no independently calibrated Berkeley-frame transform:
later reference-fitted shape scores must not be labeled dimensional accuracy.
No held-out appearance claim is possible when all 60 images enter reconstruction.

The immediate question is whether standard alignment recovers this object's
orbit, and whether an independent established product produces usable geometry.
Neither completion nor vertex/face count establishes KIRI or SOTA parity.
