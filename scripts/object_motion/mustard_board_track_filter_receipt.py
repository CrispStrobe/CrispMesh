#!/usr/bin/env python3
"""Bounded counts-only audit of sealed tracks against fixed board poses.

The --preflight path does not read feature coordinates. The supervised --run
path reads them read-only but emits only aggregate counts and diagnostics: no
observations, 3-D points, virtual images, sparse model, or dense export.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

import numpy as np

from scripts.object_motion import mustard_board_track_preflight as seal
from scripts.object_motion.board_frame_candidate_filter import (
    CalibratedFrame, filter_candidate_tracks,
)
from scripts.object_motion.mustard_verified_track_receipt import (
    ImageSource, _child_failure, _safe_failure, sha256, sources_from_reports,
)
from scripts.object_motion.sam_m1_parity import rss_kib
from scripts.object_motion.verified_track_adapter import extract_candidate_tracks


ROOT = Path(__file__).resolve().parents[2]
DATA = Path('/Volumes/backups/code/crispmesh-data')
FILTER_SHA256 = 'dc45b9aef580661f3a38ca030e57af3d17a4db778946822ff24b43f7f82c19c3'
PREFLIGHT_SHA256 = '49129f5009dc221dd78e06221383a264a9cb777ab565297df24457d9afd2862f'
ADAPTER_SHA256 = '0ff9d78b055e8c5b5f806dbe8fb86a9501845154b4f42f0ef4e6f4a9017bfc7a'
MAX_REPORT = 2 * 1024**2
FLOOR = 10 * 1024**3
SUPERVISOR_SECONDS = 120
MAX_STDERR = 16 * 1024
RSS_CAP_KIB = 2 * 1024**2
OUTPUT = DATA / 'mustard-board-track-filter-001.json'


def free_space(extra_bytes=0):
    values = {'internal_free_bytes': shutil.disk_usage(ROOT).free,
              'external_free_bytes': shutil.disk_usage(DATA).free}
    if values['internal_free_bytes'] < FLOOR or values['external_free_bytes'] < FLOOR + extra_bytes:
        raise RuntimeError('both disks must retain at least 10 GiB free')
    return values


def validate_output(path):
    path = Path(path)
    if (path != OUTPUT or path.exists() or path.is_symlink() or
            DATA.is_symlink() or not DATA.is_dir()):
        raise ValueError('exact fresh external board-track-filter-001.json required')
    return path


def check_worker_rss(sample_kib):
    if not isinstance(sample_kib, int) or sample_kib < 0 or sample_kib > RSS_CAP_KIB:
        raise RuntimeError('2 GiB worker RSS cap exceeded or unavailable')
    return sample_kib


def preflight(output):
    output = validate_output(output)
    if (sha256(Path(filter_candidate_tracks.__code__.co_filename)) != FILTER_SHA256 or
            sha256(Path(seal.__file__)) != PREFLIGHT_SHA256 or
            sha256(Path(extract_candidate_tracks.__code__.co_filename)) != ADAPTER_SHA256):
        raise ValueError('frozen filter/preflight/verified-track adapter source changed')
    prepared = seal.preflight()
    free_space(MAX_REPORT)
    return {**prepared, 'output': str(output), 'filter_sha256': FILTER_SHA256,
            'preflight_sha256': PREFLIGHT_SHA256, 'adapter_sha256': ADAPTER_SHA256}


def _statistics(values):
    if not values:
        return {'count': 0, 'median': None, 'p95': None, 'max': None}
    array = np.asarray(values, dtype=float)
    return {'count': len(values), 'median': float(np.median(array)),
            'p95': float(np.quantile(array, .95)), 'max': float(np.max(array))}


def summarize(filtered, source_count):
    """Only aggregate fields leave the worker; never a point or observation."""
    accepted = filtered.accepted
    name_counts = {}
    for track in accepted:
        for obs in track.virtual_observations:
            name_counts[obs.image_name] = name_counts.get(obs.image_name, 0) + 1
    if filtered.input_tracks != 518 or source_count != 48 or (
            len(accepted) + sum(filtered.rejected_by_reason.values()) != filtered.input_tracks):
        raise ValueError('candidate/filter count conservation failed')
    return {'candidate_tracks': filtered.input_tracks,
            'accepted_tracks': len(accepted),
            'sparse_minimum_eight_met': len(accepted) >= 8,
            'rejected_by_reason': dict(sorted(filtered.rejected_by_reason.items())),
            'dropped_unposed_observations': filtered.dropped_unposed_observations,
            'accepted_observations': sum(len(track.virtual_observations) for track in accepted),
            'accepted_posed_name_count': len(name_counts),
            'accepted_observations_by_posed_name': dict(sorted(name_counts.items())),
            'camera_facing_board_sign': filtered.camera_facing_sign,
            'accepted_signed_height_squares': _statistics([track.signed_height_squares for track in accepted]),
            'accepted_max_distorted_reprojection_px': _statistics(
                [track.max_distorted_reprojection_px for track in accepted]),
            'accepted_max_virtual_reprojection_px': _statistics(
                [track.max_virtual_reprojection_px for track in accepted]),
            'accepted_max_pair_parallax_degrees': _statistics(
                [track.parallax_degrees for track in accepted])}


def _worker_receipt(output):
    before = preflight(output)
    pose = seal._json(seal.POSE, seal.POSE_SHA256)
    stage = seal._json(seal.STAGE / 'stage-report.json', seal.STAGE_REPORT_SHA256)
    producer = seal._json(seal.PRODUCER / 'result.json', seal.PRODUCER_RESULT_SHA256)
    sources: tuple[ImageSource, ...] = sources_from_reports(stage, producer, seal.STAGE / 'images')
    k = np.asarray(pose['intrinsics']['K'], dtype=float)
    d = np.asarray(pose['intrinsics']['distortion'], dtype=float)
    source_by_name = {source.name: source for source in sources}
    frames = []
    for view in pose['views']:
        if not view['detected']:
            continue
        source = source_by_name[view['name']]
        frames.append(CalibratedFrame(view['name'], source.width, source.height,
                                      (float(k[0, 0]), float(k[1, 1]),
                                       float(k[0, 2]), float(k[1, 2])),
                                      tuple(map(float, d)),
                                      tuple(tuple(map(float, row)) for row in view['camera_from_board_rotation']),
                                      tuple(map(float, view['camera_from_board_translation_square_units']))))
    candidates = extract_candidate_tracks(seal.PRODUCER / 'database.db', seal.DATABASE_SHA256,
                                          seal.STAGE / 'images', seal.STAGE / 'masks', sources)
    if len(candidates.tracks) != 518:
        raise ValueError('sealed candidate track count changed')
    filtered = filter_candidate_tracks(candidates.tracks, frames,
                                       {source.name for source in sources})
    counts = summarize(filtered, len(sources))
    after = preflight(output)
    if before['pose_sha256'] != after['pose_sha256'] or (
            before['track_receipt_sha256'] != after['track_receipt_sha256'] or
            before['database_sha256'] != after['database_sha256']):
        raise ValueError('sealed sources changed during candidate filtering')
    return {'schema': 'mustard_board_track_filter_counts_v1',
            'status': 'diagnostic_only_no_sparse_or_dense_model',
            'scope': 'sealed 48 TRAIN candidate graph, 39 capture-assisted fixed board poses; counts only',
            'pose_report_sha256': seal.POSE_SHA256,
            'track_receipt_sha256': seal.TRACK_RECEIPT_SHA256,
            'database_sha256': seal.DATABASE_SHA256,
            'source_manifest_sha256': seal.SOURCE_MANIFEST_SHA256,
            'stage_report_sha256': seal.STAGE_REPORT_SHA256,
            'producer_result_sha256': seal.PRODUCER_RESULT_SHA256,
            'filter_sha256': FILTER_SHA256,
            'preflight_sha256': PREFLIGHT_SHA256,
            'adapter_sha256': ADAPTER_SHA256,
            'runner_sha256': sha256(Path(__file__)),
            'gates': {'min_posed_views': 3, 'max_pair_parallax_degrees_at_least': 1.0,
                      'max_raw_and_virtual_reprojection_px': 2.0,
                      'min_camera_facing_height_squares': 1.0,
                      'printed_board_half_width_height_squares': [5.0, 4.5]},
            'counts': counts,
            'limitations': 'coarse-mask tracks may still include elevated support; no object-only proof, virtual image stage, sparse model, or dense export',
            'disk_before': before['disk_free_bytes'], 'disk_after': after['disk_free_bytes']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--preflight', action='store_true')
    mode.add_argument('--run', action='store_true', help='requires separate review')
    parser.add_argument('--worker', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    output = validate_output(args.output)
    if args.preflight:
        print(json.dumps(preflight(output), sort_keys=True, indent=2))
        return
    if args.worker:
        try:
            payload = (json.dumps(_worker_receipt(output), sort_keys=True, separators=(',', ':')) + '\n').encode()
            if len(payload) > MAX_REPORT:
                raise ValueError('counts-only report exceeds 2 MiB')
            sys.stdout.buffer.write(payload)
        except Exception as error:
            sys.stderr.write(json.dumps(_safe_failure(error), separators=(',', ':')))
            raise SystemExit(2) from None
        return
    preflight(output)
    environment = os.environ.copy()
    environment['OPENCV_OPENCL_RUNTIME'] = 'disabled'
    environment['OMP_NUM_THREADS'] = '2'
    child = subprocess.Popen([sys.executable, '-m', 'scripts.object_motion.mustard_board_track_filter_receipt',
                              '--run', '--worker', '--output', str(output)],
                             cwd=ROOT, env=environment, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE)
    deadline = time.monotonic() + SUPERVISOR_SECONDS
    peak_rss_kib = 0
    try:
        while child.poll() is None:
            if time.monotonic() > deadline:
                raise TimeoutError('board-track filter exceeded 120 seconds')
            peak_rss_kib = max(peak_rss_kib, check_worker_rss(rss_kib(child.pid)))
            free_space(MAX_REPORT)
            time.sleep(.2)
        stdout, stderr = child.communicate(timeout=5)
    except BaseException:
        child.kill()
        child.wait()
        raise
    if child.returncode or len(stdout) > MAX_REPORT or len(stderr) > MAX_STDERR:
        raise RuntimeError(f'counts-only filter failed: {_child_failure(stderr)}')
    if peak_rss_kib <= 0:
        raise RuntimeError('worker exited without a positive RSS sample')
    receipt = json.loads(stdout)
    if (receipt.get('schema') != 'mustard_board_track_filter_counts_v1' or
            receipt.get('status') != 'diagnostic_only_no_sparse_or_dense_model' or
            receipt.get('counts', {}).get('candidate_tracks') != 518):
        raise ValueError('worker did not return exact counts-only receipt')
    free_space(MAX_REPORT)
    receipt['supervisor'] = {'peak_sampled_worker_rss_kib': peak_rss_kib,
                             'rss_cap_kib': RSS_CAP_KIB,
                             'wall_cap_seconds': SUPERVISOR_SECONDS,
                             'output_cap_bytes': MAX_REPORT}
    payload = (json.dumps(receipt, sort_keys=True, separators=(',', ':')) + '\n').encode()
    if len(payload) > MAX_REPORT:
        raise ValueError('supervised receipt exceeds 2 MiB')
    with output.open('xb') as target:
        target.write(payload)
    free_space()
    print(json.dumps({'report': str(output), 'bytes': len(payload),
                      'sha256': sha256(output), 'accepted_tracks': receipt['counts']['accepted_tracks']},
                     sort_keys=True))


if __name__ == '__main__':
    main()
