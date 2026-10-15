# Fresh turntable pipeline and license boundary

Frozen before the next live run, 2026-09-27. The subject is the same original
60 Berkeley NP3 cracker-box photographs and the existing photo-derived coarse
masks. Reconstruction inputs exclude the Berkeley pose/calibration metadata and
Google reference mesh. All large artifacts and scratch go to
`/Volumes/backups/code/crispmesh-data`, with at least 10 GiB free on both the
Mac internal volume and external SSD throughout.

## Fresh image-only camera stage

Replay the one earlier successful turntable configuration without altering the
source JPEGs/masks: shared SIMPLE_RADIAL intrinsics, mask at feature extraction,
SIFT max 1,800 features / 1,200 image pixels, sequential matching with overlap
8 (no loop detector), automatic incremental mapping, seed 20260927, two CPU
threads. Preserve every option and input hash. The fresh result, rather than the
old 60/60 model, is the only candidate for continuation. A failed replay stays
failed; do not splice old poses into it.

The first acceptance gate is 60/60 registration with finite, structurally valid
tracks and plausible camera intrinsics. Independently compare all named cameras
to sealed Berkeley rig metadata *after* mapping: require center RMS <5% of the
reference camera-layout radius and p95 orientation <10°. These thresholds are
diagnostic, not claims of physical pose truth, and the rig metadata never enters
reconstruction. Retain the full evaluation and all failed/partial artifacts.

This implements masks and a sequential overlap graph. Meshroom's documented
2 px minimum 2D motion filter is a relevant design idea but is not assumed to
exist in the selected PyCOLMAP API. Our result must not be labeled an exact
Meshroom or AliceVision turntable implementation.

## Native complete oracle path

Only after the camera gate, use the *same fresh model* through undistortion,
mask reprojection into those exact undistorted cameras, native dense stereo,
depth fusion, meshing, refinement and texturing. Verify 60 mask/image pairs and
native DMAP exclusion outside the masks. Freeze a high-resolution profile before
launch; object regions occupy only a few percent of the original 1280×1024
images, so a 320 px whole-image depth map is not a credible quality test.
Inspect the bare mesh and compare to the independent scanner reference using
unchanged pre-existing metric settings, keeping whole-reference and object
coverage visible. A finished OBJ/texture or 60 cameras alone is not acceptance.

The locally available OpenMVS v2.4.0 binary is **AGPL-3.0** and is used only as
an evaluation oracle. Its algorithms and result formats can inform an original
adapter or independently implemented compatible alternative; its code and
binary are not approved for the commercial/App Store package. COLMAP itself is
BSD licensed, subject to an exact linked-dependency audit. Selected MVE is
BSD licensed but has not achieved the required object quality. AliceVision core
is MPL-2.0; a full current Apple Silicon dense build is unverified. The
project's present AGPL license also requires an explicit future licensing
decision for the intended App Store distribution.

The previous composed PyCOLMAP→OpenMVS masked run `classical-ycb-native-masked-008`
already reached texturing on this old model. Do not repeat it merely to count
another complete run. The new test addresses **fresh image-only repeatability**
and a camera-gated high-resolution masked continuation.

## Fresh sparse execution, 2026-09-27

The unchanged historical foreground worker was rerun from the original 60 JPEGs
and coarse photo-derived masks in the fresh external folder
`/Volumes/backups/code/crispmesh-data/turntable-sparse-profile-001`. Its
[`turntable-profile.json`](/Volumes/backups/code/crispmesh-data/turntable-sparse-profile-001/turntable-profile.json)
records 2/60 registered images, 127 points, 40.425 s wall time, and a verified
but rejected binary model. The source inputs and pinned PyCOLMAP 3.11.1 binary,
all mapping/feature/matching options, feature counts (40,600), 444 pair rows,
and automatically selected seed images #33/#28 match the earlier 60/60 case.
Keypoint/descriptor bytes and verified two-view geometry differ between the
fresh and earlier databases despite matching feature counts. The fresh seed
model ended at `f=3023.57 px, k=-7.122`; every proposed third camera failed
registration. This is a real repeatability failure in fresh feature/match/SfM
execution, not a resource or wrapper failure. The failed result remains intact.

A separately labeled development diagnostic copied that **freshly extracted**
database, held its image-only starting `SIMPLE_RADIAL` camera
`[1536,640,512,0]` fixed during automatic-pair mapping, then ran one global
BA that refined focal length, distortion, and extrinsics with principal point
fixed. PyCOLMAP changed the copied SQLite file bytes during mapping while its
six logical tables and schema remained identical; the first
`turntable-sparse-fixed-cache-002` attempt stopped on an overstrict byte-level
assertion after making a 60-view model. The corrected fresh output
[`turntable-sparse-fixed-cache-003/turntable-profile.json`](/Volumes/backups/code/crispmesh-data/turntable-sparse-fixed-cache-003/turntable-profile.json)
sealed 60/60, 3,923 points, and 12.106 s worker time, with final
`f=1126.16 px, k=-0.0890`. The separate post hoc
[`independent-camera-gate.json`](/Volumes/backups/code/crispmesh-data/turntable-sparse-fixed-cache-003/independent-camera-gate.json)
passed the frozen rig thresholds: center RMS/radius 2.0014% (<5%) and p95
orientation 3.016° (<10°). This cached-DB diagnostic is not the one-command
fresh pipeline and contributes no Berkeley metadata to reconstruction.

