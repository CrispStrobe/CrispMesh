"""Cheap measures between two turntable photos, tried as a warning while taking photos ("too far apart").

For every 3DLF object and steps of 1 to 5 photos (about 5 to 25 degrees) over all 73 pairs of the turn:

  sift      SIFT matches (ratio 0.8) and their fundamental-matrix inliers at full size, features on the dark
            object only (grey < 90, dilated): close to what the camera solver sees
  orbW      ORB (FAST + rotated BRIEF), 1000 features at W pixels across, cross-checked, Hamming <= 50,
            matches that moved at least a pixel (the static backdrop excluded), their fundamental-matrix inliers
  step      apparent turn in degrees: median horizontal motion of the moving ORB matches at 320 pixels over half
            the width of the region that changed between the two photos

Writes one JSON with the per-pair values and prints median / 10th percentile / minimum per object and step.
Usage: python pair_measures.py TEST_DATA OUT.json   (TEST_DATA holds 3dlf-scan-<object>/pro/<object>/rgb)
"""
import json
import sys

import cv2
import numpy as np

OBJECTS = ['bunny', 'armadillo', 'dragon', 'lucy', 'thai_statue', 'happy']
COUNT = 73
STEPS = (1, 2, 3, 4, 5)
WIDTHS = (320, 480, 640)


def ransac_inliers(pa, pb, threshold):
    if len(pa) < 8:
        return 0
    _, mask = cv2.findFundamentalMat(np.float32(pa), np.float32(pb), cv2.FM_RANSAC, threshold, 0.999)
    return int(mask.sum()) if mask is not None else 0


def sift_pair(fa, fb):
    (ka, da), (kb, db) = fa, fb
    if da is None or db is None or len(ka) < 8 or len(kb) < 8:
        return [0, 0]
    pairs = cv2.BFMatcher(cv2.NORM_L2).knnMatch(da, db, k=2)
    good = [p[0] for p in pairs if len(p) == 2 and p[0].distance < 0.8 * p[1].distance]
    pa = [ka[g.queryIdx].pt for g in good]
    pb = [kb[g.trainIdx].pt for g in good]
    return [len(good), ransac_inliers(pa, pb, 2.0)]


def orb_pair(fa, fb):
    (ka, da), (kb, db) = fa, fb
    if da is None or db is None:
        return [0, 0, 0], []
    matches = cv2.BFMatcher(cv2.NORM_HAMMING, crossCheck=True).match(da, db)
    good = [g for g in matches if g.distance <= 50]
    moving = [(ka[g.queryIdx].pt, kb[g.trainIdx].pt) for g in good
              if np.hypot(kb[g.trainIdx].pt[0] - ka[g.queryIdx].pt[0], kb[g.trainIdx].pt[1] - ka[g.queryIdx].pt[1]) >= 1]
    inliers = ransac_inliers([a for a, _ in moving], [b for _, b in moving], 1.5)
    return [len(good), len(moving), inliers], [b[0] - a[0] for a, b in moving]


def apparent_step(a, b, dx):
    changed = np.abs(a.astype(int) - b.astype(int)) > 12
    columns = np.where(changed.mean(0) > 0.02)[0]
    if len(dx) < 5 or len(columns) == 0:
        return float('nan')
    half_width = (columns.max() - columns.min() + 1) / 2
    # A surface point moves by r * angle * cos(phi); |cos| averages 2 / pi over the visible half.
    return float(np.degrees(np.median(np.abs(dx)) / half_width * np.pi / 2))


def main():
    data, out = sys.argv[1], sys.argv[2]
    sift = cv2.SIFT_create(nfeatures=6000, contrastThreshold=0.03)
    orb = cv2.ORB_create(nfeatures=1000, fastThreshold=10, edgeThreshold=15, patchSize=15)
    results = {}
    for name in OBJECTS:
        grey = [cv2.imread(f'{data}/3dlf-scan-{name}/pro/{name}/rgb/{name}_{i}_rgb.png', cv2.IMREAD_GRAYSCALE) for i in range(COUNT)]
        masks = [cv2.dilate((g < 90).astype(np.uint8), np.ones((15, 15), np.uint8)) for g in grey]
        sift_features = [sift.detectAndCompute(g, m) for g, m in zip(grey, masks)]
        small = {w: [cv2.resize(g, (w, round(g.shape[0] * w / g.shape[1])), interpolation=cv2.INTER_AREA) for g in grey] for w in WIDTHS}
        orb_features = {w: [orb.detectAndCompute(s, None) for s in small[w]] for w in WIDTHS}
        by_step = {}
        for k in STEPS:
            rows = []
            for i in range(COUNT):
                j = (i + k) % COUNT
                row = {'i': i, 'sift': sift_pair(sift_features[i], sift_features[j])}
                for w in WIDTHS:
                    row[f'orb{w}'], dx = orb_pair(orb_features[w][i], orb_features[w][j])
                    if w == 320:
                        row['step'] = apparent_step(small[w][i], small[w][j], dx)
                rows.append(row)
            by_step[k] = rows
        results[name] = by_step
        print(name, flush=True)
    json.dump(results, open(out, 'w'))
    for key in ['sift', 'orb320', 'orb480', 'orb640', 'step']:
        print(f'== {key} (last value: inliers, or degrees): median / p10 / min per step')
        for name, by_step in results.items():
            line = f'{name:12s}'
            for k in STEPS:
                v = np.array([r[key][-1] if isinstance(r[key], list) else r[key] for r in by_step[k]], float)
                line += f' | {k}: {np.nanmedian(v):6.1f} {np.nanpercentile(v, 10):6.1f} {np.nanmin(v):5.0f}'
            print(line)


if __name__ == '__main__':
    main()
