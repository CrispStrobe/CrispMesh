#!/usr/bin/env python3
"""Hash-only 39-pose/518-track source preflight; never opens feature blobs.

This does not call the verified-track extractor, read track coordinates, fit a
point, stage virtual images, construct a sparse model, or write an artifact.
"""

import argparse
import hashlib
import json
from pathlib import Path
import shutil

import numpy as np

from scripts.object_motion import checkerboard_calibrated_pose as calibrated
from scripts.object_motion.board_frame_candidate_filter import CalibratedFrame, prepare_frames
from scripts.object_motion.mustard_verified_track_receipt import (
    sources_from_reports, sha256,
)


ROOT = Path(__file__).resolve().parents[2]
DATA = Path('/Volumes/backups/code/crispmesh-data')
POSE = DATA / 'mustard-checkerboard-calibrated-pose-001.json'
POSE_SHA256 = '25f37be625cf4edd00bdf29c479529d341cd76d3dc748d8b966e482c4668619d'
TRACK_RECEIPT = DATA / 'mustard-verified-tracks-002.json'
TRACK_RECEIPT_SHA256 = 'a374019ff3cd6c211ddacaf270eccbbd690cbe9530d194d8cf4ee65c1b8fc898'
STAGE = DATA / 'mustard-sfm-train-001'
STAGE_REPORT_SHA256 = 'bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0'
PRODUCER = DATA / 'mustard-sfm-masked-fixed-exhaustive-001'
PRODUCER_RESULT_SHA256 = '5a83c3e05e2b66887411799ecc8c5f3f41e3b8af3bbd775d308055ef7ee0041e'
DATABASE_SHA256 = '5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075'
SOURCE_MANIFEST_SHA256 = '266190342e7d807be19f3b791409c88740702ca1cc773cefde98270c0aaa2ca5'
CALIBRATION_SHA256 = 'b9ff8208e18cb16d3348ce07967f868bf64576a13dc36e3094497912f85817e0'
POSE_RUNNER_SHA256 = '6dad7bd210848480699c10348b00ec792ae5f017108a1f0ba9c74c83c218c705'
FLOOR = 10 * 1024**3


def _json(path, expected):
    if path.is_symlink() or not path.is_file() or sha256(path) != expected:
        raise ValueError(f'sealed input path/hash mismatch: {path.name}')
    return json.loads(path.read_text())


def _disk_space():
    free = {'internal_free_bytes': shutil.disk_usage(ROOT).free,
            'external_free_bytes': shutil.disk_usage(DATA).free}
    if min(free.values()) < FLOOR:
        raise ValueError('both disks require 10 GiB free')
    return free


def preflight():
    before = _disk_space()
    pose = _json(POSE, POSE_SHA256)
    track_receipt = _json(TRACK_RECEIPT, TRACK_RECEIPT_SHA256)
    stage = _json(STAGE / 'stage-report.json', STAGE_REPORT_SHA256)
    producer = _json(PRODUCER / 'result.json', PRODUCER_RESULT_SHA256)
    if (pose.get('schema') != 'mustard_checkerboard_calibrated_assisted_pose_v1' or
            pose.get('detected_count') != 39 or len(pose.get('views', [])) != 48 or
            pose.get('calibration_sha256') != CALIBRATION_SHA256 or
            pose.get('software_sha256') != POSE_RUNNER_SHA256 or
            track_receipt.get('schema') != 'mustard_verified_track_receipt_v1' or
            track_receipt.get('status') != 'diagnostic_only' or
            track_receipt.get('counts', {}).get('candidate_tracks') != 518 or
            track_receipt.get('stage_report_sha256') != STAGE_REPORT_SHA256 or
            track_receipt.get('producer_result_sha256') != PRODUCER_RESULT_SHA256 or
            track_receipt.get('database_sha256') != DATABASE_SHA256 or
            track_receipt.get('input_manifest_sha256') != SOURCE_MANIFEST_SHA256):
        raise ValueError('pose/track receipt schema, count, or provenance differs')
    k = np.asarray(pose['intrinsics']['K'], dtype=np.float64)
    d = np.asarray(pose['intrinsics']['distortion'], dtype=np.float64)
    if (not np.allclose(k, calibrated.EXPECTED_K, atol=1e-10, rtol=0) or
            not np.allclose(d, calibrated.EXPECTED_D, atol=1e-12, rtol=0)):
        raise ValueError('pose report intrinsics differ from frozen capture calibration')
    sources = sources_from_reports(stage, producer, STAGE / 'images')
    manifest = [{'name': source.name, 'image_sha256': source.image_sha256,
                 'mask_sha256': source.mask_sha256, 'width': source.width, 'height': source.height}
                for source in sources]
    manifest_hash = hashlib.sha256(json.dumps(manifest, sort_keys=True,
                                              separators=(',', ':')).encode()).hexdigest()
    if manifest_hash != SOURCE_MANIFEST_SHA256:
        raise ValueError('stage RGB/mask manifest differs from sealed track receipt')
    mask_dir = STAGE / 'masks'
    if (mask_dir.is_symlink() or not mask_dir.is_dir() or
            {path.name for path in mask_dir.iterdir()} != {source.name + '.png' for source in sources}):
        raise ValueError('mask inventory differs from sealed TRAIN names')
    for source in sources:
        path = mask_dir / (source.name + '.png')
        if path.is_symlink() or sha256(path) != source.mask_sha256:
            raise ValueError('coarse mask hash mismatch')
    database = PRODUCER / 'database.db'
    if (database.is_symlink() or not database.is_file() or database.stat().st_size > 20*1024**2 or
            sha256(database) != DATABASE_SHA256 or
            any(Path(str(database) + suffix).exists() for suffix in ('-wal', '-shm'))):
        raise ValueError('sealed verified-index DB path/hash/sidecar differs')
    by_name = {source.name: source for source in sources}
    if len(by_name) != 48 or {view['name'] for view in pose['views']} != set(by_name):
        raise ValueError('pose and sealed TRAIN image names differ')
    frames = []
    for view in pose['views']:
        source = by_name[view['name']]
        if view['source_sha256'] != source.image_sha256:
            raise ValueError('pose and staged RGB hashes differ')
        if not view['detected']:
            continue
        frames.append(CalibratedFrame(view['name'], source.width, source.height,
                                      (float(k[0, 0]), float(k[1, 1]),
                                       float(k[0, 2]), float(k[1, 2])),
                                      tuple(map(float, d)),
                                      tuple(tuple(map(float, row)) for row in view['camera_from_board_rotation']),
                                      tuple(map(float, view['camera_from_board_translation_square_units']))))
    _, side = prepare_frames(frames, expected_count=39)
    return {'schema': 'mustard_board_track_filter_preflight_v1',
            'status': 'ready_without_track_coordinates', 'writes': False,
            'sealed_candidate_tracks': 518, 'posed_images': len(frames),
            'source_images': len(sources), 'camera_facing_board_sign': side,
            'pose_sha256': POSE_SHA256, 'track_receipt_sha256': TRACK_RECEIPT_SHA256,
            'database_sha256': DATABASE_SHA256,
            'source_manifest_sha256': SOURCE_MANIFEST_SHA256,
            'stage_report_sha256': STAGE_REPORT_SHA256,
            'producer_result_sha256': PRODUCER_RESULT_SHA256,
            'disk_free_bytes': before, 'track_coordinates_read': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--preflight', action='store_true', required=True)
    parser.parse_args()
    print(json.dumps(preflight(), sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
