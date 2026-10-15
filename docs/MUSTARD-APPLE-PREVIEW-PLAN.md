# Mustard Apple Object Capture preview: frozen oracle plan

Run one Apple-only evaluation of the exact sealed 48 TRAIN mustard RGB JPEGs
at `/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001/images` through
the existing `scripts.apple_object_capture.launch` and compiled RealityKit
probe. Request `.preview` with the default `PhotogrammetrySession`
configuration. No masks, held-out photos, depth, reference mesh, supplied
camera poses, or reconstructed poses enter the Apple input directory or the
scoring stage. Apple Object Capture is an evaluation-only product oracle,
not the cross-platform CrispMesh backend.

The stage report `mustard-sfm-train-001/stage-report.json` must retain SHA-256
`bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`.
Its 48 train names and per-photo hashes must equal the live read-only folder
inventory; the source has 48,683,099 JPEG bytes and no held-out names.
The compiled probe must retain SHA-256
`f6fe2bbb5677177eeba8b1b19b8a1c49c2e785305b9cfa1f781b67ae10bd277b`;
the Swift source and Python launcher preflight hashes are respectively
`cd9a8895ac4361ff41a799019c8bbd97aad504f0b00bbfe280ad658ecad3c813`
and `615fcb2a028b07a37f665c9080a10f6909b4bbae3de429bb80aed9f4deb101e5`.
The prior support result and cracker-box run used this probe binary, but
neither predicts mustard completion or quality.

Use the fresh external directory
`/Volumes/backups/code/crispmesh-data/apple-mustard48-preview-001`.
The exact launcher options are `--max-output-mib 1024` and
`--timeout-minutes 15`; the existing launcher also caps the stage log at
16 MiB and sampled direct-child RSS at 8 GiB. Its child temporary directory
is inside the run directory and included in the monitored 1 GiB output cap.
Before launch require the output volume to have at least 10 GiB plus the
1 GiB cap and 256 MiB preflight buffer free, and the separate internal
probe-binary volume to have at least 10 GiB plus 256 MiB free. Preserve a
failed or partial result; make no automatic retry or parameter change.

If the run finishes, record the launch report, elapsed time, resource
measurements, source/binary unchanged flags, USDZ byte count and SHA-256.
Then check USDZ structure with installed `usdchecker --arkit` and `usdtree`.
Only if the USDZ is at most 2 MiB and has the constrained one-Mesh triangle
schema supported by `review_mesh.py`, use that existing bounded tool for an
untextured PLY, structural vertex/face/finite-coordinate checks, and a
neutral three-view preview. A human may inspect object coverage, obvious
board/background leakage, holes, and extrusions; keep a nonempty or
structurally valid file separate from a clean visual-quality decision.
If structure differs or the model exceeds the review tool's input bound,
report the limitation rather than forcing a parser.

Do not claim metric scale, reference surface accuracy, held-out appearance,
physical camera correctness, or overall product quality from completion,
triangle count, or a preview. The 60-photo cracker-box Apple run uses a
different object and input set, so its speed and shape are not a controlled
mustard comparison. Prior mustard sparse/dense runs have different internal
algorithms and, in some arms, feature masks; this oracle can be compared at
the limited level of whether the same 48 original RGBs yield a structurally
inspectable candidate, not by an unshared quality score.
