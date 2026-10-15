# Bunny OpenMVG HIGH photo control

Run the 73 contrast-preprocessed PNGs at `build-opencv/bunny-gamma05-clahe2/`
through the existing evaluated OpenMVG v2.1 native binaries. This is the
same sealed input series recorded by
`build-opencv/classical-bunny-contrast-001/inputs.json`. Preflight verifies
every PNG's SHA-256, size, dimensions, and inventory against the preprocess
manifest and comparison manifest. The SfM worker receives only copied PNGs.

The listing uses a 2098.8 pixel initial focal estimate, equal to 1.2 times
the 1749 pixel image width. This is a photo-only heuristic. It does not read
the dataset's calibration, camera poses, depth, point cloud, or scanner mesh.
Features use `SIFT_ANATOMY` and `HIGH`; the remaining stages use putative
matching, essential-matrix geometric filtering, and incremental SfM.

The one-shot output is
`/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-photo-sfm-001`.
Default CLI invocation only preflights. `--run-bunny` creates fresh output.
The runner enforces a 1 GiB output cap, 4 GiB process-tree RSS cap, 16 MiB
per-stage log cap, two worker threads, and finite 120/1800/1800/1200/1800
second stage caps within a 7200 second total worker cap. Both external and
internal volumes retain at least 11 GiB free; preflight reserves the output
cap on external storage. The receipt seals staged photo hashes, command and
binary hashes, stage artifacts, logs, and the output inventory.

The camera-quality gate is independent of registration count. Even with
73/73 poses, the runner reports `completed_pending_geometry_review`.
Before any OpenMVS dense continuation, independently inspect the camera
trajectory and intrinsics, reprojection residuals, sparse point structure,
and object-centered support; bind a separate camera-quality receipt to this
run's `receipt.json` and `sparse/sfm_data.bin` SHA-256 hashes. A pose count
alone cannot pass the gate. OpenMVG is evaluation-only; this plan makes no
shipping or license clearance claim.

The `frame_####.png` numbering follows lexicographic source filename order,
not numeric capture order. The receipt preserves the sealed
`source_frame_by_photo` mapping. A trajectory or temporal orbit audit must
order by the numeric `bunny_N_rgb.png` source frame number.
