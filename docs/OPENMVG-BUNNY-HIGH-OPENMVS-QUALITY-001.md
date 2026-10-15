# OpenMVG HIGH bunny to OpenMVS: quality evaluation status

The continuation at
`/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-001`
stopped during `DensifyPointCloud`. No `dense.ply`, `mesh.ply`, or native
refined mesh exists, so **there is no candidate surface to inspect or score**.
No OpenMVG-to-OpenMVS shape claim or ranking is available from this run.

The frozen result receipt (SHA-256
`4f70d6ef7859ef061a928a0101fd3be3a4da8eff7460629045a39ec4a4098acb`)
reports `failed`, with `convert` complete in 99.24 seconds and `densify`
stopped by SIGTERM (`returncode: -15`) after 569.24 recorded seconds. The
depth log reached 46/73 estimated maps; 46 `.dmap` files were present.
The converter produced `scene.mvs` from the sealed 73-view sparse model.
The receipt reports `source_unchanged: true`, 583,256,351 output bytes, and
13,366,390,784 free bytes on the external volume after stopping, above the
11 GiB reserve. The run supervisor intentionally sent SIGTERM to the exact
OpenMVS child after host elapsed time exceeded 53 minutes. The Mac had slept;
the Python monotonic watchdog counted only 569 seconds and failed to enforce
the promised 20-minute real wall limit. This is a supervised resource stop,
not evidence of a native OpenMVS crash. The partial output is preserved.

A separate, fresh-root rough-mesh cache continuation ran at
`/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-cache-resume-002`.
The [preflight and runner](../scripts/classical_backend/openmvg_bunny_high_cache_rough.py)
seal the failed receipt, scene, all 73 converted images, 46 completed base
depth maps, binaries, and original sparse/camera-review lineage. They copy
each input and check hashes; the failed source is never a working directory.
The native command keeps the first run's resolution level 3, minimum 640,
maximum 1280, and two geometric iterations, so cached maps remain comparable.
OpenMVS's [v2.4.0 usage guide](https://github.com/cdcseacave/openMVS/blob/v2.4.0/docs/wiki/Usage.md)
states that existing `depthXXXX.dmap` files skip estimation and only missing
maps are computed. The runner aborts early unless `depth0046.dmap` appears
within 90 real seconds while all 46 copied maps remain unchanged. It wraps
native commands with `caffeinate -disu` and checks both epoch time and
`ps` elapsed time against 20 minutes. It retains the 1 GiB output cap,
11 GiB disk floor, 4 GiB process-tree RSS cap, and 16 MiB per-stage log cap.
It reused the 46 original maps and completed all 73 base maps, then exceeded
the 1 GiB logical output cap while generating geometric-consistency maps.
Its receipt (SHA-256
`b559ef8970065b15cbf3df4a7796fef2275fd3b6831ea42321ddc1540933fd2e`)
records `failed`, `source_unchanged: true`, 1,077,497,289 output bytes,
387.428 real-wall seconds, and 22 `.geo.dmap` files. It produced no rough
mesh. The supervisor stopped it on its configured output cap; this is not a
native crash. Both failed roots remain preserved.

A third, separately labeled cached-fusion arm is prepared at the fresh root
`/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-openmvs-cache-fusion-003`.
Its [runner](../scripts/classical_backend/openmvg_bunny_high_cache_fusion.py)
seals the 73 converted images, 73 completed base maps, source receipt, scene,
and binaries. It clones these inputs as distinct APFS copy-on-write inodes,
hash-checks every file, and excludes all partial `.geo.dmap` files. The native
densifier uses the same resolution settings but `--geometric-iters 0` to fuse
the cached base maps directly. It fails closed if the native log does not
reach fusion within 90 real seconds, reports depth-map re-estimation, or
rewrites a cached map. A verified local `cp -c` probe consumed only 16 KiB
of additional free space for a 10,086,207-byte map. Because `cp -c` may
silently fall back to a physical copy, each clone is also checked against
a 4 MiB physical free-space delta and the whole staging against 32 MiB.
The full run retains the 11 GiB floor, 20-minute epoch/`ps` wall watchdog,
4 GiB process-tree RSS and 16 MiB log caps. Its raised 1.25 GiB logical cap
allows native dense and mesh outputs, while incremental physical use is
limited to 400 MiB. Read-only preflight passed with 12,343,361,536 external
free bytes, 21,974,142,976 internal free bytes, and 855,555,526 logical
bytes to clone. This arm has **not been launched**. Only rough meshing is in
scope; refinement and texturing still require a separate hash-bound review.

## Frozen evaluation when a completed native mesh exists

Use the existing [bunny classical comparison protocol](BUNNY-CLASSICAL-COMPARISON.md)
without changing its fit, crop, thresholds, samples, or seeds:

1. Verify a completed continuation receipt and source/model/mesh hashes;
   preserve native `mesh.ply` and, if available, `refined.ply`. Losslessly
   normalize each candidate through `scripts/classical_backend/geometry.py`,
   recording both source and exported SHA-256 hashes. Confirm finite triangle
   geometry and count zero-area faces.
2. Inspect unaligned candidate shape and shared-bounds XY/XZ/YZ previews
   against the whole Revopoint scanner mesh and its already frozen scanner-only
   Y > -24 ROI. Inspect the bunny silhouette, ears, body, support disk,
   detached pieces, and extra undersurface before interpreting metrics.
3. Fit each whole candidate independently with the unchanged
   `scripts/object_dataset/align.py` 1,024-sample, seed-2026 proper Sim(3)
   procedure to (a) the whole scanner and (b) the frozen ROI. Keep all
   candidate faces in both fits. Use `scripts/object_dataset/preview.py` for
   the paired 10,000-sample, seed-2030 orthographic previews.
4. If visual inspection supports reporting a descriptive fit, run
   `scripts/object_dataset/surface_metrics.py` with 4,096 area-weighted query
   samples per mesh, seed 2027, and exact nearest-triangle distances. Whole
   scanner thresholds are 1.0429398885, 2.085879777, and 4.171759554
   reference units. ROI panel A thresholds are 0.6140926143, 1.2281852285,
   and 2.4563704570; panel B repeats the whole-scanner absolute thresholds.
   Report both precision and recall with F, topology, and fitted scale.

The full scanner has a support base and uncertain cross-sensor registration;
the ROI is a post-hoc scanner-only crop. Proper Sim(3) fits scale and pose to
the same scan later used for scoring. These are development diagnostics, not
independent metric accuracy or a reliable intrinsic quality ranking across
MVE, COLMAP/OpenMVS, and OpenMVG/OpenMVS. Keep existing MVE and classical
scores as published in the comparison document; do not rescore or retune them
to make a new order. The dense continuation and any later evaluation have
separate timings and resource records.
