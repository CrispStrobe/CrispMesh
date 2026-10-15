# OpenMVG Sceaux `-002`: sealed outdoor photo control

The fresh, evaluation-only Sceaux run completed its five planned stages.
Its [receipt](/Volumes/backups/code/crispmesh-data/openmvg-sceaux-photo-sfm-002/receipt.json)
has SHA-256 `4b82d6546a36d3e3e9b5ff3feec871082b35583152fba6a68ba789d53e4639e6`
and status `completed_pending_geometry_review`. This is an **outdoor software
pipeline control**, not a mustard/object result, physical camera-accuracy
measurement, or dense reconstruction.

Independent read-only verification found that the receipt exactly matches
the sealed 11 source photos and five built binary receipts, all 11 staged
photo hashes, all **56** inventoried output-file sizes/hashes, all five stage
commands, logs, and required stage artifacts. The HTML report independently
matches the receipt: **11 views, 11 poses, one intrinsic, 6,048 tracks, and
19,807 residual observations**. The sparse cereal model
[`sfm_data.bin`](/Volumes/backups/code/crispmesh-data/openmvg-sceaux-photo-sfm-002/sparse/sfm_data.bin)
is 902,327 bytes with SHA-256
`3f7409c6fad154eaeefbbfb4a02354a06618d0dd22285a1e63723acee2c3cff2`;
the [SfM HTML report](/Volumes/backups/code/crispmesh-data/openmvg-sceaux-photo-sfm-002/sparse/SfMReconstruction_Report.html)
has SHA-256 `8d91b9d7490081c1db07a227398761d62e5f179253b8fc91af1021fba6fba0fa`.

| Stage | Wall time | Peak RSS | Log bytes | Verified log SHA-256 |
| --- | ---: | ---: | ---: | --- |
| Listing | 0.534 s | 2,848 KiB | 755 | `7aaa88b065799f743fece767dd194568027d10509bb3cf5965d84bd1a4df6125` |
| SIFT_ANATOMY features | 6.330 s | 690,736 KiB | 552 | `9e6a991f2234b81b5449becd90dfdc4973412444eaf3c757657648f18e98d63b` |
| Putative matches | 0.530 s | 448 KiB | 1,403 | `c1b7423a9913e4fb666d927f892416e115a91557194a900d074fabb9d2a18a1c` |
| Essential-matrix filter | 1.049 s | 19,552 KiB | 1,332 | `60f72facb7318c2fdefc17813328c8098e0f39042d50d5611f6d656f02a77135` |
| Incremental SfM | 2.126 s | 39,088 KiB | 16,483 | `df8f740bf400fdf02fa2f7cd1485cb00c05f7d40d41871c7072643234bff6628` |

Every stage was below its individual wall limit (120/420/420/420/420 s),
4 GiB RSS cap, and 16 MiB log cap; the five stage times sum to 10.569 s.
At verification the entire output tree was **48,439,660 bytes** (<1 GiB),
external free was **14,361,636,864 bytes**, and internal free was
**22,428,549,120 bytes** (both >11 GiB). The output was not modified.

The HTML's **0.392261 px scene RMSE** and the SfM log's **0.140605 px
residual median** are different internal reprojection-error summaries; neither
establishes pose accuracy against an external reference. Full camera-orbit
shape, alignment, and physical geometry remain unverified. No dense MVS,
meshing, or object reconstruction was run, and OpenMVG's unresolved shipping
license/dependency questions still confine this build to evaluation use.
