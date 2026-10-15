# Sceaux OpenMVG photo SfM control: fresh attempt 002

Status: **read-only preflight passed; no 002 photo run authorized**. This is
evaluation-only software, with no shipping-license conclusion. The first
attempt is retained at
`/Volumes/backups/code/crispmesh-data/openmvg-sceaux-photo-sfm-001`.
It stopped at ComputeFeatures before any feature extraction because that
binary rejected `-n 2`; image listing had completed. Its receipt SHA-256 is
`555f394b32f565bf0232e16899193c638871d4f614d81b10969a3168ac6f230f`.
The listing and failed-feature logs have SHA-256
`f4c22fa1badd2dc39a5086c1e4495f9b6736a3e2d55afc0205eaf28e5bce7687`
and `d8725e6169614bd05cf5f338930cc10abfa5aab0dcbcc42c3089e53ebac1edf3`;
the saved listing `matches/sfm_data.json` hash is
`def7ca0d3cec6996625e20a358aee66a6589d6e759c29f125a5cd13ae63dc110`.
The new supervisor verifies these files and the complete failed inventory
without changing or reusing the first output.

## Cause and parser audit

The earlier source review found `cmd.add(make_option('n', ...))` in
`main_ComputeFeatures.cpp` but missed its enclosing
`#ifdef OPENMVG_USE_OPENMP`. The sealed build has
`OpenMVG_USE_OPENMP:BOOL=OFF`, so `-n` is absent from the compiled feature
parser. The five exact pinned binaries were each invoked with no arguments
for read-only usage output. Their planned option sets for attempt 002 are:

| Stage | Planned options confirmed in compiled usage |
| --- | --- |
| Image listing | `-i -o -f -c -g` |
| Features | `-i -o -m -p` |
| Putative matching | `-i -o -r` |
| Geometric filtering | `-i -m -o -g` |
| Incremental SfM | `-i -m -M -o -s -f` |

The supervisor repeats this no-argument check on every read-only preflight
and rejects a planned flag missing from its corresponding compiled usage.
The source parser also explicitly accepts `ADJUST_ALL` for SfM `-f`; its
`-M` selects `matches.e.bin` in the match directory. The option `-n` exists
on the separate ComputeMatches binary as a *nearest matching method* flag;
it is not a thread option there. The new feature command omits `-n`.
The source and sample requirements otherwise remain those frozen in the
[first plan](OPENMVG-PHOTO-ONLY-CONTROL-PLAN.md).

## One-shot output and limits

Attempt 002 uses only the same 11 sealed Sceaux JPEGs from the local sample
manifest (SHA-256
`1ee34cff7e6d75fca9f56de4aa5644b45a382030af54a474824ada6af77316a3`),
copied into a **new** external output
`/Volumes/backups/code/crispmesh-data/openmvg-sceaux-photo-sfm-002`.
No sample scene, reference poses, calibration, mesh or depth is an input.
Both source and staged photos are hashed. The exact five-stage contract is:

```text
SfMInit_ImageListing -i IMAGES -o MATCHES -f 3398.4 -c 2 -g 1
ComputeFeatures -i MATCHES/sfm_data.json -o MATCHES -m SIFT_ANATOMY -p NORMAL
ComputeMatches -i MATCHES/sfm_data.json -o MATCHES/matches.putative.bin -r 0.8
GeometricFilter -i MATCHES/sfm_data.json -m MATCHES/matches.putative.bin -o MATCHES/matches.e.bin -g e
SfM -i MATCHES/sfm_data.json -m MATCHES -M matches.e.bin -o SPARSE -s INCREMENTAL -f ADJUST_ALL
```

All executables are the hash-verified, evaluation-only OpenMVG v2.1 LiGT-off
fork targets. Focal 3398.4 px is the 1.2×2832 photo-width heuristic, not a
reference-camera value. The run has no prior pose, forced pair or injected
matches. `OPENMVG_USE_OPENMP=OFF` makes feature extraction single threaded;
the environment caps other math libraries at two threads. The prior
per-process CPU, 4 GiB process-tree RSS, 16 MiB per-log, 1 GiB total output,
120/420/420/420/420-second stage and 1800-second total wall limits remain.
Both disks must stay at least 11 GiB free; external preflight additionally
reserves the full 1 GiB allocation. The exact output must not already exist.
Failed output is preserved with stage receipts; there is no retry or cleanup.

Read-only preflight:

```text
python -m scripts.classical_backend.openmvg_sceaux_photo_control_v2
```

The one-shot live path `--run-sceaux` awaits separate review. A successful
native exit still needs all 11 poses and nonzero tracks/residuals from the
pinned SfM HTML report, then neutral geometry review of the frozen binary
model. Mustard 48 remains a distinct subsequent decision.
