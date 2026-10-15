"""Sealed preflight and optional paired mustard feature-mask SfM ablation.

The existing classical_backend.run producer performs each arm. This wrapper
checks reviewed train-only inputs and imposes one wall/output budget on both.
Preflight is read-only. Live execution requires the explicit --run switch.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time

from PIL import Image

from scripts.classical_backend import mustard_stage


ROOT = Path(__file__).resolve().parents[2]
DATA = Path('/Volumes/backups/code/crispmesh-data')
STAGE = DATA / 'mustard-sfm-train-001'
V2 = DATA / 'mustard-photo-refine-002'
QA = ROOT / 'tests/datasets/mustard_photo_refine_v2_review.json'
RUNNER = ROOT / 'scripts/classical_backend/run.py'
PYTHON = ROOT / '.local-tools/colmap-sparse/venv/bin/python'
PYCOLMAP_CORE = ROOT / '.local-tools/colmap-sparse/venv/lib/python3.11/site-packages/pycolmap/_core.cpython-311-darwin.so'
BINS = ROOT / '.local-tools/classical-backend/bin'
OUTPUTS = (DATA / 'mustard-feature-mask-pair-001-sam',
           DATA / 'mustard-feature-mask-pair-001-v2')
RECEIPT = DATA / 'mustard-feature-mask-pair-001-receipt.json'
STAGE_SHA = 'bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0'
V2_SHA = '0c47218e91f11f2499d618589fa91f6e4879a95ef0c89fb1681af57f677fb125'
QA_SHA = '65a4bd1a3cd33a492d75edf8e2d6f7e8675cf5a87017a5d4fd1ab926d2484105'
RUNNER_SHA = 'c65f73396e52a22e56d7edc358461c1f25a58f163d390a372e3b4c20e9b7441b'
PYCOLMAP_SHA = '5e306a76bfb1a3a02139ea9d336770091a77376bb870a383ed3882b7250008a8'
PYTHON_SHA = '09e1a00906ae3a7cf190155f47d0c23fc0b40d207997a9c44c7995ba9db896c2'
NAMES_SHA = 'a81d1647109db27991c39c5fe1a9ae308c04081f8ada49d8b0f5ab1c15245544'
MIN_FREE = 10 * 1024**3
TOTAL_LIMIT = 1 * 1024**3
ARM_GIB = 0.45
WALL_SECONDS = 600
ARM_WALL_MINUTES = 4.5
BIN_HASHES = {
    'InterfaceCOLMAP': '34d7ee529fe232dc26e23db53097ad4567aa502e9e42f901d1cb1baf613a4d98',
    'DensifyPointCloud': '8ea970b0349270754d23f31a11dea87417c06d1001fa1ef437ee69b6146a1ce0',
    'ReconstructMesh': '05f0eae93628ce50fa481db2879e9f62d1636b68f06422f76fd8b5d8d4773b0e',
    'RefineMesh': '0c94f22f43e733f7d8fd95fa1628b599135fe0ae7864bcc9089b7cf5c2c9cf81',
    'TextureMesh': '02887713214539f59286f8a2af430a48f6e237a5a98c8a83d050489bc426837f',
}


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def sealed(path: Path, expected: str) -> dict:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 1024**2 or sha(path) != expected:
        raise ValueError(f'sealed JSON changed: {path}')
    return json.loads(path.read_text())


def command(output: Path, masks: Path, minutes: float) -> list[str]:
    return [str(PYTHON), '-m', 'scripts.classical_backend.run',
            '--images', str(STAGE / 'images'), '--image-list', str(STAGE / 'train-names.txt'),
            '--pose-mask-dir', str(masks), '--output', str(output),
            '--python', str(PYTHON), '--binary-dir', str(BINS),
            '--max-views', '48', '--stop-after-sfm',
            '--camera-model', 'SIMPLE_RADIAL', '--matching', 'exhaustive',
            '--sift-max-features', '1800', '--sift-max-image-size', '1200',
            '--seed', '20260927', '--sfm-max-models', '5', '--sfm-min-model-size', '10',
            '--sfm-intrinsics-policy', 'fixed-initial', '--min-registered-fraction', '0.7',
            '--max-threads', '2', '--max-gib', str(ARM_GIB), '--max-rss-gib', '4',
            '--max-log-mib', '16', '--timeout-minutes', f'{minutes:.4f}']


def preflight(allow_existing_outputs: frozenset[Path] = frozenset(), allow_receipt: bool = False) -> dict:
    if any((path.exists() or path.is_symlink()) and path not in allow_existing_outputs for path in OUTPUTS):
        raise FileExistsError('paired outputs must both be fresh')
    if (RECEIPT.exists() or RECEIPT.is_symlink()) and not allow_receipt:
        raise FileExistsError('paired receipt must be fresh')
    if any(path.is_symlink() or not path.is_dir() for path in allow_existing_outputs):
        raise ValueError('previous arm output changed type')
    if DATA.is_symlink() or STAGE.is_symlink() or V2.is_symlink():
        raise ValueError('source root must be a real directory')
    if STAGE.stat().st_dev == ROOT.stat().st_dev:
        raise ValueError('staged inputs are not on a distinct external device')
    if shutil.disk_usage(ROOT).free < MIN_FREE or shutil.disk_usage(DATA).free < MIN_FREE + TOTAL_LIMIT:
        raise RuntimeError('10 GiB reserve plus paired output allowance unavailable')
    stage = sealed(STAGE / 'stage-report.json', STAGE_SHA)
    v2 = sealed(V2 / 'report.json', V2_SHA)
    qa = sealed(QA, QA_SHA)
    names_path = STAGE / 'train-names.txt'
    if names_path.is_symlink() or sha(names_path) != NAMES_SHA:
        raise ValueError('training image list changed')
    names = names_path.read_text().splitlines()
    if (names != list(mustard_stage.TRAIN_NAMES) or
            stage.get('status') != 'complete' or
            set(stage.get('train_photo_sha256', {})) != set(names) or
            set(stage.get('cleaned_masks', {})) != set(names) or
            v2.get('status') != 'generated_unreviewed' or
            [row.get('name') for row in v2.get('images', [])] != names or
            v2.get('stage_report_sha256') != STAGE_SHA or
            qa.get('candidate_status') != 'approved_coarse_feature_ablation_only' or
            qa.get('candidate_report_sha256') != V2_SHA or
            qa.get('reviewed_training_views') != 48):
        raise ValueError('training and reviewed candidate lineage mismatch')
    if (v2.get('accepted_sam_inventory_sha256') != qa['source_sam_inventory_sha256'] or
            v2.get('accepted_sam_visual_qa_sha256') != qa['source_sam_qa_sha256'] or
            stage.get('inventory_sha256') != qa['source_sam_inventory_sha256'] or
            stage.get('qa_sha256') != qa['source_sam_qa_sha256'] or
            qa.get('source_stage_report_sha256') != STAGE_SHA):
        raise ValueError('SAM control lineage mismatch')
    photos = STAGE / 'images'
    old_masks = STAGE / 'masks'
    new_masks = V2 / 'masks'
    for directory, expected in ((photos, set(names)), (old_masks, {n + '.png' for n in names}),
                                (new_masks, {n + '.png' for n in names})):
        mustard_stage.exact_files(directory, expected)
    for row in v2['images']:
        name = row['name']
        photo = photos / name
        control = old_masks / (name + '.png')
        candidate = new_masks / (name + '.png')
        if (sha(photo) != stage['train_photo_sha256'][name] or
                sha(control) != stage['cleaned_masks'][name]['sha256'] or
                sha(candidate) != row['candidate_sha256'] or
                row['photo_sha256'] != stage['train_photo_sha256'][name] or
                row['prior_sha256'] != stage['cleaned_masks'][name]['sha256']):
            raise ValueError(f'input byte hash mismatch: {name}')
        mustard_stage.mask_pixels(control, photo, (1280, 1024))
        mustard_stage.mask_pixels(candidate, photo, (1280, 1024))
        with Image.open(control) as base, Image.open(candidate) as trimmed:
            import numpy as np
            a = np.asarray(base)
            b = np.asarray(trimmed)
            if (np.any((b > 0) & (a == 0)) or not np.array_equal(a[:435], b[:435]) or
                    int(np.count_nonzero(a) - np.count_nonzero(b)) != row['trimmed_pixels']):
                raise ValueError(f'v2 preservation constraint mismatch: {name}')
    if (sha(RUNNER) != RUNNER_SHA or not PYTHON.is_file() or
            PYTHON.resolve() != Path('/Library/Frameworks/Python.framework/Versions/3.11/bin/python3.11') or
            sha(PYTHON) != PYTHON_SHA):
        raise ValueError('runner/interpreter changed')
    if sha(PYCOLMAP_CORE) != PYCOLMAP_SHA:
        raise ValueError('PyCOLMAP binary changed')
    for name, expected in BIN_HASHES.items():
        if sha(BINS / name) != expected:
            raise ValueError(f'OpenMVS binary changed: {name}')
    return {'schema': 'mustard_mask_pair_preflight_v1', 'status': 'ready_for_review',
            'training_views': len(names), 'stage_report_sha256': STAGE_SHA,
            'v2_report_sha256': V2_SHA, 'v2_qa_sha256': QA_SHA,
            'wrapper_sha256': sha(Path(__file__)),
            'runner_sha256': RUNNER_SHA, 'pycolmap_binary_sha256': PYCOLMAP_SHA,
            'wall_seconds_combined': WALL_SECONDS, 'total_output_cap_bytes': TOTAL_LIMIT,
            'per_arm_output_cap_bytes': int(ARM_GIB * 1024**3),
            'commands': [command(OUTPUTS[0], old_masks, ARM_WALL_MINUTES),
                         command(OUTPUTS[1], new_masks, ARM_WALL_MINUTES)],
            'internal_free_bytes': shutil.disk_usage(ROOT).free,
            'external_free_bytes': shutil.disk_usage(DATA).free}


def source_hashes() -> dict[str, str]:
    names = mustard_stage.TRAIN_NAMES
    paths = [STAGE / 'stage-report.json', STAGE / 'train-names.txt',
             V2 / 'report.json', QA, RUNNER, Path(__file__), PYTHON, PYCOLMAP_CORE]
    paths += [BINS / name for name in sorted(BIN_HASHES)]
    paths += [STAGE / 'images' / n for n in names]
    paths += [STAGE / 'masks' / (n + '.png') for n in names]
    paths += [V2 / 'masks' / (n + '.png') for n in names]
    return {str(path): sha(path) for path in paths}


def output_hashes(output: Path) -> dict[str, str]:
    if not output.is_dir() or output.is_symlink():
        return {}
    files = sorted(path for path in output.rglob('*') if path.is_file())
    if any(path.is_symlink() for path in output.rglob('*')):
        raise ValueError('run output contains a symlink')
    return {str(path.relative_to(output)): sha(path) for path in files}


def save_receipt(report: dict) -> None:
    payload = (json.dumps(report, indent=2, sort_keys=True) + '\n').encode()
    if len(payload) > 1024**2:
        raise RuntimeError('paired receipt exceeds 1 MiB')
    temp = RECEIPT.with_suffix('.json.tmp')
    if temp.is_symlink() or temp.exists():
        raise FileExistsError(temp)
    with temp.open('xb') as stream:
        stream.write(payload)
    os.replace(temp, RECEIPT)


def completed(report: dict) -> bool:
    return (not report.get('failure') and not report.get('final_audit_failure') and
            len(report.get('arms', [])) == 2 and
            all(row.get('exit_code') == 0 and row.get('producer_status') == 'sparse_complete'
                for row in report['arms']) and
            report.get('sources_unchanged') is True and
            report.get('internal_free_bytes_after', 0) >= MIN_FREE and
            report.get('external_free_bytes_after', 0) >= MIN_FREE and
            report.get('total_output_bytes', TOTAL_LIMIT) < TOTAL_LIMIT)


def run_pair() -> dict:
    start_preflight = preflight()
    report = {'schema': 'mustard_mask_pair_receipt_v1', 'status': 'running',
              'preflight': start_preflight, 'source_hashes_before': source_hashes(),
              'arms': []}
    save_receipt(report)
    begin = time.monotonic()
    try:
        for output, masks in zip(OUTPUTS, (STAGE / 'masks', V2 / 'masks')):
            preflight(frozenset(Path(row['output']) for row in report['arms']), allow_receipt=True)
            remaining = WALL_SECONDS - (time.monotonic() - begin)
            if remaining < 60:
                report['failure'] = 'combined wall time leaves less than 60 seconds for next arm'
                break
            if min(shutil.disk_usage(ROOT).free, shutil.disk_usage(DATA).free) < MIN_FREE:
                report['failure'] = '10 GiB floor reached before next arm'
                break
            cmd = command(output, masks, ARM_WALL_MINUTES)
            env = os.environ.copy()
            env.update(TMPDIR=str(DATA), TMP=str(DATA), OPENBLAS_NUM_THREADS='2', OMP_NUM_THREADS='2')
            process = subprocess.Popen(cmd, cwd=ROOT, env=env, start_new_session=True,
                                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            try:
                stdout, _ = process.communicate(timeout=remaining)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                stdout, _ = process.communicate(timeout=5)
            producer_path = output / 'result.json'
            producer = json.loads(producer_path.read_text()) if producer_path.is_file() else {}
            options_path = output / 'pycolmap-options.json'
            options = json.loads(options_path.read_text()) if options_path.is_file() else None
            report['arms'].append({'output': str(output), 'command': cmd,
                                   'exit_code': process.returncode, 'stdout_tail': stdout[-2048:],
                                   'producer_status': producer.get('status'),
                                   'producer_result_sha256': sha(producer_path) if producer_path.is_file() else None,
                                   'effective_options': options,
                                   'output_hashes': output_hashes(output)})
            save_receipt(report)
    except Exception as error:
        report['failure'] = f'{type(error).__name__}: {error}'
    try:
        report['source_hashes_after'] = source_hashes()
        report['sources_unchanged'] = report['source_hashes_after'] == report['source_hashes_before']
        report['internal_free_bytes_after'] = shutil.disk_usage(ROOT).free
        report['external_free_bytes_after'] = shutil.disk_usage(DATA).free
        report['total_output_bytes'] = sum(sum((Path(row['output']) / name).stat().st_size
                                            for name in row['output_hashes']) for row in report['arms'])
    except Exception as error:
        report['final_audit_failure'] = f'{type(error).__name__}: {error}'
    report['status'] = 'completed' if completed(report) else 'partial_or_failed'
    save_receipt(report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', action='store_true', help='launch only after root approval')
    args = parser.parse_args()
    print(json.dumps(run_pair() if args.run else preflight(), indent=2))


if __name__ == '__main__':
    main()
