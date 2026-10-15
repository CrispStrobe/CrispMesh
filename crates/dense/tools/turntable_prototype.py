"""Prototype of the planned `turntable` camera provider: cameras for ordered
turntable photos with a known lens, without AliceVision or COLMAP.

An experiment to settle the approach before it is written in Rust; it uses
OpenCV's SIFT and essential-matrix code and SciPy's least squares.

    python turntable_prototype.py --images DIR --masks DIR --calibration LENS.json --output MODEL_DIR

`--images` holds capture_NNNN.png (the contrast images of `crispmesh-dense
photos --keep-intermediates`, `work/contrast`), `--masks` the object masks
under the same names. The result is a COLMAP text model (cameras.txt,
images.txt, points3D.txt) that `crispmesh-dense photos --cameras import:MODEL_DIR`
reads, so gates, scene and dense stages are the usual ones.

Steps:
 1. SIFT features inside the masks; matches between each photo and its next
    three (the turn is closed: the last photos are matched with the first).
 2. Essential matrix per consecutive pair. A turntable repeats one motion: a
    rotation about a fixed axis. The axis direction and its position relative
    to the camera are taken as medians over all pairs (distance camera to
    axis = 1 fixes the scale).
 3. The angle of every step by a one-dimensional search on that pair's matches
    with the axis fixed (a step of zero, a turntable that did not move, is
    allowed). Poses follow by adding up the angles.
 4. Matches checked against those poses, joined into tracks, triangulated.
 5. Bundle adjustment of all poses (six free parameters each; nothing forces
    them back onto the turntable model) and points, lens fixed, Huber loss;
    observations far off are dropped and the adjustment repeated.
"""

import argparse, json, time
from pathlib import Path

import cv2
import numpy as np
from scipy.optimize import minimize_scalar
from scipy.sparse import bsr_matrix, csr_matrix
from scipy.spatial.transform import Rotation


def load_lens(path, width, height):
    d = json.loads(Path(path).read_text())
    sx, sy = width / d["calibration_width"], height / d["calibration_height"]
    K = np.array([[d["fx"] * sx, 0, (d["cx"] + 0.5) * sx - 0.5], [0, d["fy"] * sy, (d["cy"] + 0.5) * sy - 0.5], [0, 0, 1.0]])
    return K, np.array([d["k1"], d["k2"], 0, 0, d["k3"]])


def sampson(E, a, b):
    """Sampson distance of normalised point pairs (b' E a = 0)."""
    ah, bh = np.c_[a, np.ones(len(a))], np.c_[b, np.ones(len(b))]
    Ea, Etb = ah @ E.T, bh @ E
    num = (bh * Ea).sum(1) ** 2
    return num / (Ea[:, 0] ** 2 + Ea[:, 1] ** 2 + Etb[:, 0] ** 2 + Etb[:, 1] ** 2 + 1e-18)


def skew(v):
    return np.array([[0, -v[2], v[1]], [v[2], 0, -v[0]], [-v[1], v[0], 0]])


