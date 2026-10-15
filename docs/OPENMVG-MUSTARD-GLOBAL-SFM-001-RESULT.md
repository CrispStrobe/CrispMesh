# Mustard GLOBAL SfM control -001: sealed result audit

Read-only audit of the completed evaluation-only GLOBAL trial at
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-global-sfm-001`.
The one-shot process exited 0 in 7.011 s (sampled peak RSS 23,408 KiB), but
the supervisor correctly marked `failed_registration_or_sparse_gate`:
**48 views, 46 poses, one intrinsic, 69 tracks, 1,375 HTML residuals**. The
same sealed 48 TRAIN JPEGs and 105 staged photo-derived match files were used;
no earlier stage or ground-truth input was supplied to GLOBAL.

The receipt SHA-256 is
`612429ab5448fdf8bd71bcf6c03e6bafdedf2d38196dce8f51f27b277e8e19dc`.
Independently rehashed and matched its complete output inventory, all 105
staged match files, source -001 receipt/full inventory/JPEGs, the SfM binary
and build receipts, and the three reviewed GLOBAL source files. Key artifacts:

| Artifact | SHA-256 |
| --- | --- |
| `sparse/sfm_data.bin` | `eb5d96ce4379ee32bdef3f946f833fd2c7a85b0a5c818faa4ce799766f74fc00` |
| `logs/01-sfm-global.log` | `5bed68eb709c8e534262b0f0a5f0689512bcf288db18eb15c8ee4d2a629b1f3d` |
| `sparse/SfMReconstruction_Report.html` | `e73278112857ea2810db0fb8652222455d0c3e2d07fd8004ce729cea5f74d77c` |

The log shows the largest initial bi-edge-connected component covered 48
views. It computed 311 relative rotations, then rotation-triplet filtering
removed 19 edges and retained a 46-view component; translation averaging
subsequently solved 46 camera translations from 264 relative translation
estimates over 2,059 triplets. It built 392 candidate tracks, rejected 68
at triangulation, had 324 before outlier removal, 77 after pixel/angle
filters, and 69 after final refinement/cleaning. No fatal/error line appears:
the failure here is incomplete registration and a small final sparse model,
not a CLI or process crash.

The HTML report has 48 named rows. `NP3_252` (view 34) and `NP3_276` (view 37)
have no residual/observation columns, and those IDs are absent from the log's
track-image list. They are **missing-pose candidates, not confirmed pose IDs**:
pinned `sfm_report.cpp` also emits a blank row for a posed view with no
residuals. Named registration would require a separately reviewed sealed
JSON export. The 1,375 HTML residuals are not directly comparable to the
log's 2,750 coordinate residuals without accounting for two coordinates per
observation.

Against the earlier incremental `ADJUST_ALL` result (46/48 poses, 222 tracks,
1,497 HTML residuals), GLOBAL did not improve the registration count and
retained fewer tracks. Counts alone do not establish pose, orbit, mesh, or
physical accuracy. This result does not use or revise posthoc Berkeley scores
and grants no GPL/AGPL shipping clearance.
