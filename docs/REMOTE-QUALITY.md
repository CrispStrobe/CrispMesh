# Remote quality experiment staging

This is a staging path for a bounded **CPU** quality experiment. No remote job has been submitted and no result in this document is a benchmark. The local Mac has under 30 GiB free; preserve at least 10 GiB and use it only for small validation. On the Linux VPS, keep large immutable inputs under `/mnt/storage` and use `/mnt/volume1` for fast scratch. The runner refuses to run on macOS or without both mounts.

## Experiment contract

Select one fixed input population and one explicit command. For the current quality work, record source image IDs, mask/pose source, algorithm settings, executable/source hashes, elapsed time, peak RSS, fixed-population coverage, matched accuracy and missing-inclusive error. Mark a nonzero exit, missing metrics, or changed input hash as **failed**. The synthetic fixture and existing Middlebury scenes are development diagnostics; they do not prove real-object accuracy. The `tree_dense` experiment can use this runner once its CLI and output schema are frozen, with the same input archive and scoring for baseline and candidate.

The package builder accepts only a JSON array of explicit repository-relative paths; for example:

```json
[
  "scripts/remote_quality/run_cpu.py",
  "scripts/quality_ablation/score.py"
]
```

Run `python3 scripts/remote_quality/package.py --selection selection.json --output /path/outside/crispmesh-quality.zip`. The output parent must exist. The builder rejects symlinks, traversal, build/data directories, credential-like names and common token patterns in contents; it caps files at 256 and compressed/uncompressed bytes at 250 MiB, demands a fresh output and a 10 GiB free-space reserve. It deliberately does not collect the whole workspace or any local datasets. For photo inputs, create a separate explicit allowlist and verify their ownership, size and consent before transport. Never copy `../kaggle_usage.md`, local `.env`, tokens or Kaggle credential datasets into this repository or package.

On the VPS, transfer the archive by the user's normal approved channel to an existing path under `/mnt/storage`. No transfer command is run by this tooling. A runner config contains only nonsecret paths and an explicit command array, e.g.:

```json
{
  "input_archive": "/mnt/storage/quality-input/crispmesh-quality.zip",
  "work_dir": "/mnt/volume1/quality-run-r01",
  "output_dir": "/mnt/storage/quality-results/r01",
  "command": ["/usr/bin/python3", "scripts/your_experiment.py", "--output", "/mnt/storage/quality-results/r01/metrics.json"],
  "result_file": "metrics.json",
  "result_fields": {"status": "string", "metrics": "object"},
  "threads": 2,
  "timeout_seconds": 1800,
  "scratch_cap_bytes": 8589934592
}
```

`python3 scripts/remote_quality/run_cpu.py run-config.json` is preflight only. Add `--execute` only after checking the exact paths, command and input manifest. Work and output directories must be fresh. Execution has a 2 hour maximum timeout, 1–16 threads, an 8 GiB aggregate scratch target, a 64 MiB per-log cap and a 2 GiB free-space reserve on each mount. It checks disk use once per second and kills the process group when a limit is breached. It writes `stdout.log`, `stderr.log`, and `status.json`; `execution_succeeded_with_artifact` means the command exited zero and produced a small JSON object with the declared fields and types, with `quality_acceptance: not_evaluated`. Set `result_fields` to the actual frozen experiment schema before execution. The experiment's metrics and acceptance gate still need independent validation. No packages are installed automatically. One-second monitoring can overshoot a quota briefly; use an external filesystem quota if the command can write faster than the reserve permits.

## Kaggle option, staged only

Kaggle is a later execution option if a same-account, private input dataset already contains the bounded archive. The local usage guide reports that CPU workers lacked internet despite the internet flag; staging therefore requests a GPU worker for the clone and authenticated downloads even though this algorithm is CPU-based. Recheck the current worker behavior and quota before any launch. A Kaggle push runs immediately, so `prepare_kaggle.py` only creates a reviewable push directory. It never calls Kaggle or changes an account.

Example staging config (replace all identifiers with real, verified values):

```json
{
  "account": "example-account",
  "slug": "crispmesh-quality-r01",
  "run_id": "quality-r01-unique",
  "input_dataset": "example-account/crispmesh-quality-input",
  "archive_name": "crispmesh-quality.zip",
  "command": ["/usr/bin/python3", "scripts/your_experiment.py"],
  "timeout_seconds": 1800,
  "hf_downloads": []
}
```

Run `python3 scripts/remote_quality/prepare_kaggle.py kaggle-config.json --output /path/outside/kaggle-stage-r01`. The generated metadata is private, names the account and its own input dataset, and uses string booleans as Kaggle expects. The generated single script sparsely clones CrispASR under `/kaggle/temp`, checks its harness against the locally reviewed hash, calls `kh.init_progress()` and `kh.resolve_hf_token()`, and wraps longer steps with `kh.build_heartbeat()`. A snapshot of the local harness is embedded in that script as a fallback because script kernels may not expose sibling files at runtime. Failed import or missing archive stops the run. HF artifact downloads are disabled in the stager until each artifact has an explicit repository type, immutable revision, expected size and hash plus aggregate disk control. The dormant kernel download path uses authenticated `huggingface_hub.hf_hub_download(..., token=token)`; no token dataset is attached by default. The input dataset must already exist, be private, and mount at either Kaggle path form; staging alone cannot verify those facts.

The generated script keeps source and scratch under `/kaggle/temp`, limits the input archive to 250 MiB, monitors an 8 GiB scratch target, and leaves only progress, a capped log and `result.json` under `/kaggle/working` with a 1 GiB output target. One-second monitoring can briefly overshoot. Its `run_id` must change with every push. Before any future push, verify the active account explicitly, metadata title/slug, datasets, unique run ID, code, quota, and the exact command. Avoid repushing a running kernel: prior sessions may continue. Do not claim a quality result from Kaggle until a completed result, full output metrics and input hashes have been checked.

Local check: `python3 -m unittest discover -s scripts/remote_quality -p 'test_*.py' -v`.
