#!/usr/bin/env python
"""Serial photo-only reconstruction checks, STL plus optional GLB.

A local JSON manifest supplies cases with id, photos, calibration, and optional
masks (threshold/background/sam), photo_options and attribution. Relative paths
resolve beside the manifest. No scanner, supplied mask/depth or supplied pose
may appear as reconstruction input. Reference scoring is a separate operation.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import time


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_geometry(stl, glb):
    """Check every triangle coordinate byte and winding, not just vertex counts."""
    source, data = stl.read_bytes(), glb.read_bytes()
    if data[:4] != b'glTF':
        raise ValueError('not a GLB')
    size = struct.unpack_from('<I', data, 12)[0]
    doc = json.loads(data[20:20+size])
    binary = memoryview(data)[28+size:]
    primitive = doc['meshes'][0]['primitives'][0]
    def accessor(index):
        row = doc['accessors'][index]
        view = doc['bufferViews'][row['bufferView']]
        return row, view.get('byteOffset', 0)+row.get('byteOffset', 0), view.get('byteStride')
    pos, po, stride = accessor(primitive['attributes']['POSITION'])
    ind, io, _ = accessor(primitive['indices'])
    count = struct.unpack_from('<I', source, 80)[0]
    if pos['componentType'] != 5126 or ind['componentType'] != 5125 or ind['count'] != count*3:
        raise ValueError('unexpected geometry layout')
    for i in range(count*3):
        v = struct.unpack_from('<I', binary, io+i*4)[0]
        start = po+v*(stride or 12)
        expected = 96+(i//3)*50+(i%3)*12
        if binary[start:start+12] != source[expected:expected+12]:
            raise ValueError(f'geometry differs at triangle corner {i}')
    return {'triangle_coordinates_and_winding_byte_equal': True, 'triangles': count}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--case', action='append', default=[])
    parser.add_argument('--minimum-free-gib', type=float, default=9.0)
    parser.add_argument('--texture', action='store_true', help='also export and check GLB from each own untextured mesh')
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text())
    cases = manifest['cases']
    if not cases or len({c['id'] for c in cases}) != len(cases):
        parser.error('manifest must contain cases with unique ids')
    if args.case:
        unknown = set(args.case)-{c['id'] for c in cases}
        if unknown:
            parser.error(f'unknown cases: {sorted(unknown)}')
        cases = [c for c in cases if c['id'] in args.case]
    if args.output.exists():
        parser.error('output already exists; choose a fresh directory')
    # Validate all cases before starting any work. Dataset truth stays out.
    for case in cases:
        if case['id'] in ('', '.', '..') or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_' for c in case['id']):
            parser.error('case id must be a simple name')
        if case.get('masks', 'threshold') not in ('threshold', 'background', 'sam'):
            parser.error('only photo-derived mask providers allowed')
        options = case.get('photo_options', [])
        if any(o.split('=')[0] in ('--masks', '--cameras', '--turntable-matches', '--output', '--photos', '--calibration') for o in options):
            parser.error('photo_options must not override data/providers or import matches')
        for key in ('photos', 'calibration'):
            path = Path(case[key])
            case[key] = str((args.manifest.resolve().parent/path).resolve())
            if not Path(case[key]).exists():
                parser.error(f'missing {key} for {case["id"]}')
    args.output.mkdir(parents=True)
    report = {'schema': 'crispmesh_photo_regression_v1', 'reconstruction_providers': 'photo-derived masks and turntable cameras; manifest photo provenance requires caller verification', 'cases': []}
    binary = str(args.binary.resolve())
    for case in cases:
        free = shutil.disk_usage(args.output).free/2**30
        if free < args.minimum_free_gib:
            report['stopped'] = f'only {free:.2f} GiB free'
            break
        folder = args.output/case['id']
        command = [binary, 'run', '--photos', case['photos'], '--calibration', case['calibration'], '--masks', case.get('masks','threshold'), '--cameras', 'turntable', '--output', str(folder), '--threads', '2', '--photos-option=--threads=2', '--no-live-previews', '--minimum-free-gib', str(args.minimum_free_gib)]
        command.extend('--photos-option='+o for o in case.get('photo_options', []))
        started = time.monotonic()
        with (args.output/(case['id']+'.log')).open('w') as log:
            with subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT) as process:
                while True:
                    try:
                        exit_code = process.wait(timeout=2)
                        break
                    except subprocess.TimeoutExpired:
                        if shutil.disk_usage(args.output).free/2**30 < args.minimum_free_gib:
                            process.terminate()
                            try:
                                process.wait(timeout=10)
                            except subprocess.TimeoutExpired:
                                process.kill()
                                process.wait()
                            exit_code = 125
                            break
        entry = {'id': case['id'], 'exit_code': exit_code, 'seconds': time.monotonic()-started, 'passed': False}
        try:
            pipeline = json.loads((folder/'pipeline.json').read_text())
            entry['status'] = pipeline['status']
            if exit_code != 0 or pipeline['status'] != 'complete':
                raise ValueError('photo pipeline failed; see retained report and mask sheet')
            stl = folder/'mesh/mesh.stl'
            before = sha(stl)
            entry['stl_sha256'] = before
            entry['closed'] = pipeline.get('closed')
            if case.get('require_closed', True) and entry['closed'] is not True:
                raise ValueError('expected a closed mesh; inspect holes and cameras')
            if case.get('attribution'):
                (folder/'ATTRIBUTION.txt').write_text(case['attribution'])
            if args.texture:
                with (args.output/(case['id']+'-texture.log')).open('w') as log:
                    subprocess.run([binary, 'texture', '--inputs', str(folder/'frontend/inputs'), '--mesh', str(stl), '--output', str(folder/'texture')], stdout=log, stderr=subprocess.STDOUT, check=True)
                entry['geometry'] = verify_geometry(stl, folder/'texture/mesh.glb')
                if sha(stl) != before:
                    raise ValueError('texturing modified source STL')
                entry['glb_sha256'] = sha(folder/'texture/mesh.glb')
                texture = json.loads((folder/'texture/result.json').read_text())
                entry['textured_area_fraction'] = 1-texture['untextured_area_fraction']
                if entry['textured_area_fraction'] < case.get('minimum_textured_area_fraction', 0.6):
                    raise ValueError('insufficient observed texture coverage')
            entry['passed'] = True
        except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
            entry['error'] = str(error)
        report['cases'].append(entry)
        (args.output/'result.json').write_text(json.dumps(report, indent=2)+'\n')
        print(f'{case["id"]}: {"PASS" if entry["passed"] else "FAIL"}', flush=True)
    (args.output/'result.json').write_text(json.dumps(report, indent=2)+'\n')
    return 0 if len(report['cases']) == len(cases) and all(c['passed'] for c in report['cases']) else 1


if __name__ == '__main__':
    raise SystemExit(main())
