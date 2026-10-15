# Mustard Apple Object Capture preview result

The single approved Apple-only `.preview` run followed the
[frozen plan](MUSTARD-APPLE-PREVIEW-PLAN.md). The launcher returned
`complete` on the exact 48 sealed TRAIN JPEGs in 16.908 seconds. Its
[external result](/Volumes/backups/code/crispmesh-data/apple-mustard48-preview-001/result.json)
is 9,432 bytes, SHA-256
`98b4e9b09dd96f5d999acad0f4fbbc060966c595cf658bb83d96e551659b4a59`.
The [USDZ](/Volumes/backups/code/crispmesh-data/apple-mustard48-preview-001/model.usdz)
is 339,420 bytes, SHA-256
`f0542aab70b34e8036376adfb5f6afe79d2360b1284aa7b3312840cf09049905`.
The launcher recorded 351,419 bytes in the entire run directory, below its
1 GiB cap, and 240,025,600 bytes peak sampled direct-child RSS, below 8 GiB.
The log cap was 16 MiB and the deadline was 15 minutes. Direct-child RSS
does not include all Apple system/XPC service memory.

The launcher rehashed all 48 inputs and the compiled probe after processing;
both unchanged flags are true. The stage report retained SHA-256
`bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`,
the probe binary retained
`f6fe2bbb5677177eeba8b1b19b8a1c49c2e785305b9cfa1f781b67ae10bd277b`,
and `launch.py` retained
`615fcb2a028b07a37f665c9080a10f6909b4bbae3de429bb80aed9f4deb101e5`.
The input folder contained only the 48 original TRAIN JPEGs, totaling
48,683,099 bytes. No masks, held-out views, depth, reference mesh, or
supplied/reconstructed poses were passed to Object Capture. Internal and
external free space remained above 10 GiB after the run.

Installed `usdchecker --arkit` exited successfully and printed `Success!`,
with two preceding USD plugin-registration diagnostics. `usdtree` found one
Mesh under `/ObjectCapture/Geometry`. The bounded
[structural review](/Volumes/backups/code/crispmesh-data/apple-mustard48-review-001/result.json)
is SHA-256 `3ba1dc17ee04b7db501a173afe08522760d0456584c3eaf015bd98e9e6595c7d`.
It exported the exact mesh to a 35,406-byte untextured PLY, SHA-256
`c460f11fea76f9f2a64e8d60a9ec3ddcc6241f503aedc89745ab1c8a6b430004`:
935 vertices, 1,847 triangles, finite coordinates, valid face indices,
no repeated-index faces, and no zero-area triangles. The authored extents
are approximately 0.787 × 0.319 × 0.719 in USD units; authored
`metersPerUnit=1` is not independently verified metric scale.

The [neutral three-view preview](/Volumes/backups/code/crispmesh-data/apple-mustard48-review-001/preview-neutral.png)
shows a broad, thin surface with a much smaller upright central form.
Against the original TRAIN photo, this is consistent with the turntable or
support surface being reconstructed along with the mustard bottle. The
root reviewer independently inspected the neutral preview and two original
TRAIN photos and agreed that the broad support sheet dominates the small
bottle-like center. The output is structurally inspectable but fails an
object-only visual-quality check. No cleanup, crop, component removal, or
retry was applied.

This run shows that the Apple workflow produced a valid USDZ candidate on
the 48 RGBs. It does not establish a clean mustard mesh, metric accuracy,
held-out appearance, or physical camera correctness. The separate 60-photo
cracker-box result uses different inputs and is not a controlled timing or
shape-quality comparator. The Apple workflow remains evaluation-only, not
the cross-platform backend.
