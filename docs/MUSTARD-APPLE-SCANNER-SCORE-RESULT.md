# Mustard Apple whole-mesh scanner diagnostic: registration rejected

The separate [frozen scoring proposal](MUSTARD-APPLE-SCANNER-SCORE-PLAN.md)
was applied only through its registration-review gate. The Apple `.preview`
candidate was already sealed before the Google scanner reference was opened.
The full 935-vertex, 1,847-triangle Apple mesh, including its broad support
sheet, was retained unchanged. Input PLY and script hashes matched the plan;
the reference self-control report matched SHA-256
`e0e13b588e0e4bc85981131b8b3989f70b7c3daf08bf1d5cfe28aa0b0bc9d336`.

The unchanged aligner fitted one proper Sim(3) from 1,024 area-weighted
samples per mesh, seed 2026. The fitted scale is `0.2488882276`; its final
symmetric sampled-point RMS objective is `0.0173164` in reference-coordinate
units, about 7.7% of the scanner bounding-box diagonal. The corrected
[transform](/Volumes/backups/code/crispmesh-data/apple-mustard48-scanner-002/apple-to-google-sim3.json)
has SHA-256 `37885f2c61688a2430df3ccc77cdffecc3e5af6a196c9edf04ec4d1eb067d264`.
The [shared-bounds overlay](../.local-tools/apple-mustard48-scanner-002-overlay.png)
has SHA-256 `bc4baf3e171fb449f39c42915e31fb391f0a0ae70c8ffffd18febe007127f582`.

I inspected the overlay: the orange Apple sheet crosses and extends far beyond
the cyan reference bottle in the XY projection; the other two projections
also contain a large non-bottle surface. This is **not a validated whole-mesh
registration**. Under the frozen gate, I did not run the exact-triangle
precision/recall/F1 scorer. Publishing an F1 under this transform would imply
an object correspondence that the visual check rejects. The Apple preview is
structurally valid but remains an object-only quality failure.

One bookkeeping error is preserved explicitly. The first fresh aligner output
under `apple-mustard48-scanner-001` had the incorrect metadata object ID
`005_tomato_soup_can` (SHA-256
`1db2c72637ca89e3193ad311e69e91b0bf8481ed4fabfac9cb88a9e7c2f87cc6`).
It was not scored or used for the overlay. I reran the exact same deterministic
fit on the same sealed PLYs with the correct ID `006_mustard_bottle` in a second
fresh directory; the resulting 4×4 matrix and objective are byte-for-byte
identical. No seed, geometry, crop or alignment setting was changed. Both
artifacts remain recoverable; neither was overwritten or deleted.

This negative result is specific to Apple's whole-mesh `.preview` output on
these 48 RGBs. It does not evaluate a masked Apple input mode, a `.full`
request, or the cross-platform reconstruction backend, and it does not prove
that the separate scanner and photo frames have an independent metric alignment.
