# OpenMVG mustard intrinsic-`NONE` `-002`: same registration failure

The evaluation-only, photo-derived SfM ablation at
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-fixed-002`
finished its single incremental SfM stage but **failed the 48/48 camera gate**.
Its sealed receipt has status `failed_registration_or_sparse_gate`, SHA-256
`5445b512ac6d171e8097134d9c86d84e6428b1d958a3088593d6234e5bb1fa24`.
This audit made no native run, reference-pose comparison, or data change.

| Existing report | `-001`: `-f ADJUST_ALL` | `-002`: `-f NONE` |
| --- | ---: | ---: |
| Views / registered poses | 48 / **46** | 48 / **46** |
| Sparse tracks | 222 | 162 |
| HTML observation count (`#residuals`) | 1,497 | 1,292 |
| HTML scene RMSE | 0.517095 px | 0.712032 px |
| Final log median absolute residual component | 0.124623 px | 0.123754 px |

The HTML scene RMSE and log median residual are **different statistics**;
the lower median in `-002` does not offset its fewer tracks, higher scene
RMSE, or missing cameras. Both HTML per-view tables have blank observation
rows for the **same IDs 6 and 7**, `NP3_042` and `NP3_048`; this is consistent
with those views lacking final poses. The exact final camera identities were
not extracted from `sfm_data.bin`, so the row interpretation retains that
qualification.

The source and both logs give a narrow failure trace. Incremental SfM first
tries view pair **(6,7)** (`NP3_042`,`NP3_048`): essential-pose estimation and
two-view BA run with 69 tiny-scene tracks, but the subsequent “InitialPair
Inlier” residual section has no retained-track statistics. In pinned
[`MakeInitialPair3D`](https://github.com/openMVG/openMVG/blob/01193a245ee3c36458e650b1cf4402caad8983ef/src/openMVG/sfm/pipelines/sequential/sequential_SfM.cpp),
tracks survive only if angle >2°, cheirality, and residual gates pass; the
function returns false when its retained structure is empty. The fallback
also logs essential-estimation failures for (0,1), (36,37), and (2,3), then
uses **(24,25)** (`NP3_180`,`NP3_186`), with 32 retained initial tracks. That
same sequence appears in both runs. The logs do **not** identify which
post-BA gate removes pair (6,7), nor prove poor matching, a wrong focal
length, or a specific physical pose error. The source mutates its working
pose/remaining-view state before returning false for an empty seed, and
later performs unstable-pose cleanup; the logs do not isolate which step
ultimately leaves IDs 6 and 7 absent.

Read-only receipt verification found all **116** `-002` inventoried files
present with matching sizes/SHA-256, including all **105** byte-identical
staged match-directory files. The original `-001` receipt and all 48 sealed
TRAIN photos still match the hashes embedded in `-002`. The `-002` log is
80,653 bytes, SHA-256
`7eaab977dddc6f1dc748fffc1d1c0affebf150bd762f56e333fdf9308898513d`;
its model is SHA-256
`4147cd6ea9e0848dd97210701535371c100425125dc83df07a67482ebd8eca55`,
and HTML report is SHA-256
`7a49f5412178c2509bd7aefc0cff9e9e67b11b2714d60dacdd763a597cd06df5`.
The stage returned 0 in 1.129 s with 13,088 KiB peak recorded RSS. Success
of the process is **not** success of the reconstruction gate. This ablation
does not justify dense reconstruction, mesh work, or a shipping-license claim.
