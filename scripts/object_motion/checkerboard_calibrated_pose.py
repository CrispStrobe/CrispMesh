#!/usr/bin/env python3
"""TRAIN-only moving-board PnP with separately supplied NP3 lens calibration.

Only calibration HDF5 /NP3_rgb_K and /NP3_rgb_d are opened. No per-angle pose,
scanner/depth/held-out source, or previous reference residual enters fitting.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

os.environ.setdefault('OPENCV_OPENCL_RUNTIME', 'disabled')
os.environ.setdefault('OPENBLAS_NUM_THREADS', '2')
import cv2
import numpy as np

from scripts.object_motion import checkerboard_pose as base
from scripts.object_motion.ycb_camera_reference import dataset


ROOT = Path(__file__).resolve().parents[2]
EXTERNAL_BASE = Path('/Volumes/backups/code/crispmesh-data')
CALIBRATION = EXTERNAL_BASE / 'mustard-pose-metadata-001/metadata/calibration.h5'
CALIBRATION_SHA256 = 'b9ff8208e18cb16d3348ce07967f868bf64576a13dc36e3094497912f85817e0'
RECEIPT = EXTERNAL_BASE / 'mustard-pose-metadata-001/receipt.json'
RECEIPT_SHA256 = '3a2b156289b7f7f11d20ef5026ffe9f19893333d155f0acb6e326f027ceb0747'
BASE_RUNNER_SHA256 = '1b61a7bff763bdd439686f0578a2f97bf7b5a2a21ef94696c322b4762a00fbbf'
EXPECTED_K = np.array([[1075.2419846389048, 0, 614.25314259272193],
                       [0, 1075.2350192560712, 483.27330482841086],
                       [0, 0, 1]], dtype=np.float64)
EXPECTED_D = np.array([-0.034258101828110302, -0.17233961642613393,
                       0.005402898860710775, 0.000508169628442456,
                       1.6656440844715967], dtype=np.float64)
TIME_CAP_SECONDS = 180


def read_calibration():
    if (CALIBRATION.is_symlink() or base.digest(CALIBRATION) != CALIBRATION_SHA256 or
            RECEIPT.is_symlink() or base.digest(RECEIPT) != RECEIPT_SHA256):
        raise ValueError('capture calibration or acquisition receipt hash mismatch')
    receipt = json.loads(RECEIPT.read_text())
    member = receipt.get('extraction', {}).get('members', {}).get('006_mustard_bottle/calibration.h5')
    if (receipt.get('status') != 'metadata_only_no_camera_evaluation' or
            member != {'bytes': 63544, 'sha256': CALIBRATION_SHA256}):
        raise ValueError('capture calibration not bound to sealed metadata receipt')
    # Deliberately name only these two datasets: no extrinsics or per-angle H5.
    k = dataset(CALIBRATION, '/NP3_rgb_K', (3, 3))
    distortion = dataset(CALIBRATION, '/NP3_rgb_d', (5,))
    if (not np.isfinite(k).all() or not np.isfinite(distortion).all() or
            not np.allclose(k, EXPECTED_K, rtol=0, atol=1e-10) or
            not np.allclose(distortion, EXPECTED_D, rtol=0, atol=1e-12)):
        raise ValueError('NP3 RGB calibration values drifted from frozen plan')
    return k, distortion


def preflight():
    prepared, profile = base.preflight()
    if base.digest(Path(base.__file__)) != BASE_RUNNER_SHA256:
        raise ValueError('frozen base checkerboard runner changed')
    k, distortion = read_calibration()
    return {'schema': 'mustard_checkerboard_calibrated_preflight_v1',
            'status': 'ready_no_photo_decode', 'writes': False,
            'image_count': len(prepared['images']),
            'manifest_sha256': base.MANIFEST_SHA256,
            'profile_sha256': base.PROFILE_SHA256,
            'calibration_sha256': CALIBRATION_SHA256,
            'metadata_receipt_sha256': RECEIPT_SHA256,
            'intrinsics': {'K': k.tolist(), 'distortion': distortion.tolist()},
            'disk_free_bytes': prepared['disk_free_bytes']}, prepared, profile, k, distortion


def pose_candidates(corners, k, distortion):
    points = np.asarray(corners, dtype=np.float64)
    if points.shape != (8, 9, 2) or not np.isfinite(points).all():
        raise ValueError('expected finite 8x9 corner grid')
    candidates = []
    for flip in (0, 1):
        image_points = (points if not flip else points[::-1, ::-1]).reshape(-1, 2).copy()
        count, rvecs, tvecs, _ = cv2.solvePnPGeneric(
            base.OBJECT_CORNERS, image_points, k, distortion, flags=cv2.SOLVEPNP_IPPE)
        if count != 2:
            continue
        for planar_branch, (rvec, tvec) in enumerate(zip(rvecs, tvecs)):
            rotation = cv2.Rodrigues(rvec)[0]
            translation = np.asarray(tvec, dtype=np.float64).reshape(3)
            depths = (rotation @ base.OBJECT_CORNERS.T + translation[:, None])[2]
            projected = cv2.projectPoints(base.OBJECT_CORNERS, rvec, tvec, k,
                                          distortion)[0].reshape(-1, 2)
            residuals = np.linalg.norm(projected - image_points, axis=1)
            if (not np.isfinite(rotation).all() or not np.isfinite(translation).all() or
                    not np.isfinite(residuals).all() or np.min(depths) <= 0):
                continue
            candidates.append({'label_flip_180': flip, 'planar_branch': planar_branch,
                               'rms_px': float(np.sqrt(np.mean(residuals**2))),
                               'p95_px': float(np.percentile(residuals, 95)),
                               'rotation': rotation, 'translation': translation,
                               'center': -rotation.T @ translation})
    return candidates


def detect_frame(gray, name, slot_index, k, distortion):
    if gray is None or gray.shape != (1024, 1280) or gray.dtype != np.uint8:
        raise ValueError('TRAIN JPEG must decode as 1280x1024 grayscale')
    found, corners = cv2.findChessboardCornersSB(gray, base.PATTERN, flags=base.SB_FLAGS)
    row = {'name': name, 'slot_index': slot_index, 'detected': bool(found)}
    if not found:
        return {**row, 'reason': 'full_board_not_detected', 'candidates': []}
    points = np.asarray(corners, dtype=np.float64).reshape(8, 9, 2)
    flat = points.reshape(-1, 2)
    if (not np.isfinite(flat).all() or np.min(flat) < 0 or
            np.max(flat[:, 0]) >= 1280 or np.max(flat[:, 1]) >= 1024):
        return {**row, 'reason': 'invalid_or_out_of_frame_corners', 'candidates': []}
    hull = float(cv2.contourArea(cv2.convexHull(flat.astype(np.float32))) / gray.size)
    bbox = list(map(int, cv2.boundingRect(flat.astype(np.float32))))
    if hull < base.MIN_HULL_FRACTION:
        return {**row, 'reason': 'board_coverage_too_small',
                'hull_fraction': hull, 'bbox_xywh': bbox, 'candidates': []}
    candidates = pose_candidates(points, k, distortion)
    return {**row, 'hull_fraction': hull, 'bbox_xywh': bbox,
            'reason': None if candidates else 'no_positive_depth_ippe_pose',
            'candidates': candidates}


def evaluate(prepared, profile, k, distortion):
    slots = base.validate_profile(profile)
    cv2.setNumThreads(2)
    cv2.ocl.setUseOpenCL(False)
    started = time.monotonic()
    frames = []
    for record in prepared['images']:
        if time.monotonic() - started > TIME_CAP_SECONDS:
            raise TimeoutError('calibrated board diagnostic exceeded 180 seconds')
        gray = cv2.imread(record['path'], cv2.IMREAD_GRAYSCALE)
        frames.append(detect_frame(gray, record['name'], slots.index(record['name']),
                                   k, distortion))
    selection = base.select_cycle(frames, len(slots))
    by_name = {frame['name']: (frame, candidate) for frame, candidate in
               zip(selection['frames'], selection['chosen'])} if selection else {}
    views, orbit_rows = [], []
    for record, frame in zip(prepared['images'], frames):
        chosen = by_name.get(record['name'])
        row = {'name': record['name'], 'source_sha256': record['sha256'],
               'detected': frame['detected'], 'reason': frame['reason'],
               'hull_fraction': frame.get('hull_fraction'),
               'bbox_xywh': frame.get('bbox_xywh')}
        if chosen:
            candidate = chosen[1]
            row.update({'label_flip_180': candidate['label_flip_180'],
                        'planar_branch': candidate['planar_branch'],
                        'reprojection_rms_px': candidate['rms_px'],
                        'reprojection_p95_px': candidate['p95_px'],
                        'quality_flags': (['rms_above_3px'] if candidate['rms_px'] > base.RMS_FLAG_PX else []) +
                                         (['p95_above_5px'] if candidate['p95_px'] > base.P95_FLAG_PX else []),
                        'camera_from_board_rotation': candidate['rotation'].tolist(),
                        'camera_from_board_translation_square_units': candidate['translation'].tolist(),
                        'camera_center_board_square_units': candidate['center'].tolist(),
                        'candidate_reprojection_rms_px': [round(item['rms_px'], 6)
                                                          for item in chosen[0]['candidates']]})
            orbit_rows.append({'name': record['name'], 'center': candidate['center'],
                               'camera_to_world_rotation': candidate['rotation'].T})
        views.append(row)
    orbit = base.orbit_evaluate(orbit_rows, profile) if len(orbit_rows) >= 12 else None
    if orbit:
        orbit = {key: orbit.get(key) for key in
                 ('status', 'reason', 'failures', 'registered', 'coverage', 'planar_axis_ratio',
                  'winding_degrees', 'reversed_significant_step_fraction',
                  'inward_facing_fraction', 'maximum_adjacent_center_step_over_radius',
                  'maximum_adjacent_orientation_step_degrees')}
    return {'schema': 'mustard_checkerboard_calibrated_assisted_pose_v1',
            'status': 'diagnostic_only' if selection else 'unavailable',
            'scope': 'sealed TRAIN photos plus Berkeley NP3 capture intrinsics only; moving board',
            'intrinsics': {'K': k.tolist(), 'distortion': distortion.tolist(),
                           'source': 'calibration.h5 /NP3_rgb_K and /NP3_rgb_d only'},
            'board': {'inner_corners_columns_rows': [9, 8], 'square_units': 'arbitrary',
                      'frame': 'centered inner-corner grid, z=0'},
            'manifest_sha256': base.MANIFEST_SHA256,
            'profile_sha256': base.PROFILE_SHA256,
            'calibration_sha256': CALIBRATION_SHA256,
            'metadata_receipt_sha256': RECEIPT_SHA256,
            'base_runner_sha256': BASE_RUNNER_SHA256,
            'software_sha256': base.digest(Path(__file__)), 'opencv_version': cv2.__version__,
            'detected_count': sum(bool(frame['candidates']) for frame in frames),
            'selection': ({'cost': selection['cost'],
                           'median_camera_range_square_units': selection['median_camera_range_square_units'],
                           'gauge': selection['gauge']} if selection else None),
            'orbit_diagnostic': orbit, 'views': views,
            'limitations': 'capture-calibration-assisted moving-board pose in arbitrary squares, not board-free/photo-only, metric bottle pose, or dense accuracy; global 180-degree gauge remains'}


def validate_output(path):
    path = Path(path)
    if (path.parent != EXTERNAL_BASE or
            not path.name.startswith('mustard-checkerboard-calibrated-pose-') or
            path.suffix != '.json' or path.exists() or path.is_symlink() or
            EXTERNAL_BASE.is_symlink() or not EXTERNAL_BASE.is_dir()):
        raise ValueError('fresh explicit external calibrated-pose JSON required')
    return path


def postflight(prepared):
    if (base.digest(base.MANIFEST) != base.MANIFEST_SHA256 or
            base.digest(base.PROFILE) != base.PROFILE_SHA256 or
            base.digest(CALIBRATION) != CALIBRATION_SHA256 or
            base.digest(RECEIPT) != RECEIPT_SHA256):
        raise ValueError('sealed input changed during calibrated PnP')
    for record in prepared['images']:
        if base.digest(record['path']) != record['sha256']:
            raise ValueError('TRAIN JPEG changed during calibrated PnP')
    return base.free_space()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--evaluate', action='store_true')
    parser.add_argument('--worker', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    output = validate_output(args.output)
    summary, prepared, profile, k, distortion = preflight()
    if not args.evaluate:
        print(json.dumps({**summary, 'output': str(output)}, sort_keys=True))
        return
    if not args.worker:
        command = [sys.executable, '-m', 'scripts.object_motion.checkerboard_calibrated_pose',
                   '--evaluate', '--worker', '--output', str(output)]
        environment = os.environ.copy()
        environment['OPENCV_OPENCL_RUNTIME'] = 'disabled'
        environment['TMPDIR'] = str(EXTERNAL_BASE)
        environment['OMP_NUM_THREADS'] = '2'
        child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 env=environment, cwd=ROOT)
        deadline = time.monotonic() + TIME_CAP_SECONDS
        try:
            while child.poll() is None:
                if time.monotonic() > deadline:
                    raise TimeoutError('calibrated checkerboard worker exceeded 180 seconds')
                base.free_space()
                time.sleep(1)
            stdout, stderr = child.communicate(timeout=5)
        except BaseException:
            child.kill()
            child.wait()
            raise
        if child.returncode or len(stdout) > 1024 or len(stderr) > 4096:
            raise RuntimeError(f'calibrated checkerboard worker failed ({child.returncode}): {stderr[:1024]!r}')
        print(stdout.decode().strip())
        return
    result = evaluate(prepared, profile, k, distortion)
    postflight(prepared)
    payload = (json.dumps(result, sort_keys=True, indent=2) + '\n').encode()
    if len(payload) > base.OUTPUT_CAP:
        raise ValueError('calibrated diagnostic exceeds 2 MiB output cap')
    with output.open('xb') as stream:
        stream.write(payload)
    base.free_space()
    print(json.dumps({'status': result['status'], 'detected_count': result['detected_count'],
                      'report': str(output), 'report_sha256': base.digest(output)}, sort_keys=True))


if __name__ == '__main__':
    main()
