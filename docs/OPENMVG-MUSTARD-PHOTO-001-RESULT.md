# OpenMVG mustard TRAIN photo-only `-001`: registration gate failed

The evaluation-only OpenMVG v2.1 run at
`/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-001` finished
all five native CLI stages, but its sealed [receipt](/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-001/receipt.json)
has status **`failed_registration_or_sparse_gate`**, not success. Receipt
SHA-256: `e45c400dcf71a3ba86fa22f7af2138418d6dc511fddcd6d8470ef33599804744`.
The SfM HTML report records **48 views, 46 poses, one intrinsic, 222 sparse
tracks, and 1,497 residual observations**. The two report rows lacking
observation/residual statistics are view ID 6 (`NP3_042`) and ID 7
(`NP3_048`), consistent with the two missing registered poses. This is a
camera-coverage failure for the sealed 48-photo TRAIN task; it is not a
physical accuracy measurement.

Independent read-only verification rehashed the pinned 48 TRAIN JPEGs and
metadata (48 distinct 1280×1024 images; no held-out name), all 48 staged
copies, all **170** receipt-inventoried output files, all five stage logs and
required stage artifacts. The staged names equal the sealed TRAIN list,
and stage commands contain only the photo pipeline's paths—no masks,
Berkeley poses, scanner, mesh, reference, or held-out input. The
[sparse model](/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-001/sparse/sfm_data.bin)
is 67,169 bytes, SHA-256
`dc11f9b3a74809ebc080260b360ff1dd0e6f9f9526ea151aac002ace75b2df8b`;
the [HTML report](/Volumes/backups/code/crispmesh-data/openmvg-mustard-photo-sfm-001/sparse/SfMReconstruction_Report.html)
is SHA-256 `2d071e87db65a987623c460c3168aafea3fbde334dbb6356ab4d5fe0af62307d`.

| Completed stage | Wall time | Peak RSS | Log bytes | Verified log SHA-256 |
| --- | ---: | ---: | ---: | --- |
| Listing | 0.527 s | 544 KiB | 1,030 | `e8188440c32afa213af8f47240b0c7060b29d106ba95a1433ed210067b7439d7` |
| SIFT_ANATOMY features | 4.233 s | 152,752 KiB | 839 | `486329625e1f522ff8665172bcab8a5f958089aede28f839f4954b478a455dbe` |
| Putative matches | 0.531 s | 704 KiB | 1,982 | `cea9a91783b2f288da1564ff278528bcde9a62e03d66837b2703969f459e7399` |
| Essential-matrix filter | 5.286 s | 13,296 KiB | 1,976 | `44b78c923a4fc9235312cb68ac8ea4495c7c5b05126e056594e16744fd2fee7a` |
| Incremental SfM | 1.070 s | 13,904 KiB | 78,547 | `f0b79d015501feaa08b6de00855492c6d57e682696b4af2a06843657dd08f64b` |

Every process returned a stage artifact and stayed below its wall, 4 GiB RSS,
and 16 MiB log caps; stage wall times sum to 11.647 s. At audit time the
output tree was **54,004,084 bytes** (<1 GiB), external free was
**14,308,057,088 bytes**, and internal free was **22,458,130,432 bytes**
(both >11 GiB). These resource checks do not change the failed registration
decision. There was no dense/MVS or meshing step. The low track count and
missing poses require a frozen model/metric protocol before any reference-pose
comparison; this audit did not read Berkeley poses or modify the output.
