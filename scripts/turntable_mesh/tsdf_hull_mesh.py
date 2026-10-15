"""Extract a closed surface from fused TSDF evidence bounded by the silhouette hull.

Observed voxels use the confidence-smoothed averaged truncated signed distance.
Unobserved hull voxels within a few voxels of evidence take its extrapolation;
the rest fall back to the hull's own signed distance, so the surface there is
the silhouette bound. Nothing outside the hull can be surface. Marching cubes runs on the float field.
"""

import argparse
import json
from pathlib import Path
import struct
import time

import numpy as np
from scipy import ndimage, sparse
from scipy.sparse.csgraph import connected_components
from skimage import measure

from .dense_config import DenseConfig, add_arguments, build
from .dense_events import EventLog


def field(volume, config):
    shape = tuple(int(v) for v in volume["shape"])
    index = volume["index"]
    truncation = float(volume["truncation"]) / float(volume["voxel"])
    pad = 3
    padded = tuple(s + 2 * pad for s in shape)
    hull = np.zeros(padded, bool)
    where = tuple(index[:, a] + pad for a in range(3))
    hull[where] = True
    signed = ndimage.distance_transform_edt(~hull) - ndimage.distance_transform_edt(hull)
    prior = np.clip((signed + np.where(signed > 0, -0.5, 0.5)) / truncation, -1, 1).astype(np.float32)
    del signed
    weight = np.zeros(padded, np.float32)
    total = np.zeros(padded, np.float32)
    weight[where] = volume["weight"]
    total[where] = volume["total"]
    observed = weight >= config.mesh_minimum_weight
    average = np.where(observed, total / np.maximum(weight, 1e-6), 0).astype(np.float32)
    # Confidence saturates at a few views; one stray vote must not outweigh them.
    cap = config.mesh_confidence_cap
    confidence = np.where(observed, np.minimum(weight, cap) / cap, 0).astype(np.float32)
    del weight, total
    value = prior.copy()
    filled = np.zeros(padded, bool)
    # Confidence-weighted smoothing where observed, then bounded extrapolation of
    # the observed field into nearby unobserved hull voxels. Whatever is still
    # unfilled keeps the silhouette prior.
    for sigma in (config.mesh_smooth, *config.mesh_fill_sigmas):
        numerator = ndimage.gaussian_filter(average * confidence, sigma)
        denominator = ndimage.gaussian_filter(confidence, sigma)
        take = hull & ~filled & (denominator > 0.02)
        value[take] = numerator[take] / denominator[take]
        filled |= take
    del numerator, denominator
    value = np.maximum(value, prior).astype(np.float32)
    base = {"flat_base_applied": False}
    if config.mesh_flat_base and "support_height" in volume and np.isfinite(volume["support_height"]):
        # Signed distance to the support plane, in truncation units, outside below it.
        voxel = float(volume["voxel"])
        down = volume["support_down"].astype(np.float64)
        offset = (volume["origin"] - volume["support_point"]) @ down - float(volume["support_height"])
        axes = [((np.arange(n) - pad + 0.5) * voxel * d).astype(np.float32) for n, d in zip(padded, down)]
        depth_below = axes[0][:, None, None] + axes[1][None, :, None] + axes[2][None, None, :] + np.float32(offset)
        plane = np.clip((depth_below / voxel - config.mesh_base_margin) / truncation, -1, 1)
        base = {"flat_base_applied": True, "hull_fraction_below_support": float((plane[where] > 0).mean())}
        value = np.maximum(value, plane)
        del depth_below, plane
    # The maximum leaves creases that give marching cubes ambiguous cells.
    if config.mesh_final_smooth:
        value = ndimage.gaussian_filter(value, config.mesh_final_smooth)
    return value, pad, {**base, "observed_hull_fraction": float(observed[where].mean()),
                        "extrapolated_hull_fraction": float((filled & ~observed)[where].mean())}


def taubin(vertices, faces, cycles, lam=0.5, mu=-0.53):
    n = len(vertices)
    edges = np.concatenate((faces[:, [0, 1]], faces[:, [1, 2]], faces[:, [2, 0]]))
    adjacency = sparse.coo_matrix((np.ones(len(edges)), (edges[:, 0], edges[:, 1])), (n, n)).tocsr()
    adjacency = ((adjacency + adjacency.T) > 0).astype(np.float64)
    degree = np.asarray(adjacency.sum(1)).ravel()
    operator = sparse.diags(1 / np.maximum(degree, 1)) @ adjacency
    v = vertices.astype(np.float64)
    for _ in range(cycles):
        v = v + lam * (operator @ v - v)
        v = v + mu * (operator @ v - v)
    return v


