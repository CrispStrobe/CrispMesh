"""Read-only, sealed TRAIN mask geometry; semantic QA only with reviewed labels."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import cv2
import numpy as np
from PIL import Image

STAGE = Path('/Volumes/backups/code/crispmesh-data/mustard-sfm-train-001')
STAGE_SHA256 = 'bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0'
APPROVED_SEMANTIC_MANIFEST_SHA256 = None  # No independently accepted labels exist yet.
NAMES = tuple(f'NP3_{a:03d}.jpg' for a in range(0, 360, 6) if a % 30 != 24)
SIZE = (1280, 1024)
ROLE = 'accepted coarse pose support, not object silhouette or ground truth'
CLASSES = {'bottle_object': 1, 'board_support': 2, 'background': 3}


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def regular(path: Path, limit: int) -> None:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > limit:
        raise ValueError(f'missing, linked, or oversized file: {path}')


def exact_directory(path: Path, expected: set[str]) -> None:
    if path.is_symlink() or not path.is_dir() or {p.name for p in path.iterdir()} != expected:
        raise ValueError(f'missing, linked, extra, or absent TRAIN files: {path}')
    if any(p.is_symlink() or not p.is_file() for p in path.iterdir()):
        raise ValueError(f'linked or non-file TRAIN item: {path}')


def decode(path: Path, fmt: str, mode: str, size: tuple[int, int]) -> np.ndarray:
    regular(path, 8 * 1024 * 1024)
    with Image.open(path) as im:
        if im.format != fmt or im.mode != mode or im.size != size:
            raise ValueError(f'format, mode, or size mismatch: {path}')
        im.load()
        return np.asarray(im).copy()


def mask_metrics(mask: np.ndarray) -> dict:
    if mask.ndim != 2 or not np.all((mask == 0) | (mask == 255)):
        raise ValueError('support mask must be 0/255 grayscale')
    binary = (mask == 255).astype(np.uint8)
    kept = int(binary.sum())
    if not 0 < kept < binary.size:
        raise ValueError('support mask must have both classes')
    n, _, stats, _ = cv2.connectedComponentsWithStats(binary, connectivity=8)
    ys, xs = np.nonzero(binary)
    return {
        'kept_pixels': kept,
        'support_fraction': kept / binary.size,
        'bbox_xyxy_exclusive': [int(xs.min()), int(ys.min()), int(xs.max() + 1), int(ys.max() + 1)],
        'components_8': int(n - 1),
        'largest_component_share': int(stats[1:, cv2.CC_STAT_AREA].max()) / kept,
        'border_pixels': int(binary[0, :].sum() + binary[-1, :].sum() +
                             binary[1:-1, 0].sum() + binary[1:-1, -1].sum()),
    }


def semantic_metrics(mask: np.ndarray, label: np.ndarray) -> dict:
    if label.shape != mask.shape or not np.all(label <= 3):
        raise ValueError('semantic label shape or codes differ')
    support = mask == 255
    result = {'unknown_pixels': int(np.count_nonzero(label == 0))}
    for title, code in CLASSES.items():
        pixels = label == code
        n = int(np.count_nonzero(pixels))
        overlap = int(np.count_nonzero(pixels & support))
        result[title] = {'labeled_pixels': n, 'inside_support': overlap,
                         'support_coverage_of_labeled_class': overlap / n if n else None}
    return result


def validate_labels(manifest: dict, names: tuple[str, ...], photo_hashes: dict) -> None:
    if (manifest.get('schema') != 'mustard_photo_semantic_labels_v1' or
            manifest.get('status') != 'validated_photo_only' or
            manifest.get('source') != 'manual_photo_review' or
            not isinstance(manifest.get('reviewer'), str) or not manifest['reviewer'].strip() or
            not isinstance(manifest.get('reviewed_at'), str) or not manifest['reviewed_at'].strip() or
            set(manifest.get('labels', {})) != set(names)):
        raise ValueError('independently reviewed photo-only semantic manifest required')
    for name in names:
        entry = manifest['labels'][name]
        digest = entry.get('sha256')
        if (entry.get('photo_sha256') != photo_hashes[name] or
                not isinstance(digest, str) or len(digest) != 64 or
                set(digest) - set('0123456789abcdef')):
            raise ValueError(f'invalid semantic binding: {name}')


def audit(stage_root: Path, stage: dict, label_root: Path | None = None,
          label_manifest: dict | None = None, size: tuple[int, int] = SIZE,
          names: tuple[str, ...] = NAMES) -> dict:
    if (stage.get('schema') != 'mustard_train_only_stage_v1' or
            stage.get('status') != 'complete' or stage.get('mask_role') != ROLE or
            stage.get('train_names') != list(names) or
            set(stage.get('cleaned_masks', {})) != set(names) or
            set(stage.get('train_photo_sha256', {})) != set(names)):
        raise ValueError('staged TRAIN manifest differs')
    if (label_root is None) != (label_manifest is None):
        raise ValueError('semantic manifest and directory must be supplied together')
    if label_manifest is not None:
        validate_labels(label_manifest, names, stage['train_photo_sha256'])
        exact_directory(label_root, {name + '.png' for name in names})
    exact_directory(stage_root / 'images', set(names))
    exact_directory(stage_root / 'masks', {name + '.png' for name in names})
    rows = []
    for name in names:
        photo = stage_root / 'images' / name
        mask_path = stage_root / 'masks' / (name + '.png')
        if sha256(photo) != stage['train_photo_sha256'][name] or sha256(mask_path) != stage['cleaned_masks'][name]['sha256']:
            raise ValueError(f'changed photo or support mask: {name}')
        decode(photo, 'JPEG', 'RGB', size)
        mask = decode(mask_path, 'PNG', 'L', size)
        metrics = mask_metrics(mask)
        if metrics['kept_pixels'] != stage['cleaned_masks'][name]['kept_pixels'] or size[0] * size[1] - metrics['kept_pixels'] != stage['cleaned_masks'][name]['ignored_pixels']:
            raise ValueError(f'staged support count differs: {name}')
        if label_manifest is None:
            semantics = {'status': 'unknown', 'reason': 'no independently validated photo-only labels',
                         **{key: None for key in CLASSES}}
        else:
            label_path = label_root / (name + '.png')
            if sha256(label_path) != label_manifest['labels'][name]['sha256']:
                raise ValueError(f'changed semantic label: {name}')
            semantics = {'status': 'validated_photo_only', **semantic_metrics(mask, decode(label_path, 'PNG', 'L', size))}
            if sha256(label_path) != label_manifest['labels'][name]['sha256']:
                raise ValueError(f'semantic label changed during audit: {name}')
        if sha256(photo) != stage['train_photo_sha256'][name] or sha256(mask_path) != stage['cleaned_masks'][name]['sha256']:
            raise ValueError(f'photo or support mask changed during audit: {name}')
        rows.append({'name': name, 'support': metrics, 'semantics': semantics,
                     'review_flags': [flag for flag, active in
                                      [('touches_image_border', metrics['border_pixels'] > 0),
                                       ('multiple_support_components', metrics['components_8'] > 1)] if active]})
    return {'schema': 'mustard_mask_qa_v1', 'views': len(rows),
            'semantic_status': 'validated_photo_only' if label_manifest else 'unknown_abstain',
            'total_support_pixels': sum(row['support']['kept_pixels'] for row in rows), 'per_view': rows}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage', type=Path, default=STAGE)
    parser.add_argument('--semantic-manifest', type=Path)
    parser.add_argument('--semantic-manifest-sha256')
    parser.add_argument('--semantic-dir', type=Path)
    args = parser.parse_args()
    if args.stage != STAGE:
        raise ValueError('only the sealed TRAIN stage is accepted by CLI')
    stage_file = args.stage / 'stage-report.json'
    regular(stage_file, 1024 * 1024)
    if sha256(stage_file) != STAGE_SHA256:
        raise ValueError('TRAIN stage seal differs')
    stage = json.loads(stage_file.read_text())
    manifest = None
    if args.semantic_manifest or args.semantic_dir or args.semantic_manifest_sha256:
        if not (args.semantic_manifest and args.semantic_dir and args.semantic_manifest_sha256):
            raise ValueError('semantic manifest, SHA-256, and directory required together')
        if (APPROVED_SEMANTIC_MANIFEST_SHA256 is None or
                args.semantic_manifest_sha256 != APPROVED_SEMANTIC_MANIFEST_SHA256):
            raise ValueError('no independently approved semantic manifest is pinned')
        regular(args.semantic_manifest, 1024 * 1024)
        if sha256(args.semantic_manifest) != args.semantic_manifest_sha256:
            raise ValueError('semantic manifest seal differs')
        manifest = json.loads(args.semantic_manifest.read_text())
    result = audit(args.stage, stage, args.semantic_dir, manifest)
    if sha256(stage_file) != STAGE_SHA256:
        raise ValueError('TRAIN stage changed during audit')
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