The next [`turntable-sparse-fresh-fixed-004/turntable-profile.json`](/Volumes/backups/code/crispmesh-data/turntable-sparse-fresh-fixed-004/turntable-profile.json)
is a genuinely fresh one-command photo/mask → feature extraction → sequential
matching → fixed-initial-intrinsics automatic mapping → delayed BA run. It
registered 60/60 with 3,917 points in 42.375 s; the final shared camera is
`f=1108.61 px, k=-0.04217`. Source hashes, model hashes, and unchanged logical
feature/match tables are sealed. Its image-only orbit diagnostic has 59/59
steps in one direction and radius coefficient of variation 0.00690. These are
structural checks only. A separate post hoc
[`independent-camera-gate.json`](/Volumes/backups/code/crispmesh-data/turntable-sparse-fresh-fixed-004/independent-camera-gate.json)
(SHA-256 `1656ac8aea32650953f4ec83413cd9022f28ad85ef69cf91af0c46026f868214`)
compared the sealed 60 named cameras to Berkeley rig metadata and passed the
frozen gate: center RMS/radius `0.01894131485` (<0.05) and p95 orientation
`2.89594377°`
(<10°). Berkeley data remained outside extraction, matching, mapping, and BA.
This camera agreement does not establish mesh accuracy; the masked native dense
continuation and bare-shape review remain separate gates.

## Fresh complete OpenMVS oracle and shape result

The camera-gated [fresh native result](/Volumes/backups/code/crispmesh-data/turntable-fresh-openmvs-005/result.json)
(SHA-256 `dafc6ccb2a90f0c5a45748291571455237acd8ae08da01dd80adc10222a3625e`)
used exactly the sealed `004` model and all 60 original photos. It undistorted
the images, reprojected all 60 coarse object masks, and ran OpenMVS v2.4.0 CPU
at full native depth resolution (about 1282×1024), two geometric-consistency
passes, and two threads. This is a **fresh photo-to-textured-mesh completion**,
not a cached-depth continuation. All native stages and source/binary hash checks
passed. The depth audit found zero positive-depth pixels outside the masks in
all 60 DMAPs. Dense fusion yielded 149,337 points; the rough mesh had 70,456
faces, and the refined textured OBJ has 5,351 vertices / 10,698 faces.

Dense reconstruction took 667.897 s; the complete native-stage sequence
(undistort through texture, including depth audit) took about 776 s, plus input
copying. Output was 1,736,236,866 bytes against a 4 GiB cap. The largest sampled
child RSS was 2,129,297,408 bytes, and the external volume had 17,129,648,128
bytes free afterward, above the 10 GiB floor. These timings include the slower
external SSD and are not an optimized throughput claim. The prepared level-1
fallback was not run.

The separate [frozen surface score](/Volumes/backups/code/crispmesh-data/turntable-fresh-openmvs-score-005/score.json)
(SHA-256 `165b8d5169e8bdb917f008864b01c434f4e03455b6ffe688d8ae8a824b2847c9`)
used the independent Google scanner mesh **only after** reconstruction. Its
proper-Sim(3) reference-fitted F1 is **22.09%, 42.23%, 58.66%** at 0.5%, 1%,
2% of the reference diagonal. The reference self-control is 100% at each
threshold. Under the older `006` transform transported by the 60 named
image-derived cameras, F1 is **15.98%, 30.92%, 52.71%**. Camera-gauge transport
itself fits with center RMS 0.374% of the old orbit radius and p95 orientation
0.291°. This is a conditional common-gauge comparison, not independent metric
scale: Berkeley rig poses and the Google mesh have no verified cross-scanner
transform. The earlier full-resolution `011` yielded 42.06% independently
fitted and 35.52% in the `006` gauge at 1%; therefore this run does **not**
establish a meaningful shape improvement, despite better fresh-run provenance.

The [neutral three-view preview](/Volumes/backups/code/crispmesh-data/turntable-fresh-openmvs-score-005/refined-three-view.png)
and a manually rendered common-gauge preview show a recognizable box, but its
common-gauge axis-aligned extent is about 20% and 33% larger than the scanner
mesh on two axes and about 3% smaller on the third. Those ratios depend on the
conditional `006` reference fit and are not physical dimensional measurements.
The bare shape is **rejected for object-quality acceptance**. Neither 60/60
cameras, a closed-looking textured box, nor a complete native pipeline is
equivalent to KIRI-level surface fidelity. OpenMVS remains an AGPL evaluation
oracle and is not part of the approved App Store backend.

A subsequent [read-only stage diagnostic](/Volumes/backups/code/crispmesh-data/turntable-fresh-stage-diagnostic-005/report.json)
(SHA-256 `5f6b1a6bf769516d2b0ff211cc4eb6204828b364fe5d98b7279032b41b6e48e2`)
kept the same transported `006` gauge and scoring settings. Rough-mesh
F1@1% is 31.79%; refinement lowers it only to 30.92%. A uniform sample of
the dense cloud has 35.35% one-way proximity within the same 1% distance,
which is **not** area-weighted surface F1. In that conditional frame, the
dense-cloud X/Y extents are already wider than the scanner reference; meshing
shrinks them slightly. Thus the major shape error is present before meshing,
while refinement adds a smaller decrement. Every sampled dense point projects
inside the coarse foreground support in at least two views, so that support
does not rule out incorrect depth or broad object boundaries. This diagnostic
does not prove that masking alone is the cause, and cross-scanner registration
uncertainty remains.
