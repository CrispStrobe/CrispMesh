# OpenMVG bunny HIGH sparse run 001: independent receipt audit

The photo-only sparse run completed its five stages and remains
`completed_pending_geometry_review`. It is a valid sealed sparse artifact;
this audit does **not** clear its cameras for dense reconstruction. No SfM or
dense stage was rerun for this review.

## Identity and input

- Output: `/Volumes/backups/code/crispmesh-data/openmvg-bunny-high-photo-sfm-001`
- Receipt SHA-256: `97cf72ab72c75a88f04d97baa29c6de49a1640103d7143dd151bc2efe54c507d`
- Sparse model SHA-256: `aea2b3129cb317663435461fd12fa4c0078c57211fc1c08cf7b1a29ad6902975`
- HTML SfM report SHA-256: `0fa1014793eddc438861fa54ff0978fa9da109eacf0976236ee6c65d21511faa`
- The staged image directory contains exactly the 73 `frame_0000.png` through
  `frame_0072.png` files. Every staged and current source SHA-256 matches the
  receipt, the sealed preprocess manifest, and the prior COLMAP input manifest.
  Total input bytes: 121,809,661. No masks, supplied camera data, depth, or
  scanner data appear in the worker input directory or stage commands.

All 276 recorded output files match their actual sizes and SHA-256 hashes;
there are no unrecorded files or symlinks. The five recorded commands,
artifact hashes, log hashes, and evaluated native binary hashes also match.

## Sparse result and camera checks

The HTML report lists 73 views, 73 poses, one intrinsic, 6,526 tracks, and
23,619 observation residuals. A separate, read-only export of the sealed
`sfm_data.bin` with the evaluated OpenMVG converter found **73 distinct named
views linked to 73 present poses**, with the exact expected filenames. The
converter binary seal was verified before use. Its temporary JSON was removed
after inspection; the sparse model was unchanged.

The one refined `pinhole_radial_k1` intrinsic is 1749 × 1155 pixels, focal
length 2384.3233 px, principal point (963.3717, 568.8665), and radial `k1`
−0.408783. The focal rose 13.60% from the image-width heuristic of 2098.8
px. These parameters are fitted by SfM and have not been checked against
independent camera geometry.

The numeric source capture order must come from the sealed
`source_frame_by_photo` mapping; processed `frame_####` numbering follows
lexicographic source names. In that numeric order, adjacent camera-center
steps have median 0.302818 model units, but `bunny_13_rgb.png` to
`bunny_14_rgb.png` measures only 0.000738. This near-coincident pair is a
specific camera-geometry review item. It is not evidence that the whole orbit
or object surface is correct.

## Logs and limits

The HIGH SIFT feature setting is confirmed in the feature log. Exhaustive
putative matching covered 2,628 pairs; the geometric graph has 73 nodes,
one connected component, and node degrees 4–8. The SfM log reports 73/73
calibrated cameras, 6,526 points, final bundle-adjustment RMSE 0.365235, and
`Used motion prior: 0`. A scan of all five logs found no warning, error,
failed, or invalid line. The final log contains successful resection statuses.

Stage times were 0.543, 23.085, 2.733, 56.998, and 28.710 seconds
(112.069 seconds total); each stayed below its finite stage cap. Maximum
recorded process-tree RSS was 239,008 KiB against 4,194,304 KiB; maximum
stage log was 132,133 bytes against 16,777,216 bytes. The final output tree
uses 136,752,102 bytes against a 1,073,741,824-byte cap. Both volumes still
pass the runner's 11 GiB free-space reserve check.

Decision: preserve the sparse result and require a separate, hash-bound
camera-quality audit of trajectory, intrinsics, sparse structure, and
object-centered support before any OpenMVS continuation. Registration count
and low reprojection error alone do not satisfy that gate.
