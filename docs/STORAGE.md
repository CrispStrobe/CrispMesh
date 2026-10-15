# Storage locations

User-confirmed Mac convention (2026-09-27):

- Source checkout and small active build files: internal disk.
- Large reconstruction artifacts: `/Volumes/backups/code/crispmesh-data`.
- AI model weights: `/Volumes/backups/ai` (use dedicated, versioned subdirectories).
- VPS large datasets, models, and outputs: `/mnt/storage`; use `/mnt/volume1`
  only after checking its available capacity.

Compute policy clarified by the user: run inference on the Mac M1 or Kaggle;
reserve the VPS for smaller CPU tasks and dataset storage. Historical VPS SAM
experiments remain evidence, not the target for further inference. Read
`../kaggle_usage.md` before using Kaggle. On the Mac, put inference environments,
weights and caches under `/Volumes/backups/ai` and experiment outputs under
`/Volumes/backups/code/crispmesh-data`, with both-volume reserve checks.

The external Mac volumes `backups` and `BKP_MAC` share one APFS container's
free space; their free-space figures must not be added together.
Keep at least 10 GiB free on the Mac internal disk. Avoid `/tmp` for large
jobs. Verify that the external volume is actually mounted before writing;
an ordinary directory at its mount path is not sufficient.

Completed local reconstruction runs may be relocated with complete file
checksum verification and links at their original paths. Never discard the
internal copy until the destination and the replacement link have been
verified. Such runs become unavailable while the external disk is detached.
Do not move an active run or build directory.

## Verified relocation helper

`python -m scripts.storage.relocate_runs --run NAME` is restricted to five
reviewed completed runs. It refuses existing destinations rather than
overwriting them. Per-run checksums and completion flags live in
`/Volumes/backups/code/crispmesh-data/build-opencv/.relocation-audits/`.
Only `backup_removed: true` together with `link_verified: true` records a
completed relocation. A partial destination is not a completed move; keep its
internal original until the copy has been verified. The external drive must
remain attached throughout a relocation.

The original four runs completed relocation on 2026-09-27: both audit flags
are true for every run, totaling 5,824,950,121 file bytes (5.42 GiB). Their
original paths now resolve through symlinks to the external drive. The verified
internal duplicates were removed; all data remains on the external drive.

The fifth allowlisted run, `classical-bunny-native-masked-005`, completed
relocation the same day. Its 252-file destination inventory matches the audit
after an independent rehash: 805,819,299 logical file bytes. The original path
is a verified symlink to
`/Volumes/backups/code/crispmesh-data/build-opencv/classical-bunny-native-masked-005`;
the internal duplicate was removed only after verification. Its audit SHA-256
is `75ca4d2e003a26a6f5043ef2b1e2588bc148a6207cc965ab34af43c9ff8df2ec`,
with `link_verified: true` and `backup_removed: true`. At the final check,
internal free space was 11,670,577,152 bytes and external free space was
20,441,006,080 bytes. The external copy remains the recoverable data source;
detaching that volume makes the original symlink unavailable.
