# OpenMVG mustard geometric matches versus coarse pose-support masks

This is a **read-only TRAIN diagnostic**, not a match filter or a new SfM arm.
No native feature extraction, matching, reconstruction, reference pose, depth,
scanner mesh, held-out photo, or mask edit was used. The [bounded Python
parser](../scripts/classical_backend/openmvg_match_mask_audit.py) and four
synthetic tests audit the existing `-001` essential-matrix matches against
the previously accepted coarse pose-support masks.

## Source/data-format feasibility

Pinned OpenMVG v2.1 `indMatch_utils.cpp` loads `matches.e.bin` with cereal
`PortableBinaryInputArchive`. The pinned `indMatch.hpp` defines a sorted
`map<Pair, vector<IndMatch>>` with two `uint32_t` view IDs per key and two
`uint32_t` feature indices per match; cereal's map/vector serialization
prepends 64-bit element counts and portable binary begins with a one-byte
endianness marker. The observed file starts with little-endian marker `01`
and 458 map entries. Pinned feature IO writes text `.feat` rows as
`x y scale orientation`; line order is the zero-based match index. The
sealed `sfm_data.json` maps view IDs 0–47 to exact TRAIN filenames. Thus a
read-only, non-native join is possible without decoding descriptors or
running an OpenMVG CLI.

The parser rejects oversized/truncated/trailing binary data, unsorted or
invalid view pairs, impossible counts, invalid feature rows or indices,
nonbinary/wrong-size masks, nonfinite coordinates, and changed source hashes.
It reads at most 1 MiB of match bytes and 256 KiB per feature file; the
observed match file is 210,849 bytes. Mask lookup uses the nearest image
pixel `floor(x+0.5), floor(y+0.5)` and reports out-of-frame coordinates
separately. For each indexed verified match it counts **both endpoints in**
the 255-valued support mask, **one in**, **neither in**, or out of frame.
The same source files are rehashed after counting.

Input seals independently checked in the script are: `-001` SfM receipt
`e45c400dcf71a3ba86fa22f7af2138418d6dc511fddcd6d8470ef33599804744`;
`matches.e.bin`
`1e2eebfee9a5326431f4d9c415d88c957e8e30b99899302e33f0397daca2becc`;
`sfm_data.json`
`8a61622cc22f76a6bb7dbcbf5a4e00725afd6a57b1e20095dabafab931112033`;
the 48 individual `.feat` hashes in the `-001` receipt; and the 48 mask
hashes in the TRAIN stage report at
`/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001/stage-report.json`,
SHA-256 `bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0`.
The 60-slot [capture-order profile](../tests/datasets/ycb_np3_full_turn_profile.json)
is SHA-256 `bb00d17e1940f121dc12f7ddb31d4adf80b7e9a935f18e5535cfc1a2cde1998d`.
Those slots are acquisition labels, **not supplied camera poses**.

## Counts on all 48 TRAIN views

The sealed E-filter file contains **458/1,128 possible view pairs** and
**25,439** indexed matches. All 25,439 endpoints pairs have valid feature
indices and image coordinates. The 48 masks are binary 1280×1024 and their
255-valued areas total 846,453 pixels, **1.345%** of all image pixels.

| Cyclic slot distance | Possible / verified pairs | Matches | Both in | One in | Neither in |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 36 / 36 | 2,796 | 201 | 1 | 2,594 |
| 2–4 | 108 / 82 | 4,632 | 109 | 1 | 4,522 |
| 5–9 | 192 / 84 | 4,543 | 33 | 2 | 4,508 |
| 10–19 | 384 / 136 | 7,152 | 26 | 4 | 7,122 |
| 20–29 | 384 / 113 | 5,960 | 14 | 5 | 5,941 |
| 30 (opposite slot) | 24 / 7 | 356 | 2 | 1 | 353 |
| **Total** | **1,128 / 458** | **25,439** | **385** | **14** | **25,040** |

There are **zero out-of-frame** endpoints. Only 151 verified pairs have at
least one both-inside match; 295 pairs have no inside endpoint at all.
For the failed initial-seed pair `NP3_042`/`NP3_048`, 15 of its 105 E-filter
matches are both inside coarse support and 90 are neither. That does not
identify which later SfM seed-quality condition removed its triangulated
tracks. Full per-pair and exact-distance counts are returned by the parser
in memory/stdout; no analysis artifact was written to the external tree.

The stage report explicitly calls the masks **“accepted coarse pose
support, not object silhouette or ground truth.”** Therefore `both in` is
not a proven bottle match: masks may include rotating board or support.
`Neither in` is not a proven stationary-background or false match: coarse
masks may miss real bottle pixels. The very small mask area makes raw overlap
fractions especially sensitive to mask coverage. The steep fall in
both-inside counts with cyclic separation is a useful support diagnostic,
not proof of true object correspondence or camera recovery. Separating
bottle, moving board, and stationary background requires independent visual
labels or a separately reviewed semantic mask; reference poses/mesh must
not be used to tune this match set.
