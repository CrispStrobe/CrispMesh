#!/usr/bin/env python3
"""Mac M1 CPU-only three-view SAM2 parity probe; never approves masks."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

import numpy as np
from PIL import Image

from scripts.object_motion import sam_mask_cleanup as cleanup
from scripts.object_motion import sam_mask_trial as smoke
from scripts.object_motion import sam_point_full as full
from scripts.object_motion import sam_point_resume as resume
from scripts.object_motion import sam_point_trial as point
from scripts.object_motion import ycb_object_masks as common


NAMES = ("NP3_000.jpg", "NP3_066.jpg", "NP3_108.jpg")
MAX_SECONDS = 180
MAX_RSS_KIB = 1536 * 1024
MIN_AVAILABLE_KIB = 2560 * 1024
MAX_OUTPUT = 20 * 1024**2
MAX_LOG = 1024**2


def darwin_available_kib(vm_stat_text):
    match = re.search(r"page size of (\d+) bytes", vm_stat_text)
    if not match:
        raise ValueError("vm_stat missing page size")
    page_size = int(match.group(1))
    if not 4096 <= page_size <= 65536:
        raise ValueError("vm_stat implausible page size")
    counts = {}
    for label in ("Pages free", "Pages inactive"):
        found = re.search(rf"^{re.escape(label)}:\s*([\d,]+)\.", vm_stat_text, re.MULTILINE)
        if not found:
            raise ValueError(f"vm_stat missing {label}")
        counts[label] = int(found.group(1).replace(",", ""))
    # Conservative free+inactive estimate, not Linux MemAvailable; omit speculative.
    return (sum(counts.values()) * page_size) // 1024


def available_kib():
    result = subprocess.run(["vm_stat"], capture_output=True, text=True, timeout=3, check=True)
    return darwin_available_kib(result.stdout)


def darwin_rss_kib(ps_text):
    token = ps_text.strip()
    if not token.isdigit():
        raise ValueError("ps did not return one numeric RSS")
    return int(token)


def rss_kib(pid):
    result = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True,
                            text=True, timeout=3)
    if result.returncode != 0:
        return 0  # A just-exited worker is checked again at its terminal gate.
    return darwin_rss_kib(result.stdout)


def disk_free_both(external):
    workspace = Path(__file__).resolve().parents[2]
    return min(shutil.disk_usage(workspace).free, shutil.disk_usage(external).free)


def host_preflight(output, check_memory=True):
    output = Path(output)
    if sys.platform != "darwin" or os.uname().machine != "arm64":
        raise RuntimeError("M1 parity requires native arm64 macOS")
    mount = Path("/Volumes/backups")
    external_root = Path("/Volumes/backups/code/crispmesh-data")
    if (not mount.is_mount() or not external_root.is_dir() or external_root.is_symlink() or
            not output.parent.is_dir() or output.parent.is_symlink() or
            not output.parent.resolve().is_relative_to(external_root.resolve()) or
            os.stat(output.parent).st_dev != os.stat(mount).st_dev or
            os.stat(output.parent).st_dev == Path(__file__).resolve().parents[2].stat().st_dev):
        raise ValueError("parity output must be on mounted external data volume")
    if disk_free_both(output.parent) < common.MIN_FREE_BYTES:
        raise RuntimeError("both internal and external volumes need 10 GiB free")
    if check_memory and available_kib() < MIN_AVAILABLE_KIB:
        raise RuntimeError("Mac conservative memory estimate below 2.5 GiB")


def external_inputs(paths):
    mount = Path("/Volumes/backups")
    if not mount.is_mount():
        raise ValueError("external data volume is not mounted")
    device = mount.stat().st_dev
    for path in paths:
        path = Path(path)
        if (path.is_symlink() or not path.exists() or
                not path.resolve().is_relative_to(mount.resolve()) or path.stat().st_dev != device):
            raise ValueError(f"input is not a real file/directory on external volume: {path}")


def geometry(mask_path):
    if mask_path.is_symlink() or not mask_path.is_file() or mask_path.stat().st_size > 2 * 1024**2:
        raise ValueError("reference mask missing, linked or oversize")
    with Image.open(mask_path) as image:
        if image.mode != "L" or image.size != common.IMAGE_SIZE:
            raise ValueError("reference mask mode/size differs")
        array = np.asarray(image)
    if not np.isin(array, (0, 255)).all():
        raise ValueError("reference mask nonbinary")
    return array


def overlap(candidate, reference):
    if candidate.shape != reference.shape or candidate.dtype != np.uint8 or reference.dtype != np.uint8:
        raise ValueError("parity arrays need same uint8 grid")
    if not np.isin(candidate, (0, 255)).all() or not np.isin(reference, (0, 255)).all():
        raise ValueError("parity arrays must be binary")
    left, right = candidate != 0, reference != 0
    union = int(np.count_nonzero(left | right))
    return {"intersection_pixels": int(np.count_nonzero(left & right)),
            "union_pixels": union,
            "iou": 1.0 if union == 0 else float(np.count_nonzero(left & right) / union),
            "identical_pixels": bool(np.array_equal(candidate, reference)),
            "candidate_pixels": int(np.count_nonzero(left)),
            "reference_pixels": int(np.count_nonzero(right))}


def parity_gate(comparison, candidate_membership, reference_membership):
    return (comparison["iou"] >= 0.99 and
            candidate_membership == reference_membership == [1, 0, 0])


def selected_inputs(package, prompts, checkpoint, source, prior_root):
    rows, all_prompts, package_sha = full.validated_inputs(package, checkpoint, source, prompts)
    sealed = resume.prior_sealed_rows(prior_root, rows, all_prompts)
    selected = [(row, prompt, sealed[index], index // 8) for index, (row, prompt) in enumerate(zip(rows, all_prompts))
                if Path(row["path"]).name in NAMES]
    if tuple(Path(row["path"]).name for row, _, _, _ in selected) != NAMES:
        raise ValueError("exact three parity photos are not in sealed first 16")
    return selected, package_sha


def worker(package, dataset_root, prompts, checkpoint, source, prior_root, output):
    package, dataset_root, prompts, checkpoint, source, prior_root, output = map(
        Path, (package, dataset_root, prompts, checkpoint, source, prior_root, output))
    if output.exists() or output.is_symlink():
        raise FileExistsError(output)
    host_preflight(output)
    external_inputs((package, dataset_root, prompts, checkpoint, source, prior_root))
    selected, package_sha = selected_inputs(package, prompts, checkpoint, source, prior_root)
    started = time.monotonic()
    baseline = {"package_sha256": package_sha, "prompt_sha256": full.PROMPTS_SHA,
                "checkpoint_sha256": point.MODEL_SHA, "source_inventory_sha256": point.SOURCE_SHA,
                "runner_sha256": common.digest(__file__), "helpers": resume.dependencies()}
    output.mkdir()
    (output / "raw_masks").mkdir()
    (output / "cleaned_masks").mkdir()
    report = {"schema": "sam21_m1_cpu_parity_v1", "status": "running", "platform": "darwin_arm64_cpu",
              "mask_status": "generated_unreviewed", "names": list(NAMES), "inputs": baseline, "images": []}
    try:
        predictor = point.sam_predictor(source, checkpoint)  # explicit device="cpu", two Torch threads
        all_parity_pass = True
        for row, prompt, reference, batch_index in selected:
            if (time.monotonic() - started > MAX_SECONDS or available_kib() < MIN_AVAILABLE_KIB or
                    rss_kib(os.getpid()) > MAX_RSS_KIB or
                    disk_free_both(output.parent) < common.MIN_FREE_BYTES):
                raise RuntimeError("worker resource gate failed between parity views")
            name = Path(row["path"]).name
            photo = common.verified_training_photo(dataset_root, row)
            raw = point.binary_raw(predictor(photo, smoke.BOX, prompt["points_xy_label"]))
            entry = {"name": name, "source_sha256": row["sha256"],
                     "points_xy_label": prompt["points_xy_label"]}
            report["images"].append(entry)
            raw_path = output / "raw_masks" / f"{name}.png"
            Image.fromarray(raw, "L").save(raw_path, "PNG")
            entry["raw_mask_sha256"] = common.digest(raw_path)
            point.point_membership(raw, prompt["points_xy_label"])
            smoke.bounded_mask(raw)
            clean, stats = cleanup.largest_component(raw)
            point.point_membership(clean, prompt["points_xy_label"])
            entry["cleanup"] = stats
            for kind, array in (("raw", raw), ("cleaned", clean)):
                path = raw_path if kind == "raw" else output / "cleaned_masks" / f"{name}.png"
                if kind == "cleaned":
                    Image.fromarray(array, "L").save(path, "PNG")
                reference_path = prior_root / f"batch-{batch_index}" / (
                    "raw_masks" if kind == "raw" else "cleaned_masks") / f"{name}.png"
                if common.digest(reference_path) != reference[f"{kind}_mask_sha256"]:
                    raise ValueError("sealed reference mask changed")
                reference_array = geometry(reference_path)
                candidate_membership = point.point_membership(array, prompt["points_xy_label"])
                reference_membership = point.point_membership(reference_array, prompt["points_xy_label"])
                comparison = overlap(array, reference_array)
                passed = parity_gate(comparison, candidate_membership, reference_membership)
                all_parity_pass &= passed
                entry[kind] = {"candidate_sha256": common.digest(path),
                               "reference_sha256": reference[f"{kind}_mask_sha256"],
                               "candidate_point_membership": candidate_membership,
                               "reference_point_membership": reference_membership,
                               "predeclared_iou_and_membership_gate": passed,
                               **comparison}
            if sum(p.stat().st_size for p in output.rglob("*") if p.is_file()) > MAX_OUTPUT:
                raise ValueError("parity output above 20 MiB")
        if (common.digest(package) != package_sha or common.digest(prompts) != full.PROMPTS_SHA or
                common.digest(checkpoint) != point.MODEL_SHA or smoke.source_digest(source) != point.SOURCE_SHA or
                common.digest(__file__) != baseline["runner_sha256"] or resume.dependencies() != baseline["helpers"] or
                len(resume.prior_sealed_rows(prior_root, *full.validated_inputs(
                    package, checkpoint, source, prompts)[:2])) != 16):
            raise ValueError("parity inputs/code changed during run")
        report["status"] = "parity_observed_pending_visual_qa" if all_parity_pass else "parity_failed"
    except BaseException as error:
        report["status"] = "failed"
        report["failure"] = repr(error)
        raise
    finally:
        payload = (json.dumps(report, indent=2, sort_keys=True) + "\n").encode()
        if len(payload) > 1024**2:
            raise ValueError("parity report above 1 MiB")
        with (output / "manifest.json").open("xb") as out:
            out.write(payload)


def terminal_guard(seconds, rss, available, output_bytes, log_bytes, disk_bytes, returncode):
    if seconds > MAX_SECONDS:
        return "worker above 180 seconds"
    if rss > MAX_RSS_KIB:
        return "worker RSS above 1.5 GiB"
    if available < MIN_AVAILABLE_KIB:
        return "Mac estimated available memory below 2.5 GiB"
    if output_bytes > MAX_OUTPUT:
        return "output above 20 MiB"
    if log_bytes > MAX_LOG:
        return "log above 1 MiB"
    if disk_bytes < common.MIN_FREE_BYTES:
        return "internal/external disk below 10 GiB"
    if returncode != 0:
        return f"worker exit {returncode}"
    return None


def manifest_status(manifest):
    manifest = Path(manifest)
    if manifest.is_symlink() or not manifest.is_file() or manifest.stat().st_size > 1024**2:
        return None, "worker exited without valid parity manifest"
    try:
        status = json.loads(manifest.read_text()).get("status")
    except (OSError, ValueError, AttributeError):
        return None, "worker manifest is unreadable or malformed"
    if status not in ("parity_observed_pending_visual_qa", "parity_failed"):
        return None, "worker manifest has unexpected status"
    return status, None


def supervise(arguments, output, input_paths=()):
    output = Path(output)
    sidecar = output.parent / f"{output.name}.supervisor.json"
    if (output.exists() or output.is_symlink() or sidecar.exists() or sidecar.is_symlink() or
            output.parent.is_symlink() or not output.parent.is_dir()):
        raise ValueError("output must be fresh under a real existing external directory")
    host_preflight(output)
    external_inputs(input_paths)
    command = [sys.executable, "-m", "scripts.object_motion.sam_m1_parity", "--worker", *arguments]
    env = dict(os.environ, OMP_NUM_THREADS="2", MKL_NUM_THREADS="2", OPENBLAS_NUM_THREADS="2",
               PYTHONDONTWRITEBYTECODE="1")
    with tempfile.TemporaryFile(dir=output.parent) as logfile:
        child = subprocess.Popen(command, stdout=logfile, stderr=subprocess.STDOUT,
                                 start_new_session=True, env=env)
        started, peak, reason = time.monotonic(), 0, None
        try:
            while child.poll() is None:
                peak = max(peak, rss_kib(child.pid))
                reason = terminal_guard(time.monotonic() - started, peak, available_kib(),
                                        sum(p.stat().st_size for p in output.rglob("*") if p.is_file()) if output.exists() else 0,
                                        logfile.seek(0, os.SEEK_END), disk_free_both(output.parent), 0)
                if reason:
                    os.killpg(child.pid, signal.SIGTERM)
                    try:
                        child.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        os.killpg(child.pid, signal.SIGKILL)
                    break
                time.sleep(0.2)
            child.wait()
            reason = reason or terminal_guard(time.monotonic() - started, peak, available_kib(),
                                              sum(p.stat().st_size for p in output.rglob("*") if p.is_file()) if output.exists() else 0,
                                              logfile.seek(0, os.SEEK_END), disk_free_both(output.parent), child.returncode)
            logfile.seek(0)
            excerpt = logfile.read(MAX_LOG).decode(errors="replace")[-4096:]
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
    worker_status = None
    manifest = output / "manifest.json"
    if reason is None:
        worker_status, reason = manifest_status(manifest)
    result = {"schema": "sam21_m1_cpu_parity_supervisor_v1",
              "status": worker_status if reason is None else "failed",
              "reason": reason or (excerpt if child.returncode else None),
              "seconds": time.monotonic() - started, "peak_worker_rss_kib": peak}
    report_path = output / "supervisor.json" if output.exists() and output.is_dir() else sidecar
    with report_path.open("xb") as out:
        out.write((json.dumps(result, indent=2) + "\n").encode())
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("package", "dataset-root", "prompts", "checkpoint", "sam-source", "reference-root", "output"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    payload = ["--package", str(args.package), "--dataset-root", str(args.dataset_root),
               "--prompts", str(args.prompts), "--checkpoint", str(args.checkpoint),
               "--sam-source", str(args.sam_source), "--reference-root", str(args.reference_root),
               "--output", str(args.output)]
    if args.worker:
        worker(args.package, args.dataset_root, args.prompts, args.checkpoint, args.sam_source,
               args.reference_root, args.output)
    else:
        result = supervise(payload, args.output, (args.package, args.dataset_root, args.prompts,
                                                args.checkpoint, args.sam_source, args.reference_root))
        if result["status"] != "parity_observed_pending_visual_qa":
            raise SystemExit(1)


if __name__ == "__main__":
    main()
