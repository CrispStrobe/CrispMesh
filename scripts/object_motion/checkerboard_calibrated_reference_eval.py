#!/usr/bin/env python3
"""Evaluation-only adapter for sealed capture-calibrated mustard board cameras.

Reference H5 pose values are loaded only by --run, after preflight. No score
changes the image-derived camera report or its detector/branch choices.
"""

import argparse
import json
from pathlib import Path
import re
import shutil

import numpy as np

from scripts.object_motion import checkerboard_pose_reference_eval as shared


ROOT = Path(__file__).resolve().parents[2]
EXTERNAL_BASE = Path('/Volumes/backups/code/crispmesh-data')
POSE_REPORT = EXTERNAL_BASE / 'mustard-checkerboard-calibrated-pose-001.json'
POSE_REPORT_SHA256 = '25f37be625cf4edd00bdf29c479529d341cd76d3dc748d8b966e482c4668619d'
POSE_RUNNER_SHA256 = '6dad7bd210848480699c10348b00ec792ae5f017108a1f0ba9c74c83c218c705'
METADATA_ROOT = EXTERNAL_BASE / 'mustard-pose-metadata-001'
RECEIPT = METADATA_ROOT / 'receipt.json'
RECEIPT_SHA256 = '3a2b156289b7f7f11d20ef5026ffe9f19893333d155f0acb6e326f027ceb0747'
ARCHIVE_SHA256 = '5d9b1837eb58b0760463e99021a53fe6e82d5cd2457141945445ed6df06ff3f7'
ARCHIVE_BYTES = 657272400
SHARED_EVALUATOR_SHA256 = 'f24442884ecb9e102ee5ad96fb4727b56dc30e94d0f92aa783c6350704878c7b'
PREFIX = '006_mustard_bottle/'
FLOOR = 10 * 1024**3
OUTPUT_CAP = 2 * 1024**2


def selected_views(report):
    if (report.get('schema') != 'mustard_checkerboard_calibrated_assisted_pose_v1' or
            report.get('status') != 'diagnostic_only' or
            report.get('software_sha256') != POSE_RUNNER_SHA256 or
            report.get('detected_count') != 39 or len(report.get('views', [])) != 48):
        raise ValueError('unexpected sealed calibrated pose report')
    seen, selected = set(), {}
    for view in report['views']:
        name = view.get('name')
        if (not isinstance(name, str) or
                not re.fullmatch(r'NP3_(?:0[0-9]{2}|[12][0-9]{2}|3[0-5][0-9])\.jpg', name) or
                int(name[4:7]) % 6 or name in seen):
            raise ValueError('invalid, duplicate, or non-slot TRAIN name')
        seen.add(name)
        if 'camera_from_board_rotation' in view:
            if not view.get('detected') or 'camera_from_board_translation_square_units' not in view:
                raise ValueError('incomplete selected board camera')
            selected[name] = view
        elif view.get('detected'):
            raise ValueError('detected frame lacks selected camera')
    if len(seen) != 48 or len(selected) != 39:
        raise ValueError('expected 48 TRAIN frames and 39 selected cameras')
    return selected


def disk_space():
    values = {'internal_free_bytes': shutil.disk_usage(ROOT).free,
              'external_free_bytes': shutil.disk_usage(EXTERNAL_BASE).free}
    if min(values.values()) < FLOOR:
        raise RuntimeError('both disks must retain at least 10 GiB free')
    return values


def validate_output(output):
    output = Path(output)
    if (output.parent != EXTERNAL_BASE or
            not re.fullmatch(r'mustard-checkerboard-calibrated-camera-eval-[a-zA-Z0-9_-]+\.json', output.name) or
            output.exists() or output.is_symlink() or EXTERNAL_BASE.is_symlink()):
        raise ValueError('fresh explicit external calibrated-camera evaluation JSON required')
    return output


def verify_small_inputs(receipt):
    if shared.sha256(POSE_REPORT) != POSE_REPORT_SHA256 or shared.sha256(RECEIPT) != RECEIPT_SHA256:
        raise ValueError('sealed report or metadata receipt hash mismatch')
    for name, record in receipt['extraction']['members'].items():
        path = METADATA_ROOT / 'metadata' / Path(name).relative_to(PREFIX)
        if path.is_symlink() or path.stat().st_size != record['bytes'] or shared.sha256(path) != record['sha256']:
            raise ValueError(f'metadata H5 hash mismatch: {name}')
    return len(receipt['extraction']['members'])


