# Cracker-box visual-hull control — sealed result

The one approved 60-view, 160³ image-only hull run completed without boundary
truncation: 55,815 occupied cells, 29,860 triangles, and a compact box-like
[neutral preview](/Volumes/backups/code/crispmesh-data/cracker-visual-hull-001/neutral-preview.png).
The separate one approved scanner-only score then reproduced the frozen rough,
refined, and reference-self baselines exactly before evaluating the hull. No
run was retried and no threshold, camera, mask, grid, or candidate was changed.

## Fixed shared-gauge surface score

Precision (P), recall (R), and F are percentages at 0.5%, 1%, and 2% of the
independent scanner bounding-box diagonal. Every row uses the same unchanged
score-005 camera-transported 006-to-reference Sim(3), 2,048 area-weighted
samples per direction, reference seed 2027 and output seed 2028.

| Candidate | 0.5% P / R / F | 1% P / R / F | 2% P / R / F |
|---|---:|---:|---:|
| Old rough OpenMVS | 16.16 / 18.55 / 17.28 | 29.49 / 34.47 / 31.79 | 49.80 / 57.81 / 53.51 |
| Old refined OpenMVS | 13.92 / 18.75 / 15.98 | 28.22 / 34.18 / 30.92 | 49.22 / 56.74 / 52.71 |
| Frozen visual hull | 8.94 / 7.32 / 8.05 | 18.21 / 14.79 / 16.33 | 35.30 / 27.49 / 30.91 |

At the primary 1% threshold, the hull loses 11.28 percentage points of
precision and 19.68 of recall versus the old rough mesh. It fails the
predeclared gate requiring **both** to improve by at least five points. The
scanner-fitted hull alignment, reported only as a secondary sensitivity
check, has 1% P/R/F of 17.43/23.39/19.98%; it does not rescue the result.
The fixed-gauge hull directional p95 distances are 0.02695 (output-to-scanner)
and 0.04416 (scanner-to-output) in reference-coordinate units, versus a
0.27863 reference bounding-box diagonal.

The neutral preview was misleading as a quality signal: it looked compact
and roughly box-like but had poor independent surface agreement. The hull has
**92 nonmanifold edges** (zero boundary edges), so no watertight mesh volume
ratio is reported. Exact X-ray parity recovers all 55,815 sealed occupied
voxels; 4,460 centers (7.99%) fall outside the scanner AABB under the fixed
gauge. That is an AABB-center diagnostic, not an IoU, a rigorous volume bound,
or a physical-mass claim. The sealed score JSON calls it a "lower-bound
extra-volume proxy"; that wording is heuristic because boundary cells can
straddle the AABB. At 1%, the hull's extra *surface* fraction is 81.79% (`1-P`) and its
unsupported scanner *surface* fraction is 85.21% (`1-R`).

This falsifies the hoped-for quality improvement from these coarse
photo-derived masks **together with** the image-derived cameras and frozen
carving rule. It does not uniquely assign error to segmentation, camera pose,
cross-scanner gauge, or their interaction. The shared gauge contains an older
reference fit and therefore does not establish physical pose or metric scale.

## Provenance and bounds

The [hull report](/Volumes/backups/code/crispmesh-data/cracker-visual-hull-001/hull-report.json)
SHA-256 is `b0e88ce667f34e6e569aace1b21987f0fc3291813666887816e3c1556b05f2af`;
the immutable hull mesh is `050f230c49f17d5e54fb5b15d8a2ddf4878e250d9662a9ad94fdd9eabdbc2de4`.
The [score report](/Volumes/backups/code/crispmesh-data/cracker-visual-hull-score-001/score.json)
SHA-256 is `98db747b2dbd5b1ad4af7bf82fa520ea5d9ac3e5e6e9c4b7eca3eb08dbf06b1a`;
its [supervisor receipt](/Volumes/backups/code/crispmesh-data/cracker-visual-hull-score-001/result.json)
is `7e13de4592610e8787130105b2435aaf1b7ac96589e319ab54af4ab68d133438`.
The exact scanner hash remained
`6e0187aa961aef4fa21dfc753023a4be7e82924e6398dc0875a2b0d609363ed4`;
both previous score/diagnostic reports, rough/refined native meshes, and
candidate result/report/mesh all rehashed unchanged after scoring. The score
stage took 17.295 seconds, sampled peak child RSS 154,238,976 bytes, and
wrote 2,056,823 bytes; both disks retained more than 10 GiB. Reference input
was scoring-only, never fed into carving or candidate modification.

## Next controlled quality experiment — proposal only

If continuing this board-free object, make **one mask-only intervention**:
independently annotate object-only silhouettes for the same 60 TRAIN RGB
views, seal and audit their per-name image/mask hashes, then hold the existing
60 cameras, sparse-derived cube, 160³ grid, all-visible rule, and scoring
protocol fixed. This would test whether coarse foreground masks account for
the failure while avoiding camera/grid retuning. The new masks must be
prepared without scanner projections, and the candidate sealed before any
scanner score. This is a proposal, not authorization to annotate or run it.
