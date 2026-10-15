# Repository backup and visibility

On 2026-09-27 the repository was made public at the owner's explicit request.
The project-owned code is now licensed under AGPL-3.0-only; third-party
code, assets and datasets retain their own terms. Repository publication
includes its Git history. Downloaded datasets, build outputs and credentials
remain excluded by the existing ignore rules.

The initial private-backup record below is historical.

Created 2026-09-26: https://github.com/CrispStrobe/CrispMesh (private).

The initial `main` commit is `3e6466ea90b09524790d75bcdadc867466179ba7`.
It contains 293 source, test, configuration, small application-asset, and
documentation files (2,249,912 uncompressed file bytes). GitHub privacy and
the remote branch commit were checked after creation/push. No application
license was selected or changed as part of this backup.

This is **not a complete data/artifact backup**. The following remain local:

- `.local-tools/`: downloaded datasets, isolated oracle environments, caches.
- `build/` and `build-*/`: binaries, generated data, detailed run artifacts.
- `node_modules/`, `target/`, generated frontend output.
- Credentials and local environment files.

Dataset acquisition scripts, metadata, frozen protocols, and written result
summaries are versioned. Links from documentation into ignored build folders
work only in the original workspace or after recreating those artifacts.
Raw benchmark outputs and photos must be separately archived on the VPS or
another approved private storage target before this Mac's copies are removed.
Nothing was deleted locally.

A filename/size/symlink check and common credential-pattern scan of the staged
files found no flagged items. This is a precaution, not a full security audit.
Backup commits use `[skip ci]` to avoid unexpectedly launching the existing
multi-platform build workflow. The previous local test results remain recorded
in STATUS.md; no new cross-platform CI success is claimed.
