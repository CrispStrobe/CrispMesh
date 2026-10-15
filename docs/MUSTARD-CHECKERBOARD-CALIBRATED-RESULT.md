# Mustard capture-calibration-assisted board poses: one TRAIN-only run

Status, 2026-09-27: **39/48 full-board detections; diagnostic poses only.** The [frozen plan](MUSTARD-CHECKERBOARD-CALIBRATED-PLAN.md) and four synthetic tests preceded the one approved live run. This assisted-capture branch used only the acquired NP3 RGB K/distortion, original sealed TRAIN photos, and explicit capture order. It did not read Berkeley per-angle poses, scanner mesh, depth, held-out photos, prior camera scores, or the previous image-derived camera report during fitting. The earlier uncalibrated report remains byte-identical.

The fresh output [`mustard-checkerboard-calibrated-pose-001.json`](</Volumes/backups/code/crispmesh-data/mustard-checkerboard-calibrated-pose-001.json>) is 53,933 bytes, SHA-256 `25f37be625cf4edd00bdf29c479529d341cd76d3dc748d8b966e482c4668619d`; runner SHA-256 `6dad7bd210848480699c10348b00ec792ae5f017108a1f0ba9c74c83c218c705`. It records source-image hashes, calibrated K/d, selected camera-from-board poses in arbitrary square units, all relative label/planar choices, and missing frames. Postflight hashes matched the original TRAIN input manifest `bc8d06024627219736959c7b4a34173cdd918bb5fba666528f08ca4a7a1957ae`, capture-order profile `bb00d17e1940f121dc12f7ddb31d4adf80b7e9a935f18e5535cfc1a2cde1998d`, calibration H5 `b9ff8208e18cb16d3348ce07967f868bf64576a13dc36e3094497912f85817e0`, metadata receipt `3a2b156289b7f7f11d20ef5026ffe9f19893333d155f0acb6e326f027ceb0747`, and all 48 TRAIN JPEGs. The original uncalibrated report remained SHA-256 `e4b686795cc6f3bfba57ac77e9225113714d9d306b5919990191dffbd049a856`.

| Frozen image-only/capture-calibration diagnostic | Result |
| --- | ---: |
| Full 9×8 board detections | 39 / 48 |
| Missed | `NP3_000`, `300`, `306`, `312`, `318`, `330`, `336`, `342`, `348` |
| Distortion-aware reprojection RMS, median / p95 / max | 0.143 / 0.315 / 0.331 px |
| Per-view corner-error p95, median / p95 / max | 0.234 / 0.587 / 0.633 px |
| Selected 180° relative flips / second IPPE branches / quality flags | 0 / 0 / 0 |
| Adjacent center step / median PCA radius, median / p95 / max | 0.109 / 0.212 / 0.225 |
| Adjacent full-orientation step, median / p95 / max | 6.15° / 11.93° / 12.02° |
| Opposite-slot center separation / radius, median (15 available pairs) | 2.076 |
| Opposite-slot full-orientation difference, median (15 pairs) | 179.32° |

The trajectory figures above were computed **read-only from the sealed output**, after pose selection, using the existing PCA-plane diagnostic. They did not feed branch selection or threshold changes. The same late arc remains missing, including a visibly bottle-occluded board at `NP3_336`; the full-turn orbit gate remains `unavailable` at 39/60 declared slots. The 180° checkerboard-label gauge is unresolved globally, and neither physical square size nor the board-to-bottle transform is known. Low distortion-aware reprojection residuals do not prove bottle pose or mesh accuracy. No posthoc Berkeley camera comparison has been run for this calibrated branch. Both disks remained above 10 GiB free (latest check: internal 23,261,196 KiB, external 14,276,476 KiB).
