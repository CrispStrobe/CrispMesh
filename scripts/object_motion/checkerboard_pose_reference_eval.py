#!/usr/bin/env python3
"""Posthoc mustard checkerboard-camera comparison to Berkeley rig metadata.

Evaluation only. Neither these HDF5 poses nor scores enter board detection,
PnP, branch choice, or intrinsics. No scanner/depth/held-out source is used.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import shutil

import numpy as np

from scripts.object_motion.ycb_camera_reference import dataset, rigid


ROOT = Path(__file__).resolve().parents[2]
EXTERNAL_BASE = Path('/Volumes/backups/code/crispmesh-data')
POSE_REPORT = EXTERNAL_BASE / 'mustard-checkerboard-pose-001.json'
POSE_REPORT_SHA256 = 'e4b686795cc6f3bfba57ac77e9225113714d9d306b5919990191dffbd049a856'
METADATA_ROOT = EXTERNAL_BASE / 'mustard-pose-metadata-001'
RECEIPT = METADATA_ROOT / 'receipt.json'
RECEIPT_SHA256 = '3a2b156289b7f7f11d20ef5026ffe9f19893333d155f0acb6e326f027ceb0747'
ARCHIVE_SHA256 = '5d9b1837eb58b0760463e99021a53fe6e82d5cd2457141945445ed6df06ff3f7'
ARCHIVE_BYTES = 657272400
OUTPUT_CAP = 2 * 1024**2
FLOOR = 10 * 1024**3
PREFIX = '006_mustard_bottle/'


def sha256(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as source:
        for block in iter(lambda: source.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def proper_rotation(rotation, label):
    rotation = np.asarray(rotation, dtype=float)
    if (rotation.shape != (3, 3) or not np.isfinite(rotation).all() or
            not np.allclose(rotation.T @ rotation, np.eye(3), atol=1e-3) or
            not np.isclose(np.linalg.det(rotation), 1.0, atol=1e-3)):
        raise ValueError(f'{label}: non-proper or nonfinite rotation')
    return rotation


def camera_center(rotation, translation):
    rotation = proper_rotation(rotation, 'camera')
    translation = np.asarray(translation, dtype=float)
    if translation.shape != (3,) or not np.isfinite(translation).all():
        raise ValueError('camera: invalid translation')
    return -rotation.T @ translation


def fit_similarity(source, target):
    """One positive-scale, proper-rotation least-squares center Sim(3)."""
    source, target = np.asarray(source, dtype=float), np.asarray(target, dtype=float)
    if (source.shape != target.shape or source.ndim != 2 or source.shape[1] != 3 or
            len(source) < 4 or not np.isfinite(source).all() or not np.isfinite(target).all()):
        raise ValueError('expected >=4 finite paired 3-D centers')
    sx, sy = source.mean(axis=0), target.mean(axis=0)
    x, y = source - sx, target - sy
    if min(np.linalg.matrix_rank(x), np.linalg.matrix_rank(y)) < 2:
        raise ValueError('camera center constellation is degenerate')
    variance = float(np.mean(np.sum(x*x, axis=1)))
    if variance <= 1e-12:
        raise ValueError('camera center spread is too small')
    u, singular, vt = np.linalg.svd(y.T @ x / len(source))
    correction = np.diag([1.0, 1.0, np.linalg.det(u @ vt)])
    q = proper_rotation(u @ correction @ vt, 'fitted world rotation')
    scale = float(np.sum(singular * np.diag(correction)) / variance)
    if not math.isfinite(scale) or scale <= 0:
        raise ValueError('fitted similarity has nonpositive scale')
    translation = sy - scale * q @ sx
    return scale, q, translation, singular


def orientation_degrees(source_world_to_camera, target_world_to_camera, q):
    source = proper_rotation(source_world_to_camera, 'image-derived camera')
    target = proper_rotation(target_world_to_camera, 'supplied camera')
    predicted = source @ proper_rotation(q, 'world alignment').T
    cosine = float(np.clip((np.trace(target @ predicted.T) - 1) / 2, -1, 1))
    return math.degrees(math.acos(cosine))


def compare(source_centers, source_rotations, target_centers, target_rotations):
    names = sorted(source_centers)
    if not names or not (set(names) == set(source_rotations) == set(target_centers) == set(target_rotations)):
        raise ValueError('camera names must match exactly')
    source = np.asarray([source_centers[name] for name in names], dtype=float)
    target = np.asarray([target_centers[name] for name in names], dtype=float)
    scale, q, translation, singular = fit_similarity(source, target)
    radius = float(np.median(np.linalg.norm(target - target.mean(axis=0), axis=1)))
    if not math.isfinite(radius) or radius <= 1e-12:
        raise ValueError('supplied median camera radius is degenerate')
    predicted = scale * (source @ q.T) + translation
    residual = np.linalg.norm(predicted - target, axis=1)
    loo = []
    for index in range(len(names)):
        keep = np.arange(len(names)) != index
        other_scale, other_q, other_t, _ = fit_similarity(source[keep], target[keep])
        loo.append(float(np.linalg.norm(other_scale * other_q @ source[index] + other_t - target[index])))
    angles = [orientation_degrees(source_rotations[name], target_rotations[name], q) for name in names]

    def stats(values):
        array = np.asarray(values, dtype=float)
        return {'median': float(np.median(array)), 'p95': float(np.quantile(array, .95)),
                'max': float(np.max(array)), 'rms': float(np.sqrt(np.mean(array**2)))}

    return {'matched_names': names, 'matched_count': len(names),
            'fit': {'scale_supplied_units_per_square': scale, 'world_rotation': q.tolist(),
                    'world_translation_supplied_units': translation.tolist(),
                    'rotation_determinant': float(np.linalg.det(q)),
                    'center_covariance_singular_values': singular.tolist(),
                    'center_covariance_smallest_to_largest_ratio': float(singular[-1] / singular[0]),
                    'center_covariance_rank': int(np.linalg.matrix_rank(np.diag(singular))),
                    'supplied_median_center_radius': radius},
            'center_residual_supplied_units': stats(residual),
            'center_residual_over_supplied_median_radius': stats(residual / radius),
            'leave_one_out_center_residual_over_supplied_median_radius': stats(np.asarray(loo) / radius),
            'orientation_residual_degrees': stats(angles),
            'per_frame': {name: {'center_residual_supplied_units': float(residual[index]),
                                 'center_residual_over_radius': float(residual[index] / radius),
                                 'leave_one_out_center_residual_over_radius': float(loo[index] / radius),
                                 'orientation_residual_degrees': float(angles[index])}
                          for index, name in enumerate(names)}}


def inspect_inputs():
    if POSE_REPORT.is_symlink() or sha256(POSE_REPORT) != POSE_REPORT_SHA256:
        raise ValueError('image-derived report hash mismatch')
    if RECEIPT.is_symlink() or sha256(RECEIPT) != RECEIPT_SHA256:
        raise ValueError('metadata acquisition receipt hash mismatch')
    archive = METADATA_ROOT / '006_mustard_bottle_berkeley_rgbd.tgz'
    if (archive.is_symlink() or archive.stat().st_size != ARCHIVE_BYTES or
            sha256(archive) != ARCHIVE_SHA256):
        raise ValueError('mustard Berkeley archive size or hash mismatch')
    report = json.loads(POSE_REPORT.read_text())
    receipt = json.loads(RECEIPT.read_text())
    if (report.get('schema') != 'mustard_checkerboard_assisted_pose_diagnostic_v1' or
            report.get('detected_count') != 39 or len(report.get('views', [])) != 48 or
            receipt.get('schema') != 'mustard_pose_metadata_acquisition_v1' or
            receipt.get('status') != 'metadata_only_no_camera_evaluation' or
            receipt.get('checkerboard_report_sha256') != POSE_REPORT_SHA256 or
            receipt.get('source', {}).get('archive', {}).get('sha256') != ARCHIVE_SHA256 or
            receipt.get('source', {}).get('archive', {}).get('bytes') != ARCHIVE_BYTES):
        raise ValueError('unexpected sealed input schemas or provenance')
    found = receipt['extraction']['members']
    if len(found) != 40 or receipt['extraction']['total_bytes'] > 2 * 1024**2:
        raise ValueError('unexpected metadata inventory')
    for name, record in found.items():
        if not name.startswith(PREFIX) or name != str(Path(name)):
            raise ValueError('unsafe metadata member')
        path = METADATA_ROOT / 'metadata' / Path(name).relative_to(PREFIX)
        if path.is_symlink() or path.stat().st_size != record['bytes'] or sha256(path) != record['sha256']:
            raise ValueError(f'extracted H5 hash mismatch: {name}')
    selected = {}
    for view in report['views']:
        if not view['detected']:
            continue
        name = view['name']
        if not re.fullmatch(r'NP3_[0-3][0-9]{2}\.jpg', name) or name in selected:
            raise ValueError('invalid or repeated image-derived camera name')
        selected[name] = view
    if len(selected) != 39:
        raise ValueError('expected 39 fixed image-derived cameras')
    expected = {f'{PREFIX}calibration.h5'} | {
        f'{PREFIX}poses/NP5_{int(name[4:7])}_pose.h5' for name in selected}
    if set(found) != expected:
        raise ValueError('metadata inventory does not match selected TRAIN names')
    return report, receipt, selected


def disk_space():
    values = {'internal_free_bytes': shutil.disk_usage(ROOT).free,
              'external_free_bytes': shutil.disk_usage(EXTERNAL_BASE).free}
    if min(values.values()) < FLOOR:
        raise RuntimeError('both disks must retain 10 GiB free')
    return values


def verify_postflight(receipt):
    if (sha256(POSE_REPORT) != POSE_REPORT_SHA256 or
            sha256(RECEIPT) != RECEIPT_SHA256):
        raise ValueError('post-score sealed report or receipt hash mismatch')
    for name, record in receipt['extraction']['members'].items():
        path = METADATA_ROOT / 'metadata' / Path(name).relative_to(PREFIX)
        if path.is_symlink() or path.stat().st_size != record['bytes'] or sha256(path) != record['sha256']:
            raise ValueError(f'post-score metadata hash mismatch: {name}')
    return {'source_report_sha256': POSE_REPORT_SHA256,
            'metadata_receipt_sha256': RECEIPT_SHA256,
            'verified_h5_members': len(receipt['extraction']['members'])}


def validate_output(output):
    output = Path(output)
    if output.parent != EXTERNAL_BASE or not re.fullmatch(r'mustard-checkerboard-camera-eval-[a-zA-Z0-9_-]+\.json', output.name):
        raise ValueError('output must be a fresh explicit external JSON path')
    if output.exists() or output.is_symlink():
        raise FileExistsError(output)
    return output


def preflight(output):
    output = validate_output(output)
    before = disk_space()
    _, receipt, selected = inspect_inputs()
    return {'schema': 'mustard_checkerboard_reference_preflight_v1',
            'status': 'ready_without_camera_scoring', 'writes': False,
            'output': str(output), 'matched': len(selected),
            'source_report_sha256': POSE_REPORT_SHA256,
            'archive_sha256': ARCHIVE_SHA256,
            'receipt_sha256': RECEIPT_SHA256,
            'metadata_member_count': len(receipt['extraction']['members']),
            'disk_free_bytes': before}, receipt, selected


def evaluate(output):
    prepared, receipt, selected = preflight(output)
    output = Path(prepared['output'])
    before = prepared['disk_free_bytes']
    metadata = METADATA_ROOT / 'metadata'
    calibration = metadata / 'calibration.h5'
    np3_from_np5 = rigid(dataset(calibration, '/H_NP3_from_NP5', (4, 4)), 'mustard NP3 calibration')
    k = dataset(calibration, '/NP3_rgb_K', (3, 3))
    distortion = dataset(calibration, '/NP3_rgb_d', (5,))
    if (not np.allclose(k[2], [0, 0, 1], atol=1e-10) or
            min(k[0, 0], k[1, 1]) <= 0):
        raise ValueError('invalid supplied NP3 RGB intrinsics')
    source_centers, source_rotations = {}, {}
    target_centers, target_rotations = {}, {}
    for name, view in sorted(selected.items()):
        rotation = proper_rotation(view['camera_from_board_rotation'], name)
        translation = np.asarray(view['camera_from_board_translation_square_units'], dtype=float)
        source_rotations[name] = rotation
        source_centers[name] = camera_center(rotation, translation)
        angle = int(name[4:7])
        pose_path = metadata / 'poses' / f'NP5_{angle}_pose.h5'
        table_from_ref = rigid(dataset(pose_path, '/H_table_from_reference_camera', (4, 4)), name)
        np3_from_table = rigid(np3_from_np5 @ np.linalg.inv(table_from_ref), name + ' NP3-from-table')
        target_rotations[name] = np3_from_table[:3, :3]
        target_centers[name] = camera_center(np3_from_table[:3, :3], np3_from_table[:3, 3])
    comparison = compare(source_centers, source_rotations, target_centers, target_rotations)
    postflight = verify_postflight(receipt)
    result = {'schema': 'mustard_checkerboard_berkeley_camera_posthoc_v1',
              'status': 'posthoc_partial_arc_diagnostic_only',
              'interpretation': 'one proper positive-scale center Sim(3); no image-derived pose or branch changed; not mesh accuracy',
              'transform_convention': 'H_NP3_from_table = H_NP3_from_NP5 @ inverse(H_table_from_reference_camera); both world-to-camera OpenCV axes; board_frame_offset not inserted',
              'convention_caveat': 'matrix-name composition is documented but board_frame_offset and absolute object frame were not independently validated against depth; orientation residual is conditional on this convention',
              'conditioning_caveat': 'a nearly planar camera-center arc weakens the third-axis Sim(3) constraint; inspect reported covariance singular values and ratio rather than treating the alignment as full-volume validation',
              'source_report_sha256': POSE_REPORT_SHA256,
              'metadata_receipt_sha256': RECEIPT_SHA256,
              'input_integrity_postflight': postflight,
              'metadata_h5_sha256': {name: value['sha256'] for name, value in receipt['extraction']['members'].items()},
              'supplied_intrinsics': {'K': k.tolist(), 'distortion': distortion.tolist(),
                                      'image_derived_assumed_K': [[1536, 0, 640], [0, 1536, 512], [0, 0, 1]]},
              'not_detected_train_names': sorted(view['name'] for view in json.loads(POSE_REPORT.read_text())['views']
                                                 if not view['detected']),
              'comparison': comparison, 'disk_before': before,
              'full_turn_orbit_gate': 'unavailable_at_39_of_60_slots',
              'google_mesh_or_depth_used': False}
    encoded = (json.dumps(result, indent=2, sort_keys=True) + '\n').encode()
    if len(encoded) > OUTPUT_CAP:
        raise ValueError('evaluation report exceeds 2 MiB cap')
    with output.open('xb') as target:
        target.write(encoded)
    disk_space()
    return {'output': str(output), 'bytes': len(encoded), 'sha256': sha256(output),
            'matched': comparison['matched_count']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--preflight', action='store_true')
    mode.add_argument('--run', action='store_true', help='requires separate review')
    args = parser.parse_args()
    result = preflight(args.output)[0] if args.preflight else evaluate(args.output)
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
