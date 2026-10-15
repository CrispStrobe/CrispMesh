# GLOBAL mustard pose JSON export — review gate

This is a separate, one-shot **diagnostic conversion** of the already sealed
GLOBAL SfM model; it neither reruns SfM nor improves its failed 46/48 sparse
registration gate. The worker pins the GLOBAL -001 receipt SHA-256
`612429ab5448fdf8bd71bcf6c03e6bafdedf2d38196dce8f51f27b277e8e19dc`,
model SHA-256
`eb5d96ce4379ee32bdef3f946f833fd2c7a85b0a5c818faa4ce799766f74fc00`,
and log SHA-256
`5bed68eb709c8e534262b0f0a5f0689512bcf288db18eb15c8ee4d2a629b1f3d`.
It also verifies the GLOBAL stage command/status/counts, complete output
inventory, all 105 staged match hashes, the sealed -001 photo-derived source
receipt/inventory/JPEGs, SfM binary/build receipts and reviewed GLOBAL source
hashes. No reference data enters the converter.

The exact existing sixth-target converter is checked against its pinned
manifest, license receipt, build log, executable hash and dependency closure.
Its only arguments are `-i` GLOBAL `sfm_data.bin`, `-o` a fresh JSON path,
and `-V -I -E` (views, intrinsics, extrinsics only). The structural checker
requires the same 48 sealed TRAIN filenames, 46 finite proper camera poses,
one intrinsic and no structure/control points. It does **not** yet establish
which two views lack poses; a separate export receipt/JSON hash review and
ID-bound `id_pose` → extrinsic-key validation are required for named results.
The HTML's blank `NP3_252`/`NP3_276` rows remain candidates, not proof.

Default `python3 -m scripts.classical_backend.openmvg_mustard_global_pose_export`
is a read-only preflight. Native `--convert` requires separate review and
may write only to fresh external
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-global-pose-export-001`.
It never reuses or overwrites the GLOBAL model or earlier exports. The gate
reserves 11 GiB free externally **plus** the full 64 MiB output cap, and
11 GiB internally; during conversion it caps observed process-tree RSS at
1 GiB, wall/CPU time at 120 s each, log at 4 MiB and total output at 64 MiB.
One thread is requested. It preserves log, command, exit/status, input/build
seals, JSON hash, integrity counts and partial output on failure, and
rehashes source/converter after processing. Evaluation only; no new geometry
quality, GT, GPL/AGPL shipping or App Store clearance claim.
