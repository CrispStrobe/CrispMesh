#!/usr/bin/env python3
"""Fetch only pinned Sceaux sample inputs and regression intermediates."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
COMMIT = "8855e0405b2c80c8286338ff21f3c09f22e9be16"
BASE = "https://raw.githubusercontent.com/cdcseacave/openMVS_sample/" + COMMIT + "/"
API = "https://api.github.com/repos/cdcseacave/openMVS_sample/"
OUTPUT = ROOT / ".local-tools/upstream-control/openmvs-sceaux-v23"
RESERVE = 10 * 1024 ** 3
DOWNLOAD_CAP = 150 * 1024 ** 2
FILE_CAP = 50 * 1024 ** 2
SELECTED = tuple([f"images/{i:05}.jpg" for i in range(11)] + [
    "scene.mvs", "scene_dense.mvs", "scene_dense.ply", "scene_dense_mesh.ply",
    "DensifyPointCloud-2407252206440034CD.log",
    "ReconstructMesh-240725221111003165.log", "README.md", "LICENSE"])


def sha256(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def git_blob(path):
    path = Path(path)
    h = hashlib.sha1()
    h.update(f"blob {path.stat().st_size}\0".encode("ascii"))
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def read_json(url):
    request = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json",
                                                 "User-Agent": "crispmesh-upstream-control"})
    with urllib.request.urlopen(request, timeout=20) as response:
        payload = response.read(2 * 1024 ** 2 + 1)
    if len(payload) > 2 * 1024 ** 2:
        raise ValueError("oversized GitHub metadata")
    return json.loads(payload)


def selected_tree():
    commit = read_json(API + "commits/" + COMMIT)
    if commit.get("sha") != COMMIT:
        raise ValueError("upstream commit differs")
    tree_sha = commit["commit"]["tree"]["sha"]
    tree = read_json(API + "git/trees/" + tree_sha + "?recursive=1")
    if tree.get("sha") != tree_sha or tree.get("truncated"):
        raise ValueError("upstream tree differs or is truncated")
    by_path = {item["path"]: item for item in tree["tree"]}
    if any(path not in by_path for path in SELECTED):
        raise ValueError("missing selected upstream member")
    selected = {path: by_path[path] for path in SELECTED}
    for path, entry in selected.items():
        if (entry.get("type") != "blob" or not isinstance(entry.get("size"), int) or
                not 0 < entry["size"] <= FILE_CAP or len(entry.get("sha", "")) != 40):
            raise ValueError(f"unexpected upstream member metadata: {path}")
    if sum(entry["size"] for entry in selected.values()) > DOWNLOAD_CAP:
        raise ValueError("selected upstream files exceed download cap")
    return tree_sha, selected


def fetch(output=OUTPUT):
    output = Path(output)
    if output.exists() or output.is_symlink():
        raise FileExistsError(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(output.parent).free < RESERVE + DOWNLOAD_CAP:
        raise RuntimeError("upstream fetch would violate 10 GiB reserve")
    tree_sha, selected = selected_tree()
    output.mkdir()
    records = []
    try:
        for path in SELECTED:
            entry = selected[path]
            target = output / path
            target.parent.mkdir(parents=True, exist_ok=True)
            url = BASE + urllib.parse.quote(path, safe="/")
            request = urllib.request.Request(url, headers={"User-Agent": "crispmesh-upstream-control"})
            with urllib.request.urlopen(request, timeout=30) as response, target.open("xb") as stream:
                count = 0
                while block := response.read(1 << 20):
                    count += len(block)
                    if count > entry["size"] or shutil.disk_usage(output).free < RESERVE:
                        raise ValueError("download exceeded pinned size or disk reserve")
                    stream.write(block)
            if count != entry["size"] or git_blob(target) != entry["sha"]:
                raise ValueError(f"upstream Git blob differs: {path}")
            records.append({"path": path, "bytes": count, "git_blob_sha1": entry["sha"],
                            "sha256": sha256(target), "url": url})
        manifest = {"schema": "upstream_openmvs_sample_v23_v1", "commit": COMMIT,
                    "tree_sha1": tree_sha, "source": "cdcseacave/openMVS_sample",
                    "scope": "local research stage oracle; scene.mvs has upstream supplied image-derived poses; not ground truth or same-input SfM",
                    "photo_rights": "Original photos credit Copyright 2012 Pierre Moulon; no separate photo redistribution license verified",
                    "files": records, "total_bytes": sum(r["bytes"] for r in records)}
        (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        return manifest
    except BaseException as exc:
        (output / "failure.json").write_text(json.dumps({"status": "incomplete", "error": str(exc),
            "completed_files": len(records)}, indent=2) + "\n")
        raise


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args()
    result = fetch(args.output)
    print(json.dumps({"files": len(result["files"]), "total_bytes": result["total_bytes"],
                      "output": str(args.output)}))


if __name__ == "__main__":
    main()
