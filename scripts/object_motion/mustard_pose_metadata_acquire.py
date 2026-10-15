#!/usr/bin/env python3
"""Bounded acquisition of mustard Berkeley *camera metadata only*.

This is deliberately separate from checkerboard pose estimation and scoring.
No reference pose is opened by --preflight. A live --run requires review.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tarfile
import time
from urllib.request import HTTPRedirectHandler, Request, build_opener

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[2]
EXTERNAL_BASE = Path('/Volumes/backups/code/crispmesh-data')
MANIFEST = ROOT / 'build-opencv/ycb-vps-acquisition-001/mustard-manifest.json'
MANIFEST_SHA256 = 'c48998dffc7798e7d21f275383b1c67dc2e5e9313a8bfe699f72303f61583b52'
POSE_REPORT = EXTERNAL_BASE / 'mustard-checkerboard-pose-001.json'
POSE_REPORT_SHA256 = 'e4b686795cc6f3bfba57ac77e9225113714d9d306b5919990191dffbd049a856'
URL = ('https://ycb-benchmarks.s3.amazonaws.com/data/berkeley/006_mustard_bottle/'
       '006_mustard_bottle_berkeley_rgbd.tgz')
ARCHIVE_BYTES = 657272400
ARCHIVE_SHA256 = '5d9b1837eb58b0760463e99021a53fe6e82d5cd2457141945445ed6df06ff3f7'
ETAG = '"c15b25428f8eea3432996eb2b4565de3-79"'
INTERNAL_FLOOR = 10 * 1024**3
EXTERNAL_FLOOR_WITH_RESERVE = 11 * 1024**3
METADATA_CAP = 2 * 1024**2
TOTAL_CAP = 660 * 1024**2
RECEIPT_CAP = 64 * 1024
TRANSFER_TIMEOUT = 1200
EXTRACTION_TIMEOUT = 1200
MAX_TAR_MEMBERS = 10000
MAX_UNCOMPRESSED_VISITED = 12 * 1024**3
PREFIX = '006_mustard_bottle/'
ARCHIVE_NAME = '006_mustard_bottle_berkeley_rgbd.tgz'


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, new_url):
        raise ValueError(f'official source redirected (HTTP {code})')


OPENER = build_opener(NoRedirect)


def sha256(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def allowlist(report):
    if report.get('schema') != 'mustard_checkerboard_assisted_pose_diagnostic_v1' or (
            report.get('detected_count') != 39 or len(report.get('views', [])) != 48):
        raise ValueError('unexpected sealed checkerboard report')
    seen, selected = set(), set()
    for view in report['views']:
        name = view.get('name')
        if not isinstance(name, str) or not re.fullmatch(r'NP3_(?:0[0-9]{2}|[12][0-9]{2}|3[0-5][0-9])\.jpg', name):
            raise ValueError('unexpected TRAIN name')
        angle = int(name[4:7])
        if angle % 6 or name in seen:
            raise ValueError('duplicate or non-slot TRAIN name')
        seen.add(name)
        if view.get('detected'):
            if 'camera_from_board_rotation' not in view:
                raise ValueError('detected pose absent')
            selected.add(f'{PREFIX}poses/NP5_{angle}_pose.h5')
    if len(seen) != 48 or len(selected) != 39:
        raise ValueError('incorrect TRAIN or detected cardinality')
    return {f'{PREFIX}calibration.h5'} | selected


def disk_space(projected=False):
    internal = shutil.disk_usage(ROOT).free
    external = shutil.disk_usage(EXTERNAL_BASE).free
    needed = TOTAL_CAP if projected else 0
    if internal < INTERNAL_FLOOR or external - needed < EXTERNAL_FLOOR_WITH_RESERVE:
        raise RuntimeError('10 GiB disk floors or 1 GiB external worker reserve not met')
    return {'internal_free_bytes': internal, 'external_free_bytes': external,
            'external_projected_after_cap_bytes': external - needed}


def validate_output(path):
    path = Path(path)
    if path.parent != EXTERNAL_BASE or path.name in ('', '.', '..') or (
            not re.fullmatch(r'mustard-pose-metadata-[a-zA-Z0-9_-]+', path.name)):
        raise ValueError('output must be a fresh, explicit mustard-pose-metadata-* child of external base')
    if path.exists() or path.is_symlink():
        raise FileExistsError(path)
    if EXTERNAL_BASE.is_symlink() or not EXTERNAL_BASE.is_dir():
        raise ValueError('external base absent or symlinked')
    return path


def preflight(output):
    output = validate_output(output)
    if MANIFEST.is_symlink() or sha256(MANIFEST) != MANIFEST_SHA256:
        raise ValueError('mustard acquisition manifest hash mismatch')
    manifest = json.loads(MANIFEST.read_text())
    source = manifest['archives']['berkeley_rgbd']
    remote = source['source']
    if (manifest.get('object_id') != '006_mustard_bottle' or
            source.get('bytes') != ARCHIVE_BYTES or
            source.get('observed_sha256') != ARCHIVE_SHA256 or
            remote.get('url') != URL or remote.get('head_content_length') != ARCHIVE_BYTES or
            remote.get('head_etag') != ETAG):
        raise ValueError('mustard source provenance mismatch')
    if POSE_REPORT.is_symlink() or sha256(POSE_REPORT) != POSE_REPORT_SHA256:
        raise ValueError('sealed image-derived pose report hash mismatch')
    members = allowlist(json.loads(POSE_REPORT.read_text()))
    return {'schema': 'mustard_pose_metadata_acquisition_preflight_v1',
            'status': 'ready_without_source_access', 'output': str(output),
            'url': URL, 'archive_expected_bytes': ARCHIVE_BYTES,
            'archive_expected_sha256': ARCHIVE_SHA256, 'etag_drift_check': ETAG,
            'manifest_sha256': MANIFEST_SHA256, 'pose_report_sha256': POSE_REPORT_SHA256,
            'member_count': len(members), 'allowlisted_members': sorted(members),
            'space': disk_space(projected=True), 'writes': False}, members


def head_source():
    with OPENER.open(Request(URL, method='HEAD'), timeout=20) as response:
        headers = response.headers
        result = {'status': response.status, 'resolved_url': response.geturl(),
                  'content_length': headers.get('Content-Length'),
                  'etag': headers.get('ETag'), 'last_modified': headers.get('Last-Modified')}
    if (result['status'] != 200 or result['resolved_url'] != URL or
            result['content_length'] != str(ARCHIVE_BYTES) or result['etag'] != ETAG):
        raise ValueError('official HEAD drifted from pinned source')
    return result


def download(archive_partial, started):
    if archive_partial.exists() or archive_partial.is_symlink():
        raise FileExistsError(archive_partial)
    with OPENER.open(Request(URL, method='GET'), timeout=30) as response:
        if (response.status != 200 or response.geturl() != URL or
                response.headers.get('Content-Length') != str(ARCHIVE_BYTES) or
                response.headers.get('ETag') != ETAG):
            raise ValueError('GET response did not match HEAD and pinned source')
        count, h = 0, hashlib.sha256()
        with archive_partial.open('xb') as target:
            while True:
                if time.monotonic() - started > TRANSFER_TIMEOUT:
                    raise TimeoutError('download wall-clock cap exceeded')
                block = response.read(1 << 20)
                if not block:
                    break
                count += len(block)
                if count > ARCHIVE_BYTES:
                    raise ValueError('archive exceeded exact byte cap')
                target.write(block)
                h.update(block)
                if count % (16 << 20) < len(block):
                    disk_space()
    if count != ARCHIVE_BYTES or h.hexdigest() != ARCHIVE_SHA256:
        raise ValueError('downloaded archive size or SHA-256 mismatch')
    return {'bytes': count, 'sha256': h.hexdigest()}


def extract_metadata(archive, metadata_dir, members, *, time_cap=EXTRACTION_TIMEOUT):
    """Stream tar with exact member allowlist; never call TarFile.extract."""
    metadata_dir = Path(metadata_dir)
    if metadata_dir.exists() or metadata_dir.is_symlink():
        raise FileExistsError(metadata_dir)
    metadata_dir.mkdir()
    expected = set(members)
    if len(expected) != 40 or not all(name.startswith(PREFIX) for name in expected):
        raise ValueError('expected calibration and exactly 39 mustard pose members')
    found, total, visited, count = {}, 0, 0, 0
    started = time.monotonic()
    with tarfile.open(archive, mode='r|gz') as stream:
        for member in stream:
            if time.monotonic() - started > time_cap:
                raise TimeoutError('metadata scan wall-clock cap exceeded')
            count += 1
            visited += max(0, member.size)
            if count > MAX_TAR_MEMBERS or visited > MAX_UNCOMPRESSED_VISITED:
                raise ValueError('tar member/expanded-size cap exceeded')
            if member.name not in expected:
                continue
            if member.name in found or not member.isfile() or member.size <= 0 or (
                    total + member.size > METADATA_CAP):
                raise ValueError('duplicate, invalid, or oversized allowlisted metadata')
            relative = Path(member.name).relative_to(PREFIX)
            destination = metadata_dir / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            h, written = hashlib.sha256(), 0
            source = stream.extractfile(member)
            if source is None:
                raise ValueError('metadata member unreadable')
            with source, destination.open('xb') as target:
                while written < member.size:
                    block = source.read(min(1 << 20, member.size - written))
                    if not block:
                        raise ValueError('truncated metadata member')
                    target.write(block)
                    h.update(block)
                    written += len(block)
            if written != member.size:
                raise ValueError('metadata member size mismatch')
            total += written
            found[member.name] = {'bytes': written, 'sha256': h.hexdigest()}
    if set(found) != expected:
        raise ValueError(f'missing allowlisted metadata members: {sorted(expected - set(found))}')
    return {'members': found, 'total_bytes': total, 'scanned_members': count,
            'scanned_uncompressed_bytes': visited}


def run(output):
    prepared, members = preflight(output)
    head = head_source()
    disk_space(projected=True)
    folder = Path(prepared['output'])
    folder.mkdir()
    partial = folder / (ARCHIVE_NAME + '.part')
    archive = folder / ARCHIVE_NAME
    transfer = download(partial, time.monotonic())
    if archive.exists() or archive.is_symlink():
        raise FileExistsError(archive)
    os.replace(partial, archive)
    metadata = extract_metadata(archive, folder / 'metadata', members)
    if sha256(archive) != ARCHIVE_SHA256 or sha256(POSE_REPORT) != POSE_REPORT_SHA256:
        raise ValueError('postflight source/report hash mismatch')
    receipt = {'schema': 'mustard_pose_metadata_acquisition_v1',
               'status': 'metadata_only_no_camera_evaluation',
               'time_utc': datetime.now(timezone.utc).isoformat(),
               'source': {'url': URL, 'head': head, 'archive': transfer,
                          'manifest_sha256': MANIFEST_SHA256},
               'checkerboard_report_sha256': POSE_REPORT_SHA256,
               'extraction': metadata, 'disk_space': disk_space()}
    encoded = (json.dumps(receipt, indent=2, sort_keys=True) + '\n').encode()
    total = ARCHIVE_BYTES + metadata['total_bytes'] + len(encoded)
    if len(encoded) > RECEIPT_CAP or total > TOTAL_CAP:
        raise ValueError('receipt or total artifact cap exceeded')
    with (folder / 'receipt.json').open('xb') as target:
        target.write(encoded)
    disk_space()
    return {'output': str(folder), 'archive_sha256': ARCHIVE_SHA256,
            'metadata_members': len(metadata['members']), 'total_artifact_bytes': total,
            'receipt_sha256': sha256(folder / 'receipt.json')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True, type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--preflight', action='store_true')
    mode.add_argument('--run', action='store_true', help='requires separate human review')
    args = parser.parse_args()
    result = preflight(args.output_dir)[0] if args.preflight else run(args.output_dir)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