def preflight(output):
    output = validate_output(output)
    before = disk_space()
    if shared.sha256(Path(shared.__file__)) != SHARED_EVALUATOR_SHA256:
        raise ValueError('frozen shared comparison algorithm changed')
    archive = METADATA_ROOT / '006_mustard_bottle_berkeley_rgbd.tgz'
    if (archive.is_symlink() or archive.stat().st_size != ARCHIVE_BYTES or
            shared.sha256(archive) != ARCHIVE_SHA256):
        raise ValueError('correct mustard Berkeley archive hash mismatch')
    if POSE_REPORT.is_symlink() or RECEIPT.is_symlink():
        raise ValueError('sealed report or receipt symlink')
    if shared.sha256(POSE_REPORT) != POSE_REPORT_SHA256 or shared.sha256(RECEIPT) != RECEIPT_SHA256:
        raise ValueError('sealed calibrated report or receipt hash mismatch')
    report = json.loads(POSE_REPORT.read_text())
    receipt = json.loads(RECEIPT.read_text())
    if (receipt.get('schema') != 'mustard_pose_metadata_acquisition_v1' or
            receipt.get('status') != 'metadata_only_no_camera_evaluation' or
            receipt.get('checkerboard_report_sha256') != shared.POSE_REPORT_SHA256 or
            receipt.get('source', {}).get('archive', {}).get('sha256') != ARCHIVE_SHA256):
        raise ValueError('unexpected metadata provenance receipt')
    selected = selected_views(report)
    expected_members = {f'{PREFIX}calibration.h5'} | {
        f'{PREFIX}poses/NP5_{int(name[4:7])}_pose.h5' for name in selected}
    if set(receipt.get('extraction', {}).get('members', {})) != expected_members:
        raise ValueError('reference inventory does not match selected TRAIN names')
    count = verify_small_inputs(receipt)
    return {'schema': 'mustard_checkerboard_calibrated_reference_preflight_v1',
            'status': 'ready_without_camera_scoring', 'writes': False,
            'output': str(output), 'matched': len(selected),
            'source_report_sha256': POSE_REPORT_SHA256,
            'metadata_receipt_sha256': RECEIPT_SHA256,
            'archive_sha256': ARCHIVE_SHA256,
            'metadata_member_count': count, 'disk_free_bytes': before}, report, receipt, selected


def evaluate(output):
    prepared, report, receipt, selected = preflight(output)
    metadata = METADATA_ROOT / 'metadata'
    calibration = metadata / 'calibration.h5'
    np3_from_np5 = shared.rigid(shared.dataset(calibration, '/H_NP3_from_NP5', (4, 4)),
                                 'mustard NP3 rig calibration')
    source_centers, source_rotations, target_centers, target_rotations = {}, {}, {}, {}
    for name, view in sorted(selected.items()):
        rotation = shared.proper_rotation(view['camera_from_board_rotation'], name)
        translation = np.asarray(view['camera_from_board_translation_square_units'], dtype=float)
        source_rotations[name] = rotation
        source_centers[name] = shared.camera_center(rotation, translation)
        angle = int(name[4:7])
        pose = metadata / 'poses' / f'NP5_{angle}_pose.h5'
        table_from_ref = shared.rigid(shared.dataset(pose, '/H_table_from_reference_camera', (4, 4)), name)
        np3_from_table = shared.rigid(np3_from_np5 @ np.linalg.inv(table_from_ref), name + ' NP3-from-table')
        target_rotations[name] = np3_from_table[:3, :3]
        target_centers[name] = shared.camera_center(np3_from_table[:3, :3], np3_from_table[:3, 3])
    comparison = shared.compare(source_centers, source_rotations, target_centers, target_rotations)
    count = verify_small_inputs(receipt)
    result = {'schema': 'mustard_calibrated_checkerboard_berkeley_camera_posthoc_v1',
              'status': 'posthoc_partial_arc_diagnostic_only',
              'interpretation': 'same fixed proper positive-scale center Sim3, LOO, and same-Q orientation as uncalibrated evaluation; no pose adjustment',
              'transform_convention': 'H_NP3_from_table = H_NP3_from_NP5 @ inverse(H_table_from_reference_camera); board_frame_offset unresolved and not inserted',
              'limitations': 'calibration-assisted moving-board partial arc; neither board-free SfM nor Google mesh accuracy; nearly planar alignment weakens third axis',
              'calibrated_report_sha256': POSE_REPORT_SHA256,
              'calibrated_runner_sha256': POSE_RUNNER_SHA256,
              'metadata_receipt_sha256': RECEIPT_SHA256,
              'shared_evaluator_sha256': SHARED_EVALUATOR_SHA256,
              'metadata_h5_sha256': {name: record['sha256'] for name, record in receipt['extraction']['members'].items()},
              'post_score_verified_h5_members': count,
              'not_detected_train_names': sorted(view['name'] for view in report['views'] if not view['detected']),
              'comparison': comparison, 'disk_before': prepared['disk_free_bytes'],
              'full_turn_orbit_gate': 'unavailable_at_39_of_60_slots',
              'google_mesh_depth_or_heldout_used': False}
    encoded = (json.dumps(result, indent=2, sort_keys=True) + '\n').encode()
    if len(encoded) > OUTPUT_CAP:
        raise ValueError('calibrated camera comparison output exceeds 2 MiB')
    disk_space()
    with Path(prepared['output']).open('xb') as target:
        target.write(encoded)
    disk_space()
    return {'output': prepared['output'], 'bytes': len(encoded),
            'sha256': shared.sha256(prepared['output']), 'matched': comparison['matched_count']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--preflight', action='store_true')
    mode.add_argument('--run', action='store_true', help='requires separate review')
    args = parser.parse_args()
    result = preflight(args.output)[0] if args.preflight else evaluate(args.output)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
