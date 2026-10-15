# Bounded YCB expansion

`scripts/object_dataset/prepare_ycb_more.py` adds `006_mustard_bottle` and
`035_power_drill` without changing the cracker-box preparer. It selects 60
original Berkeley NP3 RGB JPEGs per object (0°–354° in 6° steps), plus a
separate Google scanner mesh (mustard 16k, drill 64k). Depth, supplied masks
and poses, and Google geometry do not enter the reconstruction-photo set.
One elevation cannot establish underside coverage. The Google mesh is a
separate-sensor shape oracle, not certified physical ground truth, and this
step does not register it to the NP3 camera frame.

The metadata-only `--preflight --object all` makes four HTTPS HEAD requests.
On 2026-09-27 all returned HTTP 200 with the expected sizes and ETags:
mustard Berkeley 657,272,400 and Google 10,703,139 bytes; drill Berkeley
631,773,983 and Google 12,777,450 bytes. No object body was downloaded on
the Mac. Content-Length and ETag are drift checks, **not independent SHA-256
authentication**.

Explicit `--fetch --object <id>` downloads one object at a time into a fresh,
hidden stage on the storage filesystem and publishes the final directory
only by a same-filesystem atomic rename after validation. The default VPS
destination is canonical
`/mnt/akademie_storage/crispmesh-data/ycb-expansion-001`; `/mnt/storage` is
a symlink to `/mnt/akademie_storage` and the helper rejects symlinked
destination ancestors. Large archives and extraction stay on storage;
`/mnt/volume1` (about 6.5 GiB free) is not used for them. Bounds include a
10 GiB storage reserve, exact compressed byte limits, 900 seconds per
object, ≤2,500 tar members, ≤32 MiB per selected member, and ≤200 MB total
selected/converted extraction. Only allowlisted regular members are used;
duplicates, traversal, links, truncated bodies and changed headers fail.
All 60 selected JPEGs are decoded as RGB 1280×1024 and rehashed before
publication. The Google ASCII triangle mesh is converted to binary float64
PLY with vertices and triangle indices preserved, using the existing
reviewed helper. Failed transactions leave no final object directory.

The manifest records SHA-256 for each downloaded archive, photo, original
scanner PLY and converted PLY. Without independently obtained
`--berkeley-sha256` and `--google-sha256`, downloaded archive digests are
*observed* and the manifest says
`observed_sha256_only_unverified_upstream_identity`. Supplying both external
pins enforces them; ETag is never treated as SHA-256. Review mesh completeness,
cross-sensor frame alignment and rights before benchmark use or redistribution.

The VPS needs Python 3.11+, Pillow, and precisely this new module plus its
existing local dependency `scripts/object_dataset/prepare_ycb.py` in a normal
package layout. Run one object at a time:

```sh
python3 -m scripts.object_dataset.prepare_ycb_more --preflight --object all
python3 -m scripts.object_dataset.prepare_ycb_more --fetch --object 006_mustard_bottle
python3 -m scripts.object_dataset.prepare_ycb_more --fetch --object 035_power_drill
```

The four HEAD checks and 59 local object-dataset tests passed. At the initial
handoff the two GET acquisitions had **not** been run. Existing cracker-box
code and assets remain unchanged. The local framework Python required
`SSL_CERT_FILE` pointing to its installed `certifi` bundle for HEAD; TLS
verification was not disabled. The VPS should use its system CA bundle.

## VPS first-attempt diagnostic

The first explicit VPS fetch attempts for both objects stopped before any
archive body was retained with `OSError: [Errno 12] Cannot allocate memory`
at a combined HTTPS-response/file-open statement. No final object directory
was published. Bounded follow-up probes on the VPS succeeded: a 16-byte
Range GET, a normal GET with only 16 application bytes read, and a 16-byte
write to an exclusive file at the same two-level storage path while holding
the response open. Thus the failure was not reproducible with tiny probes;
its exact failing operation remains unproven. The helper now separates
`open_https_get`, `open_storage_archive`, `read_https_body`, and
`write_storage_archive` phases and preserves a ≤4 KiB failure report outside
the unpublished stage, including cause errno, stage archive byte counts and
script hashes. No TLS checks were disabled and no full-body retry was made
during this diagnostic. A fresh serial VPS retry was then approved.

## Actual acquisition status

The fresh serial mustard retry **completed** on the VPS with 60 fully
decoded NP3 RGB JPEGs and a distinct Google 16k mesh. The copied
[mustard manifest](../build-opencv/ycb-vps-acquisition-001/mustard-manifest.json)
has SHA-256
`c48998dffc7798e7d21f275383b1c67dc2e5e9313a8bfe699f72303f61583b52`;
the tracked [dataset anchor](../tests/datasets/ycb_mustard_bottle.json) records
the archive, selected-image and scan provenance. Berkeley archive observed
SHA-256 is
`5d9b1837eb58b0760463e99021a53fe6e82d5cd2457141945445ed6df06ff3f7`;
Google archive observed SHA-256 is
`992621f78a4fa6267da71418e530cd547bd066775da0242e578ddfb561d7cb5d`.
The geometry-preserving converted scan has 8,194 vertices and 16,384 faces,
SHA-256
`b82268e4ea6c9ad9dbcf8e6fd35b4a3b3631af3ecfcd413722df07f87cf601b4`.
The manifest explicitly states `observed_sha256_only_unverified_upstream_identity`:
these are repeatable observations of downloaded bytes, not publisher-signed
or independently supplied archive pins. No reconstruction, cross-sensor
alignment or mesh-quality benchmark has yet run on mustard.

The subsequent serial drill retry also **completed** with 60 fully decoded
NP3 JPEGs and a distinct Google 64k mesh. Its copied
[drill manifest](../build-opencv/ycb-vps-acquisition-001/drill-manifest.json)
has SHA-256
`aeef80b879676092047d54089905dfdc29859a56b1ad872d17f800d18cc81594`;
the tracked [drill anchor](../tests/datasets/ycb_power_drill.json) records
selected-image and scan hashes. Berkeley archive observed SHA-256 is
`be6fb9bdc61806f631cf81bb56d0e018325ecc669bcbc4b7b82904137fd912e2`;
Google archive observed SHA-256 is
`2f941e60f512db3392464b52c737b8c2980e522ab2458890da3ffce2e99aaf03`.
The converted scan has 32,770 vertices and 65,536 faces, SHA-256
`26ac2031ad9d4dd8300eb0794fa1b91a9c961fb21bd20871755144431604b360`.
As for mustard, the archive hashes are observed on the VPS, not independently
authenticated upstream pins. Neither object has yet undergone image-only
reconstruction, scanner-frame registration, or a quality benchmark. Successful
acquisition establishes usable input inventory, not pipeline accuracy.
