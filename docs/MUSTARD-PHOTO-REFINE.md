# Mustard photo-only boundary candidate

The input is the existing 48-training-view stage at
`/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001`. Its report SHA-256
is `bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`;
the previously accepted SAM inventory and visual-review SHA-256 values are
`c341e3798a792c83abbacac8a7f882a761c878cdd9dba693c02c5877bbb75751`
and `c07a62441a4846f53619d28d16f931bfd3a0e85f99a26df83bc6b664cdd8a2da`.
The earlier warm-colour and row-envelope candidates remain rejected. The SAM
stage is a coarse support control, not a reference silhouette.

Before any geometry score, the new fixed photo-only rule is: OpenCV GrabCut
within original-pixel ROI `[470,290,770,660)`, with the prior mask eroded by a
3-pixel elliptical kernel as certain foreground and pixels beyond an 8-pixel
dilation as certain background, for three iterations. Only pixels inside the
prior can be retained. Every core pixel and every prior pixel with original
`y < 435` is retained irrespective of GrabCut's result; this protects the
thin cap/nozzle and its edge. This cap boundary was chosen from the original
48-view RGB/SAM visual review. No prior mask may lose more than 8% of its
pixels. The full 48 source photo and mask byte hashes are checked against the
accepted stage report; a mask outside the fixed ROI or outside 10,000–30,000
pixels fails. All outputs are new, external, and capped at 128 MiB with at least
10 GiB free on both disks. No scanner mesh, sensor depth, pose, held-out photo,
or reconstruction score enters the rule or producer.

An earlier **v1 unreviewed** trial used the same GrabCut rule without the
`y < 435` cap protection. It produced 48 masks at
`/Volumes/backups/code/crispmesh-data/mustard-photo-refine-001`; the review
sheet showed cap-edge trimming. Its report SHA-256 is
`088684da17e34185c40bc500c17c29331be7a8661a102446d8a49419615d01a0`.
It remains untouched and is not accepted. This cap-preserving v2 is the one
bounded correction to be generated and visually reviewed. The machine status
`generated_unreviewed` alone does not authorize feature matching or dense masking.
Visual review must examine all 48 RGB/trim tiles, especially cap and label
front views `000/006/012`, side labels `060–216`, the checkerboard contact at
`282`, and `300–348`. A further correction would require a new decision.

## Bounded v2 result and visual decision

Three focused tests passed. The one v2 M1 run completed all 48 training views
in 16.85 s at `/Volumes/backups/code/crispmesh-data/mustard-photo-refine-002`
(760 KiB, versus a 128 MiB cap). Its [report](</Volumes/backups/code/crispmesh-data/mustard-photo-refine-002/report.json>) SHA-256 is
`0c47218e91f11f2499d618589fa91f6e4879a95ef0c89fb1681af57f677fb125`;
the [48-view sheet](</Volumes/backups/code/crispmesh-data/mustard-photo-refine-002/trim_review.jpg>) SHA-256 is
`bc316c53ed94c24f00563003d162f4f6f8af712f3d821b1da6d1d7ef9dc9b2a8`.
Runner SHA-256 is `7c9b2f331d7a230f98253c3f1cf80fb4afe4d4a090396fb30480567d19ff367f`.
Both internal and external disks still had 22 GiB and 16 GiB free, above the
10 GiB floor. The original accepted masks and the v1 trial were unchanged.

Exactly 7,084 prior foreground pixels were removed across 48 views and zero
were added. Per-view removals range 21–263 pixels; maximum fractional removal
is 1.84% at `NP3_090`. For representative views, removals are `000` 158,
`006` 153, `012` 158, `060` 229, `180` 130, `216` 158, `282` 192,
`300` 127, `318` 170, `330` 153, `336` 31, `342` 21 and `348` 53.
Every source photo and prior mask was hash-checked. The candidate's upper cap
zone is pixel-identical to the prior by construction, and label interiors
remain retained. The 48-tile visual check shows labels across front, side and
back and no obvious added checkerboard; small magenta removal appears along
some side-label and outer bottle edges. A feature narrower than the protected
three-pixel interior or outside the cap zone **can still be lost**. The sheet
cannot establish exact contour accuracy or a geometry benefit. Root
independently viewed the full 48-tile sheet. At that scale there is no obvious
interior label or cap loss; zero additions also prevent **new** board support.
Side-label/body edge removals remain ambiguous. The exact [QA decision](../tests/datasets/mustard_photo_refine_v2_review.json)
approves v2 solely as a **bounded coarse-feature-support ablation input**
alongside the accepted SAM control. It is not the default support, nor is it
an accepted silhouette, a demonstrated improvement, or a ready dense mask.
The producer's `generated_unreviewed` status remains unchanged. No camera or
dense run consumed it, and no further mask variant is authorized in this trial.

Under this limited visual decision, the next camera experiment is one paired,
48-train-only PyCOLMAP run with identical original
JPEGs, SIFT/matcher/options, seed and fixed initial intrinsics, changing only
feature masks: accepted SAM control versus v2. Use a fresh external output for
each arm and assess registered views, verified pairs, distinct-view tracks,
reprojection residuals, and orbit alias/winding diagnostics before any dense
score. Preserve any failed arm. Only with a plausible common camera model,
run a separate native dense mask ablation on that **same frozen camera model**:
accepted SAM versus v2 masks, with unchanged OpenMVS images/depth/fusion settings
and the existing 10 GiB reserve. Compare native depth support, fused-cloud
coverage and whole-mesh precision *and* recall; do not attribute a camera
change to the dense mask. The 12 held-out views remain untouched until a
separately disclosed localization/render evaluation. The candidate is too
conservative to assume it will cure the prior mustard camera fold or zero
fused-depth failure.
