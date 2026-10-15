# Frozen train/held-out protocol for the two new YCB objects

This protocol uses only the copied acquisition manifests for
`006_mustard_bottle` and `035_power_drill`. It does not reconstruct, fit to a
Google mesh, create masks or download data. The two manifest SHA-256 values
are pinned in `scripts/object_dataset/evaluation_package.py`; the corresponding
original photos and references remain on the VPS. The archive digests were
observed at acquisition, not independently authenticated publisher pins.

Sort the 60 NP3 frames by the recorded turntable angle 0°–354° in steps of
6°. Freeze held-out angles where the zero-based sorted frame index modulo 5
equals 4: **24°, 54°, 84°, 114°, 144°, 174°, 204°, 234°, 264°, 294°,
324°, 354°** (12 frames). The other 48 are training-input candidates.
This deterministic rule is fixed before reconstruction. Angle labels are
used **only** to partition images, not to seed or constrain SfM, estimate
camera poses, select alignments or tune a method. An all-image SfM run would
leak held-out photos into camera estimation and cannot claim an end-to-end
held-out result; localization of held-out cameras must be separately disclosed.

The validator checks manifest schema/status, exact object and 60-angle
inventory, unique file names and hashes, roles and selected-source member
names, distinct Google reference provenance, and train/held-out disjointness.
With `--dataset-root`, it also rehashes all 60 original JPEGs and both
reference PLY files from the VPS directory, rejecting missing files, size/
hash drift, symlinks or paths escaping the root. No reference path appears in
`training_inputs`; Google mesh entries are **evaluation-only**. Without a
dataset root, the output is only a pinned-manifest split plan, not a local
file validation. Results are written only to a fresh, bounded JSON path.

**Neither object currently has a reviewed, photo-derived object-mask set.**
Every output therefore states `runnable_image_only_training_package=false`
and `mask_status=pending_photo_derived_review`. This split/inventory is not a
ready neural training package or a request to substitute supplied Berkeley
masks. No mask is generated or tuned against reconstruction/reference scores.

The photographs are one turntable elevation. Although angle labels cover a
full rotation, they do not cover undersides and may miss narrow concavities,
handle interiors or low-texture/specular regions. The Google scanner is a
different capture system with no verified NP3-frame registration here.
Input acquisition and deterministic splitting do not imply usable camera
recovery, geometric completeness or physical-ground-truth accuracy.

## Status

No reconstruction or quality score is produced by this protocol. Manifest-only
split plans were emitted locally at
`build-opencv/ycb-evaluation-package-001/mustard.json` (SHA-256
`115bd5137803e68c54bc4656a31643a0d98388cd615caa591b06ad5257d23734`)
and `build-opencv/ycb-evaluation-package-001/drill.json` (SHA-256
`c1016b76a33e97f40d28fa5f8a74cf77bd50cde8a254304d91b6b296cc09df58`).
Both report 48/12 photos and `manifest_only_split_plan`; they do **not**
claim that local original files were rehashed.

Root subsequently ran full validation on the VPS after reviewing the helper
and tests. Both objects passed all 62 asset size/SHA-256 checks (60 original
photographs, original scanner PLY, converted scanner PLY). Outputs live in
`/mnt/storage/crispmesh-data/ycb-evaluation-001/`; small report copies are in
`build-opencv/ycb-evaluation-vps-001/`. The helper SHA-256 was
`b7dcecd236c7c3644f15a5ddd9fe6b148be74f87ebc9d41aa3277315dc427d5f`.
Mustard report SHA-256: `45b6ed1430627a747eaeb8d4dff7d2c49502083cca0a7914e25aacd45b594425`;
drill: `65cae1f7b7b75bbe2e98b015937627864d1705c8b743211345033533e4fd7f3f`.
Each seals the 48/12 split; both remain non-runnable training packages until
photo-derived masks are prepared and reviewed. No reconstruction or quality
score is produced by this protocol.
