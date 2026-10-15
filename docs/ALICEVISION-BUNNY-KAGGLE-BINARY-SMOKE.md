# AliceVision v3.3.0 Kaggle binary smoke: CLI loads, pipeline untested

On 2026-09-28, a **private CPU-only** Kaggle script was run under account
`chr1str` to test the official AliceVision Linux binary before any bunny data
transfer. Kernel [versions 1–4](https://www.kaggle.com/code/chr1str/crispmesh-alicevision-binary-smoke)
progressed from a scratch-path preflight failure to both tested CLIs printing
their help text with no unresolved shared libraries. The final strict gate
remained `runtime_compatible=false` because both `--help` commands exited 1,
not 0. This is **not** an AliceVision bunny run.
No photographs, scanner files, PRO poses, or datasets were attached or
uploaded. No GPU was enabled or detected.

The final submitted [script](../scripts/remote_quality/alicevision_binary_smoke/alicevision_binary_smoke.py)
is SHA-256 `395892c28ccd4a68939c2c424c0ff013a8d1283442769564b8061ee1e4e9f4d7`
at commit `b82ede5`; the [private CPU-only metadata](../scripts/remote_quality/alicevision_binary_smoke/kernel-metadata.json)
is SHA-256 `f798110f5299a7e21d040cca99449d254349ef209de933c00fee3e810709a5e0`.
All ten offline safety tests passed. The script downloaded the official
`AliceVision-3.3.0-Linux.tar.gz` asset, exactly 1,505,191,867 bytes with
SHA-256 `f43f498312859af627f2f7f65a6d33c2a3411b37989b8b680c04c8c690dcb640`.
It inventoried 2,307 archive members containing 2,344,265,238 regular-file
bytes on each download. The final v4 script took 106.672 seconds, below the
15-minute hard deadline.

In v2, both `aliceVision_cameraInit --help` and
`aliceVision_depthMapEstimation --help` exited 127 because bundled
`libaliceVision_cmdline.so.3` was not on the loader path. V3 derived the
bundled `aliceVision/lib` directory from the verified extraction and set
`LD_LIBRARY_PATH` only for the probes. All shared libraries then resolved,
including bundled CUDA runtime libraries, but both commands aborted because
`ALICEVISION_ROOT` was unset. V4 verified the bundled
`aliceVision/share/aliceVision/config.ocio` and set `ALICEVISION_ROOT` to the
extracted `aliceVision` install root for the probes. Both CLIs then printed
normal extensive help text; neither had missing dependencies or a timeout.
Each nevertheless returned 1, and the strict zero-exit gate therefore did
not pass. Help output is evidence that the CLIs load and parse options, not
that image processing or CUDA works. `nvidia-smi` was absent, as expected
for the CPU-only notebook.

The downloaded v2 JSON receipt is at
`/tmp/alicevision-binary-smoke-v2.oOP6I6/alicevision-binary-smoke.json` on the
local host, SHA-256
`a4fd24f46c0069a23460b39e14f52fc4c5bf4ab40b98f5d9fa4be29c385daa2b`.
The v3 receipt is
`/tmp/alicevision-binary-smoke-v3.zF3vw4/alicevision-binary-smoke.json`,
SHA-256 `a214583bbe48574f59e8cb6045c56b54004b051a6bae8bf4c9016b79b18a8e15`.
The v4 receipt is
`/tmp/alicevision-binary-smoke-v4.cUYj5i/alicevision-binary-smoke.json`,
SHA-256 `e0e8efcc82c190b5880972226586764a6724bd36ada6de096818f50adc612e31`.
The preceding v1 probe never downloaded the archive: `/kaggle/temp` did not
exist in the notebook image, so its preflight failed at 0.0 seconds. Its
downloaded receipt is
`/tmp/alicevision-binary-smoke-v1.3yqC4u/alicevision-binary-smoke.json`,
SHA-256 `b99ba7bf17110a50e7ac3c3e4fe3575131833211ba0d3cac39eb156d628d86f2`.
The path-only v2 change moved ephemeral scratch to Kaggle `/tmp`, retaining
the same private, no-photo configuration and bounds.

No full bunny AliceVision pipeline result, depth map, or mesh exists from
these no-photo runs. A separately reviewed three-photo CPU sparse attempt
followed, as described below. No GPU capability or dense stage was tested.
The supplied scanner remains evaluation-only.

## Private three-photo CPU sparse attempt: worker completion unverified

A fresh external-SSD folder
`/Volumes/backups/code/crispmesh-data/alicevision-bunny-three-photo-dataset-001`
contains only `frame_0000.png`, `frame_0022.png`, and `frame_0044.png`, plus
dataset metadata, attribution/readme, and a selection manifest. These map
to numeric `bunny_0`, `bunny_2`, and `bunny_4` in the sealed prepare manifest.
The photo payload is 5,206,273 bytes. Hashes are respectively
`c40614be25fd5f2d9113bbb8e64156598b1fc7c59f66bd4542d837ea3488c9e7`,
`82c50e9fb350ae7f1324398d0fff30271fa66438fea7d585fa2904c8ce1087af`,
and `16c67b9419c4f3e8a219be02d45c9e9d18595a6fd73d9f2c19205edfb86b4543`.
The selection manifest SHA-256 is
`e36fcbc214d76d3c29658c1b1595e4d6d3b2f0318c6523096de232d5e37483b9`;
the private dataset metadata SHA-256 is
`578d99284c49e34e60f90ac44f32418df45ac0e56448c5f421c08a90adc10d51`.
The rights note attributes Vodianyk, Nava-Baro, and Popov, *3DLF-Scan*,
version 1, DOI 10.17632/ngvgpsvd8b.1, CC BY 4.0, and flags the physical
Stanford bunny derivative's uncleared commercial rights. No scanner, PRO
poses, depth images, or supplied calibration are in the package.

The dataset was created **private**, without Kaggle's public flag, at
[version 1](https://www.kaggle.com/datasets/chr1str/crispmesh-bunny-3photo-research-smoke).
The authenticated API read-back reported owner `chr1str`, `is_private=True`,
and current version 1. Its remote file listing contains exactly the three
PNG files, `README.md`, and `selection-manifest.json`; the dataset metadata
file was not part of the downloadable payload. A fresh remote download was
independently SHA-256 checked against all three prepared-photo hashes and
the selection-manifest hash above. The account's current GPU quota remains
unknown; no GPU was requested for this kernel.

The [CPU sparse-smoke script](../scripts/remote_quality/alicevision_three_photo_smoke/alicevision_three_photo_smoke.py)
and [private kernel metadata](../scripts/remote_quality/alicevision_three_photo_smoke/kernel-metadata.json)
re-download the same pinned 1.505 GB release into Kaggle `/tmp`,
validate the two possible dataset mount paths and all photo/manifest hashes,
then run CameraInit, SIFT FeatureExtraction, ImageMatching,
FeatureMatching, and Incremental SfM. Existing OpenMVG tracks were used
solely to select overlapping frames (pairwise 155/143/85 common tracks;
73 across all three), not as AliceVision inputs. The job is CPU-only,
private, limited to 20 minutes, 8 GiB release expansion, 1 GiB ephemeral
work, a 4 GiB free-space floor, and at most 1 MiB durable JSON receipt.
The final self-contained script SHA-256 is
`e1f549dd1c3c2193913893fe2b19b426f4e7911a03823bb38173ab96f06c734f`
at commit `13dafc2`; the kernel metadata SHA-256 is
`150daff53451c8cc513560a62dc8d323248f2ed6d2379149f883c281cc5b9c2c`.
All 15 relevant offline tests passed. This is neither a dense/GPU test nor
a 73-photo result.

The [private CPU-only kernel](https://www.kaggle.com/code/chr1str/crispmesh-alicevision-bunny-three-photo-cpu-smoke)
had three versions:

1. Version 1 failed before running the script because Kaggle exposed only
   the declared `code_file`, not its imported helper file. Its log records
   `ModuleNotFoundError: No module named 'alicevision_binary_smoke'`
   (downloaded log SHA-256
   `24ce48bca3fd91df1693769e280fc155e6e054150216a2bfe12cbed9f0e99b68`).
   No photo or AliceVision command ran.
2. Version 2 made the script self-contained. Its worker log printed
   `{"status": "cpu_sparse_smoke_complete", ...}` at 88.18 seconds and
   notebook conversion ended around 97.32 seconds (downloaded log SHA-256
   `6779134728feb4bb56f9811c18157469ee8390d967c2ff6b1f60c1368b2ddd80`).
   This is evidence that the script *reported* all stages had returned
   successfully, but Kaggle ultimately marked this version
   `CANCEL_ACKNOWLEDGED` and exposed **no downloadable JSON receipt or SfM
   model**. Stage-by-stage counts, pose count, and model quality therefore
   cannot be independently verified.
3. An identical version 3 retry, with unchanged script and dataset, also
   ended `CANCEL_ACKNOWLEDGED`. Its downloadable log was only `[]`
   (SHA-256
   `4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945`),
   with no receipt. No further retry was made.

The current blocker is Kaggle worker finalization/artifact availability,
not a demonstrated bunny geometry failure. The v2 completion line must
not be promoted to a verified three-camera reconstruction. No full 73-photo
AliceVision run, depth map, or mesh was produced. Further work needs a
stable, downloadable three-photo receipt and independently checked sparse
model before any full-input or GPU/dense continuation.