def largest_component(vertices, faces):
    n = len(vertices)
    graph = sparse.coo_matrix((np.ones(len(faces) * 2), (np.r_[faces[:, 0], faces[:, 1]], np.r_[faces[:, 1], faces[:, 2]])), (n, n))
    count, labels = connected_components(graph, directed=False)
    sizes = np.bincount(labels[faces[:, 0]], minlength=count)
    keep = labels[faces[:, 0]] == sizes.argmax()
    faces = faces[keep]
    used = np.unique(faces)
    remap = np.full(n, -1)
    remap[used] = np.arange(len(used))
    return vertices[used], remap[faces], {"components": int(count), "discarded_faces": int((~keep).sum())}


def topology(vertices, faces):
    edges = np.sort(np.concatenate((faces[:, [0, 1]], faces[:, [1, 2]], faces[:, [2, 0]])), 1)
    _, counts = np.unique(edges, axis=0, return_counts=True)
    euler = len(vertices) - len(counts) + len(faces)
    a, b, c = (vertices[faces[:, i]] for i in range(3))
    return {"vertices": len(vertices), "triangles": len(faces), "boundary_edges": int((counts == 1).sum()),
            "nonmanifold_edges": int((counts > 2).sum()), "genus": int(round((2 - euler) / 2)),
            "signed_volume": float(np.einsum("ij,ij->i", a, np.cross(b, c)).sum() / 6)}


def write_stl(path, vertices, faces):
    triangles = vertices[faces].astype("<f4")
    normals = np.cross(triangles[:, 1] - triangles[:, 0], triangles[:, 2] - triangles[:, 0])
    normals /= np.maximum(np.linalg.norm(normals, axis=1, keepdims=True), 1e-20)
    record = np.zeros(len(faces), dtype=[("normal", "<f4", (3,)), ("vertices", "<f4", (3, 3)), ("attr", "<u2")])
    record["normal"], record["vertices"] = normals, triangles
    with open(path, "xb") as stream:
        stream.write(b"crispmesh tsdf hull mesh".ljust(80, b" "))
        stream.write(struct.pack("<I", len(faces)))
        stream.write(record.tobytes())


def run(volume_path, output, config=None, *, step=1, events=None, label=None):
    """``step`` > 1 extracts a coarse preview: fewer triangles, no mesh smoothing."""
    config = (config or DenseConfig()).validate()
    events = events or EventLog(None)
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    volume = np.load(volume_path)
    value, pad, report = field(volume, config)
    voxel = float(volume["voxel"])
    vertices, faces, _, _ = measure.marching_cubes(value, 0.0, step_size=step)
    del value
    vertices = (vertices - pad + 0.5) * voxel + volume["origin"]
    faces = faces.astype(np.int64)
    vertices, faces, parts = largest_component(vertices, faces)
    if config.mesh_taubin_cycles and step == 1:
        vertices = taubin(vertices, faces, config.mesh_taubin_cycles)
    info = topology(vertices, faces)
    if info["signed_volume"] < 0:
        faces = faces[:, ::-1]
        info = topology(vertices, faces)
    write_stl(output / "mesh.stl", vertices, faces)
    report.update(parts, **info, closed=info["boundary_edges"] == 0 and info["nonmanifold_edges"] == 0,
                  configuration={k: v for k, v in config.to_json().items() if k.startswith("mesh_")},
                  reference_used=False, physical_scale_established=False, seconds=time.monotonic() - started)
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    events.artifact("final_mesh" if step == 1 else "preview_mesh", output / "mesh.stl",
                    label or ("Final surface" if step == 1 else "Preview surface"), triangles=info["triangles"])
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--volume", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--step", type=int, default=1, help="marching-cubes step; above 1 gives a coarse preview")
    parser.add_argument("--events", type=Path)
    parser.add_argument("--label")
    add_arguments(parser)
    args = parser.parse_args()
    print(json.dumps(run(args.volume, args.output, build(args.config, args.set), step=args.step,
                         events=EventLog(args.events, "mesh" if args.step == 1 else "stereo"), label=args.label), indent=2))


if __name__ == "__main__":
    main()
