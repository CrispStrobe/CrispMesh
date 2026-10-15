# Mustard OpenMVG ID-bearing pose export: sealed validation

Status: **implemented, synthetic-tested, and run read-only on the sealed export**.
No converter, SfM, dense, or reference process was run by this validator. The
output is at
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-pose-export-001/`
with `sfm_camera_poses.json` and `receipt.json`. After the one-shot export was
reviewed, the validator pinned receipt SHA-256
`ab73c2b729adb8abd1f84d97f1908b26635cd8caa6d2d1756185d9a31a34dac3`,
JSON SHA-256
`c1e0ce4431245ca9392dd82b64cc8007d69e0c41a2e87cf78cec10f33ecc1124`,
and receipt schema `openmvg_mustard_pose_export_v1`. The earlier unset-pin
behavior remains synthetic-tested as a fail-closed branch.

## Exact v2.1 data contract

Pinned OpenMVG v2.1 source (`01193a245ee3c36458e650b1cf4402caad8983ef`)
and its checked-in cereal JSON sample establish the shape. `-V -I -E` exports
only `views`, `intrinsics`, and `extrinsics`; `structure` and `control_points`
are absent or empty. Each `views[*]` has `key` and
`value.ptr_wrapper.data.{local_path,filename,width,height,id_view,id_intrinsic,id_pose}`.
An intrinsic has `key` and `value.ptr_wrapper.data` with width, height,
`focal_length`, `principal_point`, and any model-specific distortion values.
Each `extrinsics[*]` has `key` and `value.{rotation,center}`; `pose3_io.hpp`
serializes the 3×3 rotation by rows and center as three numbers. `IndexT` is
uint32, with `4294967295` denoting an undefined ID. The converter's selected
`-V -I -E` switches are verified from pinned `main_ConvertSfM_DataFormat.cpp`.

The [validator](../scripts/classical_backend/openmvg_mustard_json_pose_validator.py)
requires the exact sealed 48 NP3 TRAIN names and 1280×1024 dimensions, one
finite positive-focal intrinsic, exactly 46 distinct finite poses, valid
`id_view`/`id_intrinsic`/`id_pose` links, and no shared pose ID between views.
It checks all rotation rows are unit length and orthogonal within 1e-5 and
determinant is +1 within 1e-5. It rejects duplicate JSON keys, nonfinite
numbers, unknown names, duplicate IDs, and populated structure/control
points. The named camera centers come from explicit cereal pose keys, removing
the PLY/HTML source-order assumption in the earlier sparse diagnostic.

The output keeps the **failed 48/48 registration gate** explicit even when
46 linked poses validate. It computes the TRAIN-label 6°, 12°/full-turn,
and 180° center-chord diagnostics defined in the earlier
[sparse diagnostic](OPENMVG-MUSTARD-SPARSE-DIAGNOSTIC.md), with no reference
poses or mesh. Those chord flags were defined after the SfM result was seen;
they are exploratory internal trajectory-shape measurements, not a
predeclared acceptance gate, metric camera accuracy, or object geometry
quality. Rotations are checked for mathematical validity, not against
supplied rig orientation. Track membership and independent reprojections
remain outside the `-V -I -E` export.

The sealed receipt binds the source binary model SHA-256
`dc11f9b3a74809ebc080260b360ff1dd0e6f9f9526ea151aac002ace75b2df8b`,
the failed-run receipt SHA-256
`e45c400dcf71a3ba86fa22f7af2138418d6dc511fddcd6d8470ef33599804744`,
the JSON path and JSON SHA-256, and the exact `-V -I -E` command. Both receipt
and JSON are rehashed after validation. The validator also checks the
receipt's view/pose/intrinsic counts and named posed/missing inventories
against the parsed JSON.

## Read-only ID-bound result

The 66,004-byte JSON contains exactly 48 named TRAIN views, one intrinsic,
and 46 finite named poses with proper orthonormal rotations. The missing
poses are `NP3_042.jpg` and `NP3_048.jpg`; status is
`failed_48_of_48_gate`. Explicit `id_pose` links reproduce the earlier
source-order orbit values exactly: median 6°/opposing chord ratio 0.5954
(ideal circular value 0.0523), opposing chord p90/p10 21.01, and 000°–348°
closure/other-12° median 0.3767. The first two exploratory orbit flags fail;
the local closure flag passes. This confirms the named mapping of the
earlier diagnostic, while remaining an internal photo-only result.

The exported `pinhole_radial_k1` intrinsic is focal length **552.94 px**,
principal point **(552.37, 1019.12)** on a 1280×1024 image, and
`disto_k1` **8.16144**. The validator reports these raw values and generic
sanity flags: focal shorter than half the frame width, principal point
outside the central half of the frame, and absolute radial coefficient
above one. These simple flags were added **posthoc after the export was
seen**. They are descriptive, not a predeclared pass/fail gate or a camera
comparison against Berkeley calibration. They support concern about the
image-only self-calibration but do not prove which parameter caused the
trajectory failure.

The [synthetic tests](../scripts/classical_backend/test_openmvg_mustard_json_pose_validator.py)
cover the exact 46/48 named map, circular trajectory, broken links and
names, nonfinite centers/intrinsics, non-orthonormal and reflected rotations,
duplicate JSON keys, receipt binding, and the unset-pin abstention.
