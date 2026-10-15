"""DTU MVS evaluation of a crispmesh mesh (development tool, evaluation only).

Follows the official DTU protocol as ported by DTUeval-python: the mesh is sampled to points
about 0.2 mm apart; accuracy is the distance from those points (inside the scan's observability
mask) to the structured-light points, completeness the distance from the structured-light points
above the table plane to the mesh points, both averaged over distances below 20 mm; the overall
score is their mean, in millimetres. F-scores at 1 and 2 mm are added for comparison with the
scanner F1 used elsewhere.

The mesh must be in DTU's world frame (millimetres): a run on supplied DTU poses is. A run whose
cameras were recovered (any other provider) is mapped into it first by the similarity that best
maps its camera centres onto DTU's (`--align OURS_CAMERAS DTU_CAMERAS`, matched by `source`).

usage: python -m scripts.turntable_mesh.dtu_evaluate --mesh mesh.stl --scan 24 --dtu DIR --output OUT
       [--align ours/cameras.json dtu-inputs/cameras.json]
DIR holds Points/stl/stlNNN_total.ply and "SampleSet/MVS Data/ObsMask/{ObsMaskN_10,PlaneN}.mat".
"""
import argparse, json, pathlib

import numpy as np
from scipy.io import loadmat
from scipy.spatial import cKDTree

from scripts.turntable_mesh.scan_evaluate import read_binary_stl, umeyama, apply


def read_points(path):
    """Vertices of a binary or ASCII PLY point cloud (x, y, z first)."""
    data = pathlib.Path(path).read_bytes()
    end = data.index(b"end_header") + len(b"end_header")
    end += 2 if data[end:end + 2] == b"\r\n" else 1
    header = data[:end].decode("ascii", "replace").splitlines()
    count, props, binary = 0, [], "binary_little_endian" in header[1]
    in_vertex = False
    for line in header:
        parts = line.split()
        if parts[:2] == ["element", "vertex"]:
            count, in_vertex = int(parts[2]), True
        elif parts[:1] == ["element"]:
            in_vertex = False
        elif parts[:1] == ["property"] and in_vertex:
            props.append((parts[-1], parts[1]))
    types = {"float": "<f4", "float32": "<f4", "double": "<f8", "uchar": "u1", "uint8": "u1", "int": "<i4", "short": "<i2", "ushort": "<u2"}
    if binary:
        dtype = np.dtype([(name, types[kind]) for name, kind in props])
        vertices = np.frombuffer(data, dtype, count, end)
    else:
        vertices = np.loadtxt(data[end:].decode().splitlines()[:count], usecols=range(len(props)))
        return vertices[:, :3].astype(np.float64)
    return np.stack([vertices["x"], vertices["y"], vertices["z"]], 1).astype(np.float64)


