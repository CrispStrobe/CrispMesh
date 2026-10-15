# Fresh 005 sensor-depth diagnostic: frozen pre-score plan

This plan was recorded before scoring either 005 mesh. The producer is the
complete `turntable-fresh-openmvs-005` run. Berkeley depth, masks, calibration,
and table poses are evaluation-only; no reconstruction parameter, mesh, or mask
is changed from their readings.

Use the exact 0°, 120°, 240° frames and the coarse and 16-pixel-eroded RGB
support fixed in `SENSOR-DEPTH-BENCHMARK.md`. Require its original three sorted
uint32 ray hashes and interior counts. Fit one proper Sim(3) from the exact
fresh 004 sparse model's 60 named image-derived camera centers to the sealed
Berkeley table-frame camera centers, recording center and orientation errors.
Require that the 005 copied sparse model and dense sparse poses retain this
source gauge. Do not transfer a transform from 008, 011, 014, 015, or the
Google-mesh alignment. Reject incomplete provenance, changed input hashes,
or a camera-fit residual over 5% of the Berkeley camera radius or 10° p95.

Score the native rough `mesh.ply` as the primary stage-matched arm. Score the
completed native `refined.ply` as a secondary within-005 result. Losslessly
export each to bounded triangle PLY. Use the original first positive,
double-sided 0.2–1.5 m BVH ray intersection and report per-view and pooled
hits, misses, hit-only residuals, and 5/10/20 mm precision among hits and all
supported rays. Compare rough 005 with rough 008, 014, and 015 only after
verifying the prior report has identical selected IDs, observed depths, and
interior membership. 011 may be discussed only if those conditions and a
rough-stage score exist. Refined 005 has no stage-matched prior comparator.

One batch has a 300-second deadline, three views, at most 2,048 rays/view,
the existing one-million-vertex/two-million-face cap, and a 20 MiB JSON cap.
Require at least 10 GiB free on both workspace and backup volumes before and
after. All persistent new diagnostic artifacts go under
`/Volumes/backups/code/crispmesh-data`. The output is a diagnostic on a
repeatedly inspected development object, not an independent metric-accuracy
or whole-object completeness claim.
