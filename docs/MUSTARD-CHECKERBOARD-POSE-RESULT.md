# Mustard checkerboard-assisted pose diagnostic: one TRAIN-only run

Status, 2026-09-27: **39/48 board detections; descriptive camera poses only.** The [frozen plan](MUSTARD-CHECKERBOARD-POSE-PLAN.md) and four synthetic tests preceded real evaluation. The board rotates with the bottle, so this branch uses capture assistance and does not generalize to board-free photos. No Berkeley supplied pose, scanner mesh, sensor depth, held-out photo, or existing SfM pose entered detection, PnP, branch selection, or the report.

The first approved invocation stopped **before decoding a photo**: its worker was launched by script path and could not import the `scripts` package. It created no output. Root reviewed the launch-only change to module invocation; no thresholds, branch costs, or inputs changed. The reviewed runner SHA-256 for the successful retry is `1b61a7bff763bdd439686f0578a2f97bf7b5a2a21ef94696c322b4762a00fbbf` (the pre-fix runner was `58437fe7c89cd661c928dc8ccfc3229e5802952b2a6d89757aa49f4e77ae657d`).

The successful bounded run wrote only [`mustard-checkerboard-pose-001.json`](</Volumes/backups/code/crispmesh-data/mustard-checkerboard-pose-001.json>) (53,214 bytes, SHA-256 `e4b686795cc6f3bfba57ac77e9225113714d9d306b5919990191dffbd049a856`). The report binds the exact 48 original TRAIN JPEG hashes, sealed input-list SHA-256 `bc8d06024627219736959c7b4a34173cdd918bb5fba666528f08ca4a7a1957ae`, explicit capture-profile SHA-256 `bb00d17e1940f121dc12f7ddb31d4adf80b7e9a935f18e5535cfc1a2cde1998d`, runner hash, assumed intrinsics, all per-view choices, and unobserved views. Hashes were checked before and after the run. Both disks remained above 10 GiB free; the output was below the 2 MiB cap and the evaluation completed within the 180-second cap.

| Read-only result | Value |
| --- | ---: |
| Full 9×8 inner-corner detections / TRAIN photos | 39 / 48 |
| Missing board detections | `NP3_000`, `300`, `306`, `312`, `318`, `330`, `336`, `342`, `348` |
| Inner-corner hull / image area, min / median / max | 0.0139 / 0.0220 / 0.0309 |
| Selected reprojection RMS, median / p95 / max | 0.833 / 1.438 / 1.513 px |
| Per-view corner-error p95, median / max | 1.625 / 2.852 px |
| Selected 180° label flips / second IPPE branches | 0 / 0 |
| Adjacent center step / median PCA radius, median / p95 / max (38 observed transitions) | 0.106 / 0.210 / 0.251 |
| Adjacent full-orientation change, median / p95 / max | 6.15° / 12.23° / 13.23° |
| Present opposite-slot center separation / radius, median (15 pairs) | 2.056 |
| Present opposite-slot full-orientation difference, median (15 pairs) | 175.88° |

These trajectory figures were computed **read-only from the sealed report's selected poses** using the existing PCA-plane trajectory diagnostic; they were not used to select labels or tune the runner. The observed `NP3_006` through `NP3_288` partial arc has 38/38 PCA phase steps in one direction and a descriptive 291° span. The optional full-turn orbit gate correctly reports `unavailable`: 39/60 declared capture slots (65%) are below its frozen 75% coverage threshold, and the late missing arc prevents a full-turn acceptance claim. No reprojection quality flag exceeded the frozen 3 px RMS or 5 px corner-p95 thresholds.

Independent visual inspection found the `NP3_336` checkerboard partly occluded by the bottle. The full-grid detector reports `full_board_not_detected` there, as it does throughout most of the late arc. This frozen run does not attempt partial-grid recovery or alter detection settings to fill those gaps.

The fitted `camera_from_board` matrices obey `x_cam = R x_board + t` in **arbitrary checker-square units** under fixed assumed `fx=fy=1536`, `cx=640`, `cy=512`, and zero distortion. The first detector labeling only fixes an arbitrary gauge: reversing every board label by 180° gives the same reprojection error. Small residuals and a smooth partial board-relative trajectory do not establish calibrated intrinsics, an absolute board orientation, a board-to-bottle transform, object-only tracks, physical pose truth, or mesh accuracy. No reference-camera comparison or dense reconstruction was run; those require separate review.