def step_motion(axis, centre, angle):
    R = Rotation.from_rotvec(axis * angle).as_matrix()
    return R, centre - R @ centre


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for name in ("images", "masks", "calibration", "output"):
        p.add_argument("--" + name, type=Path, required=True)
    p.add_argument("--features", type=int, default=6000)
    p.add_argument("--span", type=int, default=4, help="match every photo with this many successors")
    p.add_argument("--contrast", type=float, default=0.01, help="SIFT contrast threshold")
    p.add_argument("--export-matches", type=Path, help="write keypoints and matches as JSON (input of crispmesh-dense photos --turntable-matches)")
    p.add_argument("--open-turn", action="store_true", help="the photos do not close a full turn: do not scale the steps to 360 degrees")
    args = p.parse_args()
    started = time.time()
    names = sorted(f.name for f in args.images.glob("capture_*.png"))
    n = len(names)
    sift = cv2.SIFT_create(nfeatures=args.features, contrastThreshold=args.contrast)
    keys, descs, norms = [], [], []
    for name in names:
        gray = cv2.imread(str(args.images / name), cv2.IMREAD_GRAYSCALE)
        mask = cv2.erode((cv2.imread(str(args.masks / name), cv2.IMREAD_GRAYSCALE) > 127).astype(np.uint8), np.ones((5, 5), np.uint8))
        kp, de = sift.detectAndCompute(gray, mask)
        pts = np.array([k.pt for k in kp], np.float64).reshape(-1, 2)
        keys.append(pts)
        descs.append(de if de is not None else np.zeros((0, 128), np.float32))
    h, w = gray.shape
    K, dist = load_lens(args.calibration, w, h)
    focal = (K[0, 0] + K[1, 1]) / 2
    for pts in keys:
        norms.append(cv2.undistortPoints(pts.reshape(-1, 1, 2), K, dist).reshape(-1, 2) if len(pts) else pts)
    t_features = time.time() - started
    print("features per photo: min %d median %d" % (min(map(len, keys)), np.median(list(map(len, keys)))), flush=True)

    matcher = cv2.BFMatcher(cv2.NORM_L2)
    pairs = {}
    for i in range(n):
        for d in range(1, args.span + 1):
            j = (i + d) % n
            if len(descs[i]) < 8 or len(descs[j]) < 8:
                continue
            fwd = [m for m, q in matcher.knnMatch(descs[i], descs[j], k=2) if m.distance < 0.8 * q.distance]
            back = {m.queryIdx: m.trainIdx for m, q in matcher.knnMatch(descs[j], descs[i], k=2) if m.distance < 0.8 * q.distance}
            pairs[(i, j)] = np.array([(m.queryIdx, m.trainIdx) for m in fwd if back.get(m.trainIdx) == m.queryIdx], int).reshape(-1, 2)
    t_matching = time.time() - started - t_features

    if args.export_matches:
        args.export_matches.write_text(json.dumps({
            "schema": "crispmesh_turntable_matches_v1", "width": w, "height": h, "photos": names,
            "keypoints": [np.round(k, 3).tolist() for k in keys],
            "pairs": [{"first": i, "second": j, "matches": m.tolist()} for (i, j), m in pairs.items()]}, separators=(",", ":")))

    # One motion per step: essential matrix of each consecutive pair.
    axes, angles, centres, moved = [], [], [], []
    for i in range(n):
        m = pairs.get((i, (i + 1) % n), np.zeros((0, 2), int))
        a, b = norms[i][m[:, 0]], norms[(i + 1) % n][m[:, 1]]
        flow = np.median(np.linalg.norm(a - b, axis=1)) * focal if len(m) else 0.0
        moved.append(len(m) >= 30 and flow > 2.0)
        if not moved[-1]:
            continue
        E, inl = cv2.findEssentialMat(a, b, np.eye(3), cv2.RANSAC, 0.999, 1.5 / focal)
        if E is None or E.shape != (3, 3):
            moved[-1] = False
            continue
        _, R, t, _ = cv2.recoverPose(E, a, b, np.eye(3), mask=inl)
        rv = Rotation.from_matrix(R).as_rotvec()
        angle = np.linalg.norm(rv)
        if angle < 1e-4:
            moved[-1] = False
            continue
        axis = rv / angle
        tp = t.ravel() - axis * (axis @ t.ravel())
        tp /= np.linalg.norm(tp) + 1e-12
        alpha = np.arctan2(-np.sin(angle), 1 - np.cos(angle))
        u = Rotation.from_rotvec(-alpha * axis).apply(tp)  # direction from the camera to the axis
        axes.append(axis); angles.append(angle); centres.append(u)
    axes, centres = np.array(axes), np.array(centres)
    axes *= np.sign(axes @ axes[0])[:, None]
    axis = np.median(axes, 0); axis /= np.linalg.norm(axis)
    centres -= np.outer(centres @ axis, axis)
    centre = np.median(centres, 0); centre /= np.linalg.norm(centre)
    typical = float(np.median(angles))
    print("pairs with motion %d of %d; axis %s spread %.2f deg; step median %.2f deg" % (
        len(angles), n, axis.round(3), np.degrees(np.median(np.arccos(np.clip(axes @ axis, -1, 1)))), np.degrees(typical)), flush=True)

    # The angle of every step with the axis fixed.
    steps = []
    for i in range(n):
        m = pairs.get((i, (i + 1) % n), np.zeros((0, 2), int))
        if len(m) < 12:
            steps.append(typical); continue
        a, b = norms[i][m[:, 0]], norms[(i + 1) % n][m[:, 1]]

        def cost(angle):
            R, t = step_motion(axis, centre, angle)
            e = np.sqrt(sampson(skew(t) @ R, a, b)) * focal
            return np.minimum(e, 3.0).sum()

        # A step of zero has no epipolar geometry; it shows as no image motion.
        if not moved[i] and np.median(np.linalg.norm(a - b, axis=1)) * focal <= 2.0:
            steps.append(0.0); continue
        grid = np.linspace(0.2 * typical, 2.5 * typical, 24)
        best = grid[int(np.argmin([cost(g) for g in grid]))]
        steps.append(float(minimize_scalar(cost, bounds=(best - 0.1 * typical, best + 0.1 * typical), method="bounded").x))
    steps = np.array(steps)
    # A step far from the others is a failed search, not a jump of the turntable.
    steps[(steps > 1.6 * typical) | ((steps < 0.4 * typical) & (steps > 0))] = np.median(steps[steps > 0])
    closure = np.degrees(steps.sum())
    # A full turn closes: the steps, the one from the last photo back to the first included, add up to
    # 360 degrees. Angle per step and distance to the axis are hard to tell apart from one pair alone;
    # this fixes their product for the whole ring, and lets the matches across the seam be verified.
    if not args.open_turn and 0.85 < steps.sum() / (2 * np.pi) < 1.15:
        steps *= 2 * np.pi / steps.sum()
    print("steps deg: min %.2f median %.2f max %.2f; sum %.2f (a full turn is 360)" % (
        np.degrees(steps.min()), np.degrees(np.median(steps)), np.degrees(steps.max()), closure), flush=True)
    cumulative = np.concatenate([[0], np.cumsum(steps[:-1])])
    Rs = [step_motion(axis, centre, a)[0] for a in cumulative]
    ts = [step_motion(axis, centre, a)[1] for a in cumulative]

    def triangulate(track, Rs, ts):
        A = []
        for v, f in track:
            x, y = norms[v][f]
            P = np.c_[Rs[v], ts[v]]
            A += [x * P[2] - P[0], y * P[2] - P[1]]
        X = np.linalg.svd(np.array(A))[2][-1]
        return X[:3] / X[3]

    def build(Rs, ts, bound):
        """Matches that agree with the poses, joined into tracks and triangulated."""
        parent = {}

        def find(x):
            while parent.setdefault(x, x) != x:
                parent[x] = parent[parent[x]]
                x = parent[x]
            return x

        kept = 0
        for (i, j), m in pairs.items():
            if not len(m):
                continue
            R, t = Rs[j] @ Rs[i].T, ts[j] - Rs[j] @ Rs[i].T @ ts[i]
            if np.linalg.norm(t) < 1e-6:
                continue
            ok = np.sqrt(sampson(skew(t) @ R, norms[i][m[:, 0]], norms[j][m[:, 1]])) * focal < bound
            kept += int(ok.sum())
            for a, b in m[ok]:
                parent[find((i, int(a)))] = find((j, int(b)))
        groups = {}
        for node in list(parent):
            groups.setdefault(find(node), []).append(node)
        tracks = [g for g in groups.values() if len(g) >= 3 and len({v for v, _ in g}) == len(g)]
        points, obs = [], []
        for track in tracks:
            X = triangulate(track, Rs, ts)
            cam = [(Rs[v] @ X + ts[v]) for v, _ in track]
            if not np.isfinite(X).all() or min(c[2] for c in cam) <= 0:
                continue
            err = max(np.linalg.norm(c[:2] / c[2] - norms[v][f]) * focal for c, (v, f) in zip(cam, track))
            if err < 2 * bound:
                obs += [(v, len(points), f) for v, f in track]
                points.append(X)
        print("verified matches %d; tracks %d; points %d; observations %d" % (kept, len(tracks), len(points), len(obs)), flush=True)
        return np.array(points), np.array(obs, int)

    points, obs = build(Rs, ts, 4.0)

    # Bundle adjustment: Levenberg-Marquardt with the points eliminated (Schur complement), Huber
    # weights, all poses free except the first (gauge), lens fixed. Residuals in pixels.
    def adjust(Rs, ts, points, obs, iterations=40):
        Rs, ts, X = np.array(Rs), np.array(ts), points.copy()
        cam, pid = obs[:, 0], obs[:, 1]
        target = np.array([norms[v][f] for v, _, f in obs])
        m = len(X)

        def evaluate(Rs, ts, X):
            q = np.einsum("kij,kj->ki", Rs[cam], X[pid]) + ts[cam]
            r = (q[:, :2] / q[:, 2:3] - target) * focal
            e = np.linalg.norm(r, axis=1)
            cost = np.where(e <= 1.0, 0.5 * e * e, e - 0.5).sum()
            return q, r, e, cost

        q, r, e, cost = evaluate(Rs, ts, X)
        first, damping = cost, 1e-3
        for _ in range(iterations):
            weight = np.where(e <= 1.0, 1.0, 1.0 / np.maximum(e, 1e-12))
            iz = 1.0 / q[:, 2]
            D = np.zeros((len(obs), 2, 3))                       # d residual / d camera-frame point
            D[:, 0, 0] = D[:, 1, 1] = focal * iz
            D[:, 0, 2], D[:, 1, 2] = -focal * q[:, 0] * iz * iz, -focal * q[:, 1] * iz * iz
            rx = q - ts[cam]                                     # R X
            K_ = np.zeros((len(obs), 3, 3))                      # -[R X]x: d(exp(w) R X) / dw
            K_[:, 0, 1], K_[:, 0, 2], K_[:, 1, 0], K_[:, 1, 2], K_[:, 2, 0], K_[:, 2, 1] = rx[:, 2], -rx[:, 1], -rx[:, 2], rx[:, 0], rx[:, 1], -rx[:, 0]
            Jc = np.concatenate([D @ K_, D], axis=2)             # 2 x 6
            Jc[cam == 0] = 0
            Jp = D @ Rs[cam]                                     # 2 x 3
            wJc, wJp = Jc * weight[:, None, None], Jp * weight[:, None, None]
            U = np.zeros((n, 6, 6)); bc = np.zeros((n, 6)); V = np.zeros((m, 3, 3)); bp = np.zeros((m, 3))
            np.add.at(U, cam, np.einsum("kij,kil->kjl", wJc, Jc)); np.add.at(bc, cam, np.einsum("kij,ki->kj", wJc, r))
            np.add.at(V, pid, np.einsum("kij,kil->kjl", wJp, Jp)); np.add.at(bp, pid, np.einsum("kij,ki->kj", wJp, r))
            Wk = np.einsum("kij,kil->kjl", wJc, Jp)              # 6 x 3 per observation
            rows = (cam[:, None, None] * 6 + np.arange(6)[None, :, None]) + np.zeros((1, 1, 3), int)
            cols = (pid[:, None, None] * 3 + np.arange(3)[None, None, :]) + np.zeros((1, 6, 1), int)
            W = csr_matrix((Wk.ravel(), (rows.ravel(), cols.ravel())), shape=(6 * n, 3 * m))
            improved = False
            for _try in range(8):
                Vd = V + damping * np.einsum("kii->ki", V)[:, :, None] * np.eye(3) + 1e-12 * np.eye(3)
                Vinv = np.linalg.inv(Vd)
                Y = W @ bsr_matrix((Vinv, np.arange(m), np.arange(m + 1)), shape=(3 * m, 3 * m))
                Ud = np.zeros((6 * n, 6 * n))
                for c in range(n):
                    Ud[6 * c:6 * c + 6, 6 * c:6 * c + 6] = U[c] + damping * np.diag(np.diag(U[c])) + 1e-9 * np.eye(6)
                S = Ud - (Y @ W.T).toarray()
                rhs = -(bc.ravel() - Y @ bp.ravel())
                dc = np.linalg.solve(S, rhs).reshape(n, 6)
                dp = -np.einsum("kij,kj->ki", Vinv, bp + (W.T @ dc.ravel()).reshape(m, 3))
                Rn = Rotation.from_rotvec(dc[:, :3]).as_matrix() @ Rs
                tn = ts + dc[:, 3:]
                # exp(w) (R X + t) = exp(w) R X + exp(w) t: keep the update on R X only, as differentiated.
                qn, rn, en, cn = evaluate(Rn, tn, X + dp)
                if np.isfinite(cn) and cn < cost:
                    gain = cost - cn
                    Rs, ts, X, q, r, e, cost = Rn, tn, X + dp, qn, rn, en, cn
                    damping = max(damping / 3, 1e-9)
                    improved = gain > 1e-6 * cost
                    break
                damping *= 10
            if not improved:
                break
        print("  adjustment: cost %.1f -> %.1f" % (first, cost), flush=True)
        return list(Rs), list(ts), X, r

    for round_ in range(4):
        Rs, ts, points, r = adjust(Rs, ts, points, obs)
        e = np.linalg.norm(r, axis=1)
        print("adjustment %d: observations %d, reprojection median %.3f p95 %.3f px" % (round_ + 1, len(obs), np.median(e), np.percentile(e, 95)), flush=True)
        if round_ == 3:
            break
        if round_ == 0:
            # With adjusted poses more matches pass, also across the seam of the turn.
            points, obs = build(Rs, ts, 3.0)
            continue
        keep = e < max(2.0, 3.0 * np.median(e))
        obs = obs[keep]
        count = np.bincount(obs[:, 1], minlength=len(points))
        good = count >= 2
        remap = np.cumsum(good) - 1
        obs = obs[good[obs[:, 1]]]
        obs[:, 1] = remap[obs[:, 1]]
        points = points[good]
    t_total = time.time() - started

    out = args.output
    out.mkdir(parents=True, exist_ok=True)
    params = [K[0, 0], K[1, 1], K[0, 2] + 0.5, K[1, 2] + 0.5, dist[0], dist[1], 0, 0, dist[4], 0, 0, 0]
    (out / "cameras.txt").write_text("# crispmesh turntable prototype\n1 FULL_OPENCV %d %d %s\n" % (w, h, " ".join(repr(float(v)) for v in params)))
    by_view = {}
    for v, pid, f in obs:
        by_view.setdefault(int(v), []).append((int(f), int(pid)))
    lines = ["# crispmesh turntable prototype"]
    for v, name in enumerate(names):
        q = Rotation.from_matrix(Rs[v]).as_quat()  # x, y, z, w
        lines.append("%d %r %r %r %r %r %r %r 1 %s" % (v + 1, q[3], q[0], q[1], q[2], *map(float, ts[v]), name))
        lines.append(" ".join("%r %r %d" % (keys[v][f][0] + 0.5, keys[v][f][1] + 0.5, pid + 1) for f, pid in by_view.get(v, [])))
    (out / "images.txt").write_text("\n".join(lines) + "\n")
    (out / "points3D.txt").write_text("# crispmesh turntable prototype\n" + "".join("%d %r %r %r 128 128 128 0\n" % (i + 1, *map(float, X)) for i, X in enumerate(points)))
    report = {"photos": n, "points": len(points), "observations": len(obs), "steps_deg": np.degrees(steps).round(3).tolist(), "turn_sum_deg": closure,
              "seconds": {"features": t_features, "matching": t_matching, "total": t_total},
              "reprojection_px": {"median": float(np.median(e)), "p95": float(np.percentile(e, 95))}}
    (out / "prototype.json").write_text(json.dumps(report, indent=1) + "\n")
    print(json.dumps({k: report[k] for k in ("photos", "points", "observations", "turn_sum_deg", "seconds", "reprojection_px")}))


if __name__ == "__main__":
    main()
