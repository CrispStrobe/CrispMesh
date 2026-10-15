"""Evaluation-only, TRAIN-only SIFT/RootSIFT versus LightGlue six-pair pilot."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import time

import cv2
import numpy as np
import torch
from lightglue import LightGlue

from scripts.object_motion.mustard_two_view_pose import fit_pair


ROOT = Path(__file__).resolve().parents[2]
DATA = Path("/Volumes/backups/code/crispmesh-data")
STAGE = DATA / "mustard-sfm-train-001"
OUT = DATA / "mustard-lightglue-six-pair-001"
AI = Path("/Volumes/backups/ai/crispmesh-lightglue-sift-001")
WEIGHT = AI / "weights/sift_lightglue.pth"
STAGE_SHA = "bbcba624f6a51f82e0ebdebf6e7e4cda10804bc8df34cfd0d5b3242983a225d0"
NAMES_SHA = "a81d1647109db27991c39c5fe1a9ae308c04081f8ada49d8b0f5ab1c15245544"
WEIGHT_SHA = "5b52b8d9982d43532dc042606b346bb9594c9f5a4bd6f64362c63866287b4ac0"
PAIRS = (
    ("near", "NP3_006.jpg", "NP3_012.jpg", 1),
    ("near", "NP3_012.jpg", "NP3_018.jpg", 1),
    ("middle", "NP3_012.jpg", "NP3_336.jpg", 5),
    ("middle", "NP3_036.jpg", "NP3_126.jpg", 12),
    ("opposing", "NP3_030.jpg", "NP3_198.jpg", 23),
    ("opposing", "NP3_036.jpg", "NP3_222.jpg", 23),
)
MIN_FREE = 11 * 1024**3
MAX_OUTPUT = 512 * 1024**2
MAX_AI = 512 * 1024**2 + 64 * 1024**2
MAX_SECONDS = 600


def digest(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            result.update(chunk)
    return result.hexdigest()


def folder_size(path: Path) -> int:
    return sum(file.stat().st_size for file in path.rglob("*") if file.is_file())


def guard() -> dict[str, int]:
    free = {"workspace": shutil.disk_usage(ROOT).free,
            "backup": shutil.disk_usage(DATA).free}
    if min(free.values()) < MIN_FREE:
        raise RuntimeError("both disks require >11 GiB free throughout pilot")
    if folder_size(OUT) > MAX_OUTPUT or folder_size(AI) > MAX_AI:
        raise RuntimeError("pilot output or AI staging cap exceeded")
    return free


def validate_panel(names: list[str]) -> None:
    if len(names) != 48 or len(set(names)) != 48 or not {n for _, a, b, _ in PAIRS for n in (a, b)} <= set(names):
        raise ValueError("frozen six pairs outside sealed 48 TRAIN names")


def preflight() -> tuple[dict, list[str], dict]:
    if (not OUT.is_dir() or OUT.is_symlink() or list(OUT.iterdir()) or
            STAGE.is_symlink() or AI.is_symlink()):
        raise ValueError("pilot output must be a fresh, empty real directory")
    free = guard()
    if digest(STAGE / "stage-report.json") != STAGE_SHA or digest(STAGE / "train-names.txt") != NAMES_SHA:
        raise ValueError("sealed TRAIN stage changed")
    stage = json.loads((STAGE / "stage-report.json").read_text())
    names = (STAGE / "train-names.txt").read_text().splitlines()
    validate_panel(names)
    if digest(WEIGHT) != WEIGHT_SHA or WEIGHT.stat().st_size != 47_632_573:
        raise ValueError("pinned SIFT LightGlue weight changed")
    expected = {}
    for name in sorted({n for _, a, b, _ in PAIRS for n in (a, b)}):
        image, mask = STAGE / "images" / name, STAGE / "masks" / (name + ".png")
        if (image.is_symlink() or mask.is_symlink() or
                digest(image) != stage["train_photo_sha256"][name] or
                digest(mask) != stage["cleaned_masks"][name]["sha256"]):
            raise ValueError(f"sealed TRAIN image/mask changed: {name}")
        expected[name] = {"image": digest(image), "mask": digest(mask)}
    return free, names, expected


def rootsift(descriptors: np.ndarray) -> np.ndarray:
    result = descriptors.astype(np.float32, copy=True)
    result /= np.maximum(np.sum(np.abs(result), axis=1, keepdims=True), 1e-6)
    np.maximum(result, 1e-6, out=result)
    np.sqrt(result, out=result)
    result /= np.maximum(np.linalg.norm(result, axis=1, keepdims=True), 1e-6)
    return result


def extract(name: str) -> Path:
    image = cv2.imread(str(STAGE / "images" / name), cv2.IMREAD_GRAYSCALE)
    mask = cv2.imread(str(STAGE / "masks" / (name + ".png")), cv2.IMREAD_GRAYSCALE)
    if image is None or mask is None or image.shape != (1024, 1280) or mask.shape != image.shape:
        raise ValueError(f"unexpected image or feature mask: {name}")
    scale = 1200 / max(image.shape)
    resized = cv2.resize(image, (1200, 960), interpolation=cv2.INTER_AREA)
    resized_mask = cv2.resize(mask, (1200, 960), interpolation=cv2.INTER_NEAREST)
    detector = cv2.SIFT_create(nfeatures=1800, contrastThreshold=0.0066667,
                               edgeThreshold=10, nOctaveLayers=4)
    keys, descriptors = detector.detectAndCompute(resized, (resized_mask > 0).astype(np.uint8) * 255)
    if descriptors is None or len(keys) < 20:
        raise ValueError(f"insufficient masked SIFT features: {name}")
    # OpenCV locates the first pixel at integer (0,0); COLMAP's center is (0.5,0.5).
    xy = np.asarray([key.pt for key in keys], np.float32) / scale + np.float32(0.5)
    sizes = np.asarray([key.size for key in keys], np.float32) / scale
    angles = np.deg2rad(np.asarray([key.angle for key in keys], np.float32)).astype(np.float32)
    desc = rootsift(descriptors)
    if (len(keys) > 1800 or desc.shape != (len(keys), 128) or
            not all(np.isfinite(array).all() for array in (xy, sizes, angles, desc)) or
            not np.allclose(np.linalg.norm(desc, axis=1), 1, atol=1e-4) or
            np.any(xy < 0) or np.any(xy >= [1280, 1024])):
        raise ValueError(f"feature format failed: {name}")
    path = OUT / (name + ".npz")
    np.savez_compressed(path, keypoints=xy, scales=sizes, oris=angles,
                        descriptors=desc, image_size=np.asarray([1280, 1024], np.float32))
    return path


def read_feature(path: Path, expected_sha: str | None = None) -> dict[str, np.ndarray]:
    if expected_sha is not None and digest(path) != expected_sha:
        raise ValueError(f"shared feature artifact changed: {path}")
    with np.load(path, allow_pickle=False) as saved:
        output = {key: saved[key].copy() for key in saved.files}
    count = len(output["keypoints"])
    if (output["keypoints"].shape != (count, 2) or output["scales"].shape != (count,) or
            output["oris"].shape != (count,) or output["descriptors"].shape != (count, 128) or
            output["image_size"].tolist() != [1280, 1024] or
            any(array.dtype != np.float32 or not np.isfinite(array).all() for array in output.values())):
        raise ValueError(f"serialized feature contract failed: {path}")
    return output


def nearest(left: np.ndarray, right: np.ndarray) -> np.ndarray:
    # Unit RootSIFT: squared Euclidean distance is 2 - 2 cosine similarity.
    distance = np.maximum(2 - 2 * (left @ right.T), 0)
    nearest_two = np.argpartition(distance, kth=1, axis=1)[:, :2]
    rank = np.take_along_axis(distance, nearest_two, axis=1)
    swap = rank[:, 0] > rank[:, 1]
    nearest_two[swap] = nearest_two[swap, ::-1]
    rank[swap] = rank[swap, ::-1]
    ratio_pass = rank[:, 0] < 0.8**2 * rank[:, 1]
    return np.where(ratio_pass, nearest_two[:, 0], -1)


def classical(a: dict, b: dict) -> np.ndarray:
    forward = nearest(a["descriptors"], b["descriptors"])
    reverse = nearest(b["descriptors"], a["descriptors"])
    left = np.flatnonzero(forward >= 0)
    right = forward[left]
    keep = reverse[right] == left
    return np.column_stack((left[keep], right[keep])).astype(np.uint32)


def torch_feature(item: dict) -> dict:
    return {name: torch.from_numpy(value[None]).to("cpu") for name, value in item.items()}


def learned(model: LightGlue, a: dict, b: dict) -> np.ndarray:
    with torch.inference_mode():
        result = model({"image0": torch_feature(a), "image1": torch_feature(b)})
    matches = result["matches"][0].cpu().numpy().astype(np.uint32)
    if matches.ndim != 2 or matches.shape[1] != 2:
        raise ValueError("LightGlue returned invalid match shape")
    return matches


def validate_matches(matches: np.ndarray, left_count: int, right_count: int) -> None:
    if (matches.ndim != 2 or matches.shape[1] != 2 or matches.dtype != np.uint32 or
            len(np.unique(matches, axis=0)) != len(matches) or
            np.any(matches[:, 0] >= left_count) or np.any(matches[:, 1] >= right_count)):
        raise ValueError("invalid or duplicate matches")


def load_model() -> LightGlue:
    if digest(WEIGHT) != WEIGHT_SHA or WEIGHT.stat().st_size != 47_632_573:
        raise ValueError("pinned SIFT LightGlue weight changed")
    state = torch.load(WEIGHT, map_location="cpu", weights_only=True)
    model = LightGlue(features=None, input_dim=128, add_scale_ori=True, weights=None,
                      flash=False, filter_threshold=0.1, depth_confidence=0.95,
                      width_confidence=0.99).eval().cpu()
    loading = model.load_state_dict(state, strict=False)
    if loading.missing_keys != ["confidence_thresholds"] or loading.unexpected_keys:
        raise ValueError("model weight keys differ from pinned architecture")
    return model


def fit(stratum: str, left: str, right: str, separation: int,
        a: dict, b: dict, matches: np.ndarray, order: str) -> dict:
    if order == "canonical":
        matches = matches[np.lexsort((matches[:, 1], matches[:, 0]))]
    elif order == "reverse":
        matches = matches[::-1].copy()
    points_a = (a["keypoints"][matches[:, 0]] - [640, 512]) / 1536
    points_b = (b["keypoints"][matches[:, 1]] - [640, 512]) / 1536
    row = {"stratum": stratum, "left": left, "right": right,
           "cyclic_separation": separation, "verified_correspondences": len(matches),
           "two_view_config": None, "points_left": points_a.astype(np.float64),
           "points_right": points_b.astype(np.float64)}
    if len(matches) < 20:
        return {"status": "unavailable", "reason": "fewer than 20 matches", "matches": len(matches)}
    return fit_pair(row)


def run() -> dict:
    free, names, sources = preflight()
    torch.set_num_threads(2)
    cv2.setNumThreads(1)
    model = load_model()
    started = time.monotonic()
    features = {}
    for name in sources:
        guard()
        artifact = extract(name)
        features[name] = {"path": str(artifact), "sha256": digest(artifact),
                          "count": len(read_feature(artifact, digest(artifact))["keypoints"])}
    report = {"schema": "mustard_lightglue_six_pair_v1", "status": "running",
              "scope": "nine sealed TRAIN photos; OpenCV SIFT + RootSIFT approximation; no held-out/reference",
              "free_before": free, "input_hashes": sources, "stage_report_sha256": STAGE_SHA,
              "weight_sha256": WEIGHT_SHA, "features": features, "pairs": [],
              "config": {"resize_max": 1200, "sift_nfeatures": 1800,
                         "nn_ratio": 0.8, "mutual": True, "lightglue_filter": 0.1,
                         "lightglue_depth": 0.95, "lightglue_width": 0.99,
                         "orders": ["as_returned", "canonical", "reverse"],
                         "seed_set": [17, 23, 31], "camera_assumption": [1536, 640, 512, 0]},
              "software": {"torch": torch.__version__, "opencv": cv2.__version__,
                           "numpy": np.__version__}}
    for stratum, left, right, separation in PAIRS:
        guard()
        a = read_feature(Path(features[left]["path"]), features[left]["sha256"])
        b = read_feature(Path(features[right]["path"]), features[right]["sha256"])
        cases = {}
        for label, matches in (("classical", classical(a, b)), ("lightglue", learned(model, a, b))):
            validate_matches(matches, len(a["keypoints"]), len(b["keypoints"]))
            cases[label] = {"matches": len(matches),
                            "geometry": {order: fit(stratum, left, right, separation, a, b, matches, order)
                                         for order in ("as_returned", "canonical", "reverse")}}
        report["pairs"].append({"stratum": stratum, "left": left, "right": right,
                                "cyclic_separation": separation, "arms": cases})
        if time.monotonic() - started > MAX_SECONDS:
            raise TimeoutError("six-pair pilot exceeded 10 minutes")
    for name, item in features.items():
        if digest(Path(item["path"])) != item["sha256"] or sources[name]["image"] != digest(STAGE / "images" / name) or sources[name]["mask"] != digest(STAGE / "masks" / (name + ".png")):
            raise ValueError("feature or sealed source changed after pilot")
    report["elapsed_seconds"] = time.monotonic() - started
    report["free_after"] = guard()
    report["status"] = "complete"
    payload = (json.dumps(report, indent=2, allow_nan=False) + "\n").encode()
    if len(payload) > 20 * 1024**2:
        raise ValueError("report exceeds 20 MiB")
    (OUT / "report.json").write_bytes(payload)
    return report


def main() -> None:
    if os.environ.get("PYTHONDONTWRITEBYTECODE") != "1" or os.environ.get("PYTHONNOUSERSITE") != "1":
        raise ValueError("immutable base environment settings missing")
    if Path(os.environ.get("TORCH_HOME", "")).resolve() != (AI / "cache/torch"):
        raise ValueError("torch cache must be external")
    if Path(os.environ.get("TMPDIR", "")).resolve() != (AI / "cache"):
        raise ValueError("temp path must be external")
    signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError("pilot wall cap")))
    signal.setitimer(signal.ITIMER_REAL, MAX_SECONDS)
    try:
        result = run()
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
    print(json.dumps({"status": result["status"], "report": str(OUT / "report.json"),
                      "elapsed_seconds": result["elapsed_seconds"]}))


if __name__ == "__main__":
    main()