def sample_mesh(triangles, spacing, rng, cap=4_000_000):
    a, b, c = triangles[:, 0], triangles[:, 1], triangles[:, 2]
    area = 0.5 * np.linalg.norm(np.cross(b - a, c - a), axis=1)
    count = int(min(cap, max(10_000, area.sum() / spacing ** 2)))
    pick = rng.choice(len(triangles), count, p=area / area.sum())
    u, v = rng.random(count), rng.random(count)
    flip = u + v > 1
    u[flip], v[flip] = 1 - u[flip], 1 - v[flip]
    points = a[pick] + u[:, None] * (b[pick] - a[pick]) + v[:, None] * (c[pick] - a[pick])
    # Thin to one point per spacing-sized voxel, as the reference downsampling does.
    keys = np.floor(points / spacing).astype(np.int64)
    _, first = np.unique(keys, axis=0, return_index=True)
    return points[np.sort(first)]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--mesh", required=True)
    parser.add_argument("--scan", type=int, required=True)
    parser.add_argument("--dtu", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--align", nargs=2, metavar=("OURS", "DTU"))
    parser.add_argument("--spacing", type=float, default=0.2)
    parser.add_argument("--max-distance", type=float, default=20.0)
    args = parser.parse_args()
    out = pathlib.Path(args.output)
    out.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(0)
    dtu = pathlib.Path(args.dtu)

    triangles = read_binary_stl(args.mesh)
    alignment = None
    if args.align:
        ours = {v["source"]: v for v in json.load(open(args.align[0]))["views"]}
        theirs = {v["source"]: v for v in json.load(open(args.align[1]))["views"]}
        centre = lambda v: -np.array(v["rotation"]).T @ np.array(v["translation"])
        names = sorted(set(ours) & set(theirs))
        if names:
            pairs = [(ours[n], theirs[n]) for n in names]
        else:
            # A run from photos names its views by capture (capture_NNNN.png, in the natural order of
            # the photo names); DTU's are the photo names themselves, in the same order.
            order = lambda table: [table[k] for k in sorted(table)]
            if len(ours) != len(theirs):
                raise SystemExit(f"--align: {len(ours)} views against {len(theirs)} and no common names")
            pairs = list(zip(order(ours), order(theirs)))
            names = [b["source"] for _, b in pairs]
        source = np.array([centre(a) for a, _ in pairs])
        target = np.array([centre(b) for _, b in pairs])
        transform = umeyama(source, target)
        residual = np.linalg.norm(apply(transform, source) - target, axis=1)
        triangles = apply(transform, triangles.reshape(-1, 3)).reshape(-1, 3, 3)
        alignment = {"cameras": len(names), "scale": float(transform[0]), "centre_residual_mm": {"median": float(np.median(residual)), "max": float(residual.max())}}

    data = sample_mesh(triangles, args.spacing, rng)
    obs = loadmat(dtu / "SampleSet/MVS Data/ObsMask" / f"ObsMask{args.scan}_10.mat")
    mask, bounds, resolution = obs["ObsMask"], obs["BB"], float(obs["Res"])
    index = np.rint((data - bounds[0:1]) / resolution).astype(np.int64)
    inside = np.all((index >= 0) & (index < np.array(mask.shape)), axis=1)
    observed = np.zeros(len(data), bool)
    observed[inside] = mask[index[inside, 0], index[inside, 1], index[inside, 2]].astype(bool)
    data_in = data[observed]

    stl = read_points(dtu / "Points/stl" / f"stl{args.scan:03d}_total.ply")
    plane = loadmat(dtu / "SampleSet/MVS Data/ObsMask" / f"Plane{args.scan}.mat")["P"].reshape(4)
    above = (np.concatenate([stl, np.ones((len(stl), 1))], 1) @ plane) > 0
    stl_above = stl[above]

    d2s = cKDTree(stl).query(data_in, workers=4)[0]
    s2d = cKDTree(data).query(stl_above, workers=4)[0]
    limit = args.max_distance
    accuracy = float(d2s[d2s < limit].mean())
    completeness = float(s2d[s2d < limit].mean())
    result = {
        "schema": "dtu_evaluate_v1", "evaluation_only": True, "scan": args.scan,
        "accuracy_mm": accuracy, "completeness_mm": completeness, "overall_mm": (accuracy + completeness) / 2,
        "fscore": {f"{t}mm": {"precision": float((d2s < t).mean()), "recall": float((s2d < t).mean())} for t in (1.0, 2.0)},
        "mesh_points": int(len(data)), "mesh_points_observed": int(len(data_in)), "scan_points_above_plane": int(len(stl_above)),
        "alignment": alignment,
    }
    for t in result["fscore"].values():
        p, r = t["precision"], t["recall"]
        t["f1"] = 2 * p * r / (p + r) if p + r else 0.0
    json.dump(result, open(out / "result.json", "w"), indent=1)
    print(f"DTU scan {args.scan}: accuracy {accuracy:.3f} mm, completeness {completeness:.3f} mm, overall {(accuracy + completeness) / 2:.3f} mm, "
          f"F1 1mm {result['fscore']['1.0mm']['f1']:.3f} 2mm {result['fscore']['2.0mm']['f1']:.3f}" + (f", aligned on {alignment['cameras']} cameras (residual median {alignment['centre_residual_mm']['median']:.2f} mm)" if alignment else ""))


if __name__ == "__main__":
    main()
