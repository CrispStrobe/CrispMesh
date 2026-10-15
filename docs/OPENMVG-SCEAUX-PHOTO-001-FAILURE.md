# OpenMVG Sceaux photo control `-001`: parser-gated stop

The sealed 11-photo, evaluation-only run at
`/Volumes/backups/code/crispmesh-data/openmvg-sceaux-photo-sfm-001` stopped at
`ComputeFeatures` before putative matching, geometric filtering, or SfM. The
listing stage completed and its log reports **11 usable images and one
intrinsic**. The failed feature command included `-n 2`; the actual built
binary rejected it with `Unrecognized option -n` and exit code 1. The
feature log is 716 bytes, far below the 16 MiB log cap, so the receipt's
generic “stage returned 1 or log exceeded cap” means **exit 1** here, not
resource exhaustion. No feature artifact or later-stage output was created.

Read-only seals from the preserved `-001` tree:

| Artifact | SHA-256 |
| --- | --- |
| `receipt.json` | `555f394b32f565bf0232e16899193c638871d4f614d81b10969a3168ac6f230f` |
| `logs/01-listing.log` | `f4c22fa1badd2dc39a5086c1e4495f9b6736a3e2d55afc0205eaf28e5bce7687` |
| `logs/02-features.log` | `d8725e6169614bd05cf5f338930cc10abfa5aab0dcbcc42c3089e53ebac1edf3` |
| `matches/sfm_data.json` | `def7ca0d3cec6996625e20a358aee66a6589d6e759c29f125a5cd13ae63dc110` |

The receipt records all 11 staged photo hashes, `status: stopped`, listing
`status: completed`, and feature `status: stopped`; it was not rewritten.
Output tree size is 35,528,247 bytes. At audit time external free was
14,408,904,704 bytes and internal free 22,433,988,608 bytes, both above the
11 GiB operational margin.

## Five-binary CLI contract check

I invoked each pinned binary with **no arguments only** and compared its
printed usage with the v2.1 `cmd.add(make_option(...))` source. All five
returned exit 1 for missing required parameters; that is expected and does
not run photos. `SfMInit_ImageListing` accepts `-i/-o/-f/-c/-g` as used;
`ComputeMatches` accepts `-i/-o/-r`; `GeometricFilter` accepts
`-i/-m/-o/-g`; and `SfM` accepts `-i/-m/-M/-o/-s/-f`. `ComputeFeatures`
accepts `-i/-o/-m/-p` but its usage does **not** list `-n` in this build.
Although pinned `main_ComputeFeatures.cpp` contains a `-n` registration,
both its thread variable and registration are enclosed in
`#ifdef OPENMVG_USE_OPENMP`; the evaluated fork was configured with
`OpenMVG_USE_OPENMP=OFF`. The parser error is therefore a command-contract
bug, not an SfM quality result. The corrected fresh-run contract should
remove only feature `-n 2`, retaining the supervisor's process-level thread
environment limits. The remaining stage flags are parser-valid, but stages
after listing were **not executed** in `-001` and are not validated by this
failure. `SfM -M matches.e.bin` is a basename paired with `-m <match_dir>`;
pinned `main_SfM.cpp` joins those paths before loading.

This run provides no camera-recovery comparison. It remains separate from
any future `-002` output; do not resume, clean, or reinterpret `-001` as a
completed reconstruction.

After the fresh `-002` wrapper was committed as `ec7fa2c`, I compared its
five generated command lists with the original contract: the **only** argv
change is deletion of feature `-n 2`. A no-argument audit of each sealed
binary confirms every remaining planned option is advertised by that built
CLI; the preserved `-001` file and receipt seals also still pass. This is a
parser-contract comparison only, not a judgment on the `-002` run's outputs.
