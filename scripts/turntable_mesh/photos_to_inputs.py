"""One command from a folder of turntable photos to dense-stage inputs.

Two logical stages, reported through the run event log (``dense_events``):

  masks    coarse dark-object masks -> SAM 2.1 -> small dark-hole cleanup
  cameras  contrast images -> SIFT features -> matching -> native AliceVision
           global SfM with a locked, declared lens -> camera audit and ring
           sanity gates -> prepareDenseScene -> dense_all_views_inputs

Every step is a separate bounded process (own process group, deadline, cancel
file). The gates decide the exit code: 0 complete, 2 cameras failed a gate,
1 a step failed. No scanner mesh, supplied pose or depth is read anywhere.

Output layout::

  <output>/inputs/            what dense_all_views_inputs writes
  <output>/masks/             capture_NNNN.png, 0/255, original photo frame
  <output>/sfm/final.sfm      plus camera-audit.json, ring-sanity.json, gates.json
  <output>/native-prepared/   <viewId>.png undistorted images
  <output>/frontend.json      commands, timings, gate numbers, pass/fail, reasons
  <output>/mask-contact-sheet.png, sparse-overlay.png, photo-map.json, logs/

Assumptions about the capture, the calibration file format and the known
failure modes are in docs/PHOTOS-TO-INPUTS.md.
"""

import argparse
import json
import math
import os
from pathlib import Path
import re
import shutil
import sys
import time

from .dense_events import EventLog

REPOSITORY = Path(__file__).resolve().parents[2]
SCHEMA = "crispmesh_photos_to_inputs_v1"
CALIBRATION_SCHEMA = "crispmesh_lens_calibration_v1"
PHOTO_SUFFIXES = (".png", ".jpg", ".jpeg", ".tif", ".tiff")
INTERMEDIATES = ("work/photos", "work/contrast", "work/features", "work/matches")
INTERNAL_STEPS = ("coarse", "publish-masks", "contrast", "audit", "verify-prepared", "overlay")


class StepFailed(RuntimeError):
    pass


# ----------------------------------------------------------------------------
# Pure parts (unit tested without AliceVision or SAM)
# ----------------------------------------------------------------------------


def natural_key(name):
    """Sort key that orders ``x_2_rgb.png`` before ``x_10_rgb.png``."""
    return [int(part) if part.isdigit() else part.lower() for part in re.split(r"(\d+)", str(name))]


def capture_name(index):
    return f"capture_{index:04d}.png"


def load_calibration(path):
    """Read a lens file into {width, height, fx, fy, cx, cy, k:[k1,k2,k3], sensor_width_mm}.

    Two spellings are accepted. Ours (``schema: crispmesh_lens_calibration_v1``)
    and the raw 3DLF ``rgb_optic.json`` (``model: brown5`` with a nested
    ``distortion``). Tangential terms must be zero: the camera recovery and the
    dense stage only model radial k1, k2, k3. The principal point is in the
    convention where the centre of the top-left pixel is (0, 0).
    """
    data = json.loads(Path(path).read_text())
    if data.get("schema") == CALIBRATION_SCHEMA:
        if data.get("model") != "radialk3":
            raise ValueError("calibration model must be radialk3")
        if data.get("principal_point_convention", "pixel_centre") != "pixel_centre":
            raise ValueError("principal_point_convention must be pixel_centre")
        width, height = data["calibration_width"], data["calibration_height"]
        coefficients = [data["k1"], data["k2"], data["k3"]]
    elif data.get("model") == "brown5" and isinstance(data.get("distortion"), dict):
        distortion = data["distortion"]
        if distortion.get("p1", 0) != 0 or distortion.get("p2", 0) != 0:
            raise ValueError("tangential distortion is not supported; p1 and p2 must be zero")
        width, height = data["width"], data["height"]
        coefficients = [distortion["k1"], distortion["k2"], distortion["k3"]]
    else:
        raise ValueError(f"unknown calibration format; expected schema {CALIBRATION_SCHEMA} or a brown5 rgb_optic.json")
    values = [width, height, data["fx"], data["fy"], data["cx"], data["cy"], *coefficients]
    if any(isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v) for v in values):
        raise ValueError("calibration values must be finite numbers")
    if int(width) != width or int(height) != height or width < 1 or height < 1 or data["fx"] <= 0 or data["fy"] <= 0:
        raise ValueError("invalid calibration resolution or focal length")
    sensor = float(data.get("sensor_width_mm", 36.0))
    if not math.isfinite(sensor) or sensor <= 0:
        raise ValueError("invalid sensor_width_mm")
    return {"width": int(width), "height": int(height), "fx": float(data["fx"]), "fy": float(data["fy"]),
            "cx": float(data["cx"]), "cy": float(data["cy"]), "k": [float(v) for v in coefficients],
            "sensor_width_mm": sensor}


def scale_calibration(calibration, width, height, aspect_tolerance=0.002):
    """Scale a lens declared at one resolution to the photo resolution.

    A pure resize keeps the radial coefficients (they act on normalised
    coordinates). Focal lengths scale with the image; the principal point is
    scaled about the image corner, hence the half-pixel terms. The photos must
    be a resize of the calibration frame, not a crop: the aspect ratios must agree.
    """
    sx, sy = width / calibration["width"], height / calibration["height"]
    if abs(sx / sy - 1) > aspect_tolerance:
        raise ValueError(f"photo {width}x{height} is not a resize of the calibration frame "
                         f"{calibration['width']}x{calibration['height']} (a crop cannot be scaled)")
    return {"fx": calibration["fx"] * sx, "fy": calibration["fy"] * sy,
            "cx": (calibration["cx"] + 0.5) * sx - 0.5, "cy": (calibration["cy"] + 0.5) * sy - 0.5,
            "k": list(calibration["k"]), "scale": [sx, sy]}


def alicevision_intrinsic_fields(scaled, width, height, sensor_width):
    """The fields of an AliceVision pinhole/radialk3 intrinsic that pin the lens, all locked."""
    focal = scaled["fy"] * sensor_width / width
    return {"focalLength": str(focal), "initialFocalLength": str(focal), "pixelRatio": str(scaled["fy"] / scaled["fx"]),
            "principalPoint": [str(scaled["cx"] - width / 2), str(scaled["cy"] - height / 2)],
            "initializationMode": "calibrated", "distortionInitializationMode": "calibrated",
            "distortionParams": [str(v) for v in scaled["k"]], "locked": "true", "scaleLocked": "true",
            "offsetLocked": "true", "distortionLocked": "true", "pixelRatioLocked": "true"}


def calibrated_scene(scene, scaled):
    """Copy of an uncalibrated cameraInit scene with its single intrinsic replaced by the declared lens."""
    scene = json.loads(json.dumps(scene))
    if len(scene.get("intrinsics", [])) != 1 or scene.get("poses") or scene.get("structure"):
        raise ValueError("expected one shared intrinsic and no poses or landmarks before camera recovery")
    intrinsic = scene["intrinsics"][0]
    if intrinsic.get("type") != "pinhole" or intrinsic.get("distortionType") != "radialk3":
        raise ValueError("cameraInit did not produce a pinhole radialk3 intrinsic")
    width, height = int(intrinsic["width"]), int(intrinsic["height"])
    intrinsic.update(alicevision_intrinsic_fields(scaled, width, height, float(intrinsic["sensorWidth"])))
    return scene


def resolve_envelope(spec, width, height):
    """Search window for the object as integer (x0, y0, x1, y1).

    ``auto`` is the whole frame. Otherwise four comma separated numbers: all
    at most 1 are fractions of width and height, anything else is pixels.
    """
    if spec is None or str(spec).strip().lower() == "auto":
        return 0, 0, width, height
    try:
        values = [float(v) for v in str(spec).split(",")]
    except ValueError as error:
        raise ValueError("envelope must be 'auto' or x0,y0,x1,y1") from error
    if len(values) != 4 or not all(math.isfinite(v) and v >= 0 for v in values):
        raise ValueError("envelope must be 'auto' or x0,y0,x1,y1")
    if all(v <= 1 for v in values):
        values = [values[0] * width, values[1] * height, values[2] * width, values[3] * height]
    x0, y0, x1, y1 = (int(round(v)) for v in values)
    x1, y1 = min(x1, width), min(y1, height)
    if not (0 <= x0 < x1 and 0 <= y0 < y1):
        raise ValueError("envelope is empty or outside the photo")
    return x0, y0, x1, y1


def otsu_threshold(gray):
    """Otsu's threshold of an 8-bit array: pixels strictly below the result are the dark class."""
    import numpy as np

    histogram = np.bincount(np.asarray(gray, np.uint8).ravel(), minlength=256).astype(float)
    total = histogram.sum()
    cumulative = np.cumsum(histogram)
    weighted = np.cumsum(histogram * np.arange(256))
    with np.errstate(divide="ignore", invalid="ignore"):
        mean_dark = weighted / cumulative
        mean_light = (weighted[-1] - weighted) / (total - cumulative)
        between = cumulative * (total - cumulative) * (mean_dark - mean_light) ** 2
    between[~np.isfinite(between)] = -1
    return int(np.argmax(between)) + 1


def coarse_mask(gray, threshold, envelope):
    """Largest 8-connected dark component inside the envelope. Prompt for SAM, not a final mask.

    ``threshold`` is a grey level (dark means strictly below it) or "otsu".
    """
    import numpy as np
    from scipy import ndimage

    gray = np.asarray(gray)
    x0, y0, x1, y1 = envelope
    window = gray[y0:y1, x0:x1]
    level = otsu_threshold(window) if threshold == "otsu" else int(threshold)
    support = np.zeros(gray.shape, bool)
    support[y0:y1, x0:x1] = window < level
    labels, count = ndimage.label(support, structure=np.ones((3, 3)))
    if not count:
        raise ValueError("no dark pixels inside the envelope; is the object dark on a light backdrop?")
    sizes = np.bincount(labels.ravel())
    sizes[0] = 0
    mask = labels == int(sizes.argmax())
    rows, columns = np.nonzero(mask)
    box = [int(columns.min()), int(rows.min()), int(columns.max()) + 1, int(rows.max()) + 1]
    return mask, {"threshold": level, "foreground_pixels": int(mask.sum()),
                  "other_dark_pixels_in_envelope": int(support.sum() - mask.sum()),
                  "dark_pixels_outside_envelope": int((gray < level).sum() - support.sum()), "bbox_xyxy": box,
                  "touches_envelope": bool(box[0] <= x0 or box[1] <= y0 or box[2] >= x1 or box[3] >= y1)}


def gamma_table(gamma):
    """8-bit lookup table; gamma 0.5 is the square root used for the dark 3DLF figures."""
    if gamma == 0.5:
        return [round(255 * math.sqrt(v / 255)) for v in range(256)]
    return [round(255 * (v / 255) ** gamma) for v in range(256)]


def contrast_image(image, gamma, clahe_clip, clahe_grid):
    """Gamma then CLAHE on Lab lightness. Feature images only; the masks come from the original photos."""
    from PIL import ImageOps

    photo = ImageOps.exif_transpose(image).convert("RGB")
    if gamma and gamma != 1:
        photo = photo.point(gamma_table(gamma) * 3)
    if clahe_clip and clahe_clip > 0:
        import cv2
        import numpy as np
        from PIL import Image

        lab = cv2.cvtColor(np.asarray(photo), cv2.COLOR_RGB2LAB)
        lab[:, :, 0] = cv2.createCLAHE(clipLimit=float(clahe_clip), tileGridSize=(clahe_grid, clahe_grid)).apply(lab[:, :, 0])
        photo = Image.fromarray(cv2.cvtColor(lab, cv2.COLOR_LAB2RGB), "RGB")
    return photo


def ring_statistics(indices, centres, rotations, duplicate_step_deg=0.5):
    """How well the camera centres form one planar circle walked in capture order.

    ``indices`` are capture positions (gaps allowed where views failed to
    register), ``centres`` camera centres, ``rotations`` world-to-camera
    matrices. Photo-only: nothing here knows the true turntable angles.
    """
    import numpy as np

    order = np.argsort(indices)
    indices = np.asarray(indices)[order]
    centres = np.asarray(centres, float)[order]
    rotations = np.asarray(rotations, float)[order]
    if len(indices) < 4:
        return {"registered": int(len(indices)), "degenerate": True}
    mean = centres.mean(0)
    _, singular, basis = np.linalg.svd(centres - mean)
    normal = basis[2]
    planar = np.column_stack([(centres - mean) @ basis[0], (centres - mean) @ basis[1]])
    solution = np.linalg.lstsq(np.column_stack([2 * planar, np.ones(len(planar))]), (planar**2).sum(1), rcond=None)[0]
    middle = solution[:2]
    radius = float(np.sqrt(max(solution[2] + middle @ middle, 0)))
    if not math.isfinite(radius) or radius <= 0 or singular[1] < 1e-9 * max(singular[0], 1e-300):
        return {"registered": int(len(indices)), "degenerate": True}
    distance = np.linalg.norm(planar - middle, axis=1)
    angle = np.rad2deg(np.unwrap(np.arctan2(planar[:, 1] - middle[1], planar[:, 0] - middle[0])))
    steps = np.diff(angle) / np.diff(indices)
    if np.median(steps) < 0:
        angle, steps = -angle, -steps
    circular = np.sort(np.mod(angle - angle[0], 360))
    gaps = np.diff(np.append(circular, 360))
    centre = mean + middle[0] * basis[0] + middle[1] * basis[1]
    inward = centre - centres
    inward -= np.outer(inward @ normal, normal)
    # Each optical axis should pass close to the ring axis (any elevation is fine) and point towards it.
    forward = rotations[:, 2, :]
    across = np.cross(forward, normal)
    length = np.linalg.norm(across, axis=1)
    miss = np.where(length > 1e-9, np.abs(np.einsum("ij,ij->i", inward, across)) / np.maximum(length, 1e-9),
                    np.linalg.norm(inward, axis=1)) / radius * 100
    outward = int((np.einsum("ij,ij->i", forward, inward) <= 0).sum())
    pairs = [[int(a), int(b)] for a, b, s in zip(indices[:-1], indices[1:], steps) if abs(s) < duplicate_step_deg]

    def summary(values):
        return {"min": float(np.min(values)), "median": float(np.median(values)), "max": float(np.max(values))}

    return {"registered": int(len(indices)), "degenerate": False, "fitted_radius": radius,
            "radius_spread_percent": float((distance.max() - distance.min()) / radius * 100),
            "out_of_plane_percent": float(np.abs((centres - mean) @ normal).max() / radius * 100),
            "largest_angular_gap_deg": float(gaps.max()), "unwrapped_sweep_deg": float(angle[-1] - angle[0]),
            "step_deg_per_capture": summary(steps), "reversed_steps": int((steps < -duplicate_step_deg).sum()),
            "duplicate_pose_pairs": pairs, "optical_axis_miss_percent": summary(miss), "cameras_looking_outward": outward}


def decide_gates(audit, ring, limits):
    """(passed, reasons). ``audit`` is alicevision_cameras.audit_scene output, ``ring`` ring_statistics output."""
    behind = "landmark behind camera"
    fraction = audit.get("positive_depth_fraction")
    tolerated = fraction is not None and fraction >= limits["minimum_positive_depth_fraction"]
    reasons = ["camera audit: " + reason for reason in audit.get("reasons", []) if not (reason == behind and tolerated)]
    if behind in audit.get("reasons", []) and not tolerated:
        reasons.append(f"only {fraction:.6f} of the sparse observations lie in front of their camera; "
                       f"need {limits['minimum_positive_depth_fraction']}")
        reasons.remove("camera audit: " + behind)
    if not audit.get("passed") and not audit.get("reasons"):
        reasons.append("camera audit failed")
    if audit.get("coverage", 0) < limits["minimum_registered_fraction"]:
        reasons.append(f"registered {audit.get('registered_cameras')} of {audit.get('input_images')} photos; "
                       f"need a fraction of {limits['minimum_registered_fraction']}")
    if ring.get("degenerate", True):
        reasons.append("camera centres do not define a ring")
        return False, sorted(set(reasons))
    checks = (("radius_spread_percent", "maximum_radius_spread_percent", "ring radius spread {:.2f}% exceeds {}%"),
              ("out_of_plane_percent", "maximum_out_of_plane_percent", "camera centres leave the ring plane by {:.2f}% of the radius; limit {}%"),
              ("largest_angular_gap_deg", "maximum_angular_gap_deg", "largest gap between neighbouring cameras is {:.1f} degrees; limit {}"),
              ("reversed_steps", "maximum_reversed_steps", "{} capture steps run against the turning direction; limit {}"))
    for name, limit, text in checks:
        if ring[name] > limits[limit]:
            reasons.append(text.format(ring[name], limits[limit]))
    if ring["optical_axis_miss_percent"]["max"] > limits["maximum_optical_axis_miss_percent"]:
        reasons.append("a camera's optical axis misses the ring axis by {:.1f}% of the radius; limit {}%".format(
            ring["optical_axis_miss_percent"]["max"], limits["maximum_optical_axis_miss_percent"]))
    if ring["cameras_looking_outward"]:
        reasons.append(f"{ring['cameras_looking_outward']} cameras look away from the ring axis")
    return not reasons, sorted(set(reasons))


def alicevision_command(location, tool, arguments, python):
    """(command, extra environment) for one AliceVision tool.

    ``location`` is a wrapper script (called as ``wrapper TOOL args``; a
    ``.py`` wrapper is run with ``python``) or an install prefix containing
    ``bin/aliceVision_TOOL``.
    """
    location = Path(location)
    arguments = [str(a) for a in arguments]
    if location.is_dir():
        library = str(location / "lib")
        environment = {"ALICEVISION_ROOT": str(location)}
        for name in ("DYLD_LIBRARY_PATH", "LD_LIBRARY_PATH"):
            environment[name] = library + (os.pathsep + os.environ[name] if os.environ.get(name) else "")
        return [str(location / "bin" / ("aliceVision_" + tool)), *arguments], environment
    if location.suffix == ".py":
        return [str(python), str(location), tool, *arguments], {}
    return [str(location), tool, *arguments], {}


def sensor_database(location, explicit=None):
    if explicit:
        return Path(explicit)
    location = Path(location)
    prefix = location if location.is_dir() else location.parent / "prefix"
    return prefix / "share" / "aliceVision" / "cameraSensors.db"


def build_commands(cfg):
    """Every external command of a run, keyed by step name. ``cfg`` is the resolved configuration."""
    out = Path(cfg["output"])
    work, sfm = out / "work", out / "sfm"
    python, threads = cfg["python"], str(cfg["threads"])
    resource = ["--maxCoresAvailable", threads, "--maxMemoryAvailable", str(int(cfg["alicevision_memory_gib"] * 2**30))]

    def internal(name):
        return [python, "-m", "scripts.turntable_mesh.photos_to_inputs", "--internal-step", name, "--output", str(out)]

    def av(tool, arguments, dense=False):
        command, environment = alicevision_command(cfg["alicevision_dense"] if dense else cfg["alicevision"], tool,
                                                   arguments + resource, python)
        # The tools thread through OpenMP; keep the BLAS underneath single-threaded.
        return command, {**environment, "OPENBLAS_NUM_THREADS": "1", "VECLIB_MAXIMUM_THREADS": "1"}

    sam = [cfg["sam_python"], "-m", "scripts.turntable_mesh.segment", "--images", str(work / "photos"),
           "--coarse-masks", str(work / "coarse-masks"), "--output", str(work / "sam"), "--source", cfg["sam_source"],
           "--checkpoint", cfg["sam_checkpoint"], "--device", cfg["device"], "--views", str(cfg["photo_count"])]
    if cfg["sam_config"]:
        sam += ["--model-config", cfg["sam_config"]]
    sam += [flag for flag, on in (("--multimask", cfg["sam_multimask"]), ("--preserve-holes", cfg["sam_preserve_holes"]),
                                  ("--automatic-cues", cfg["sam_automatic_cues"])) if on]
    features, matches = str(work / "features"), str(work / "matches")
    database = str(sensor_database(cfg["alicevision"], cfg["sensor_database"]))
    commands = {
        "coarse": (internal("coarse"), {}),
        "sam": (sam, {"PYTHONPATH": os.pathsep.join([str(REPOSITORY), *cfg["sam_pythonpath"]])}),
        "cleanup": ([python, "-m", "scripts.turntable_mesh.silhouette_cleanup", "--images", str(work / "photos"),
                     "--masks", str(work / "sam" / "masks"), "--output", str(work / "mask-cleanup"),
                     "--dark-object-bright-background", "--maximum-total-filled-foreground-fraction",
                     str(cfg["hole_cleanup_budget"])], {}),
        "publish-masks": (internal("publish-masks"), {}),
        "contrast": (internal("contrast"), {}),
        "cameraInit-uncalibrated": av("cameraInit", [
            "--imageFolder", str(work / "contrast"), "--output", str(sfm / "cameraInit-uncalibrated.sfm"),
            "--sensorDatabase", database, "--defaultFieldOfView", str(cfg["initial_field_of_view"]),
            "--groupCameraFallback", "global"]),
        "featureExtraction": av("featureExtraction", [
            "--input", str(sfm / "cameraInit-uncalibrated.sfm"), "--output", features,
            "--describerTypes", cfg["describer_types"], "--describerPreset", cfg["describer_preset"],
            "--forceCpuExtraction", "true", "--masksFolder", str(out / "masks"), "--maskExtension", "png",
            "--maskInvert", "false", "--maxThreads", threads]),
        "cameraInit": av("cameraInit", [
            "--input", str(sfm / "calibrated-input.sfm"), "--output", str(sfm / "cameraInit.sfm"),
            "--sensorDatabase", database, "--groupCameraFallback", "global"]),
        "imageMatching": av("imageMatching", [
            "--input", str(sfm / "cameraInit.sfm"), "--featuresFolders", features, "--output", str(sfm / "pairs.txt"),
            "--method", cfg["matching_method"]]),
        "featureMatching": av("featureMatching", [
            "--input", str(sfm / "cameraInit.sfm"), "--featuresFolders", features, "--imagePairsList",
            str(sfm / "pairs.txt"), "--output", matches, "--describerTypes", cfg["describer_types"],
            "--randomSeed", str(cfg["random_seed"])]),
        "globalSfM": av("globalSfM", [
            "--input", str(sfm / "cameraInit.sfm"), "--featuresFolders", features, "--matchesFolders", matches,
            "--output", str(sfm / "final.sfm"), "--outputViewsAndPoses", str(sfm / "views.sfm"),
            "--extraInfoFolder", str(sfm / "extra"), "--lockAllIntrinsics", "true", "--randomSeed",
            str(cfg["random_seed"]), *cfg["sfm_option"]]),
        "audit": (internal("audit"), {}),
        "prepareDenseScene": av("prepareDenseScene", [
            "--input", str(sfm / "final.sfm"), "--output", str(out / "native-prepared"), "--outputFileType", "png",
            "--evCorrection", "0"], dense=True),
        "verify-prepared": (internal("verify-prepared"), {}),
        "inputs": ([python, "-m", "scripts.turntable_mesh.dense_all_views_inputs", "--scene", str(sfm / "final.sfm"),
                    "--prepared", str(out / "native-prepared"), "--raw-masks", str(out / "masks"),
                    "--output", str(out / "inputs")], {}),
        "overlay": (internal("overlay"), {}),
    }
    return {name: {"command": [str(c) for c in command], "environment": environment}
            for name, (command, environment) in commands.items()}


# ----------------------------------------------------------------------------
# Internal steps: run in the scientific interpreter as bounded subprocesses
# ----------------------------------------------------------------------------


def _photos(cfg):
    return sorted((p for p in Path(cfg["photos"]).iterdir() if p.suffix.lower() in PHOTO_SUFFIXES and p.is_file()),
                  key=lambda p: natural_key(p.name))


def step_coarse(out, cfg):
    """Stage photos as capture_NNNN.png (PNG byte-exact, others decoded once) and write coarse masks."""
    import numpy as np
    from PIL import Image, ImageOps

    photos, masks = out / "work/photos", out / "work/coarse-masks"
    photos.mkdir(parents=True)
    masks.mkdir(parents=True)
    rows, size = [], None
    for index, source in enumerate(_photos(cfg)):
        name = capture_name(index)
        with Image.open(source) as image:
            byte_exact = source.suffix.lower() == ".png" and image.getexif().get(0x0112, 1) == 1
            image = ImageOps.exif_transpose(image).convert("RGB")
            gray = np.asarray(image.convert("L"))
            if byte_exact:
                shutil.copyfile(source, photos / name)
            else:
                image.save(photos / name, format="PNG", compress_level=3)
        if size not in (None, image.size):
            raise ValueError(f"{source.name} is {image.size}; all photos must share one size ({size})")
        size = image.size
        envelope = resolve_envelope(cfg["envelope"], *size)
        mask, info = coarse_mask(gray, cfg["dark_threshold"], envelope)
        Image.fromarray(mask.astype(np.uint8) * 255).save(masks / (name + ".png"))
        rows.append({"capture": name, "source": source.name, "byte_exact_copy": byte_exact, **info})
    report = {"width": size[0], "height": size[1], "envelope_xyxy": list(envelope), "photos": rows}
    (out / "photo-map.json").write_text(json.dumps(report, indent=1) + "\n")


def step_publish_masks(out, cfg):
    """Final masks under their capture names, statistics and the contact sheet."""
    import numpy as np
    from PIL import Image, ImageDraw
    from scipy import ndimage

    target = out / "masks"
    target.mkdir()
    names = [capture_name(i) for i in range(cfg["photo_count"])]
    rows, areas = [], []
    for name in names:
        with Image.open(out / "work/mask-cleanup/masks" / (name + ".png")) as image:
            mask = np.asarray(image.convert("L"))
        if not np.isin(mask, (0, 255)).all() or not mask.any():
            raise ValueError("mask is empty or not 0/255: " + name)
        Image.fromarray(mask).save(target / name)
        with Image.open(out / "work/coarse-masks" / (name + ".png")) as image:
            coarse = np.asarray(image) > 0
        mask = mask > 0
        areas.append(int(mask.sum()))
        rows.append({"capture": name, "foreground_pixels": areas[-1], "added_to_coarse_pixels": int((mask & ~coarse).sum()),
                     "dropped_from_coarse_pixels": int((coarse & ~mask).sum())})
    dropped = [r["dropped_from_coarse_pixels"] / max(r["foreground_pixels"], 1) for r in rows]
    stats = {"count": len(rows), "median_area_pixels": float(np.median(areas)),
             "median_area_fraction": float(np.median(areas) / mask.size), "minimum_area_pixels": min(areas),
             "maximum_area_pixels": max(areas), "maximum_dropped_coarse_fraction": float(max(dropped)),
             "views_dropping_over_3_percent_of_coarse": [r["capture"] for r, d in zip(rows, dropped) if d > 0.03],
             "views": rows}
    (out / "masks-report.json").write_text(json.dumps(stats, indent=1) + "\n")
    picks = [names[i] for i in np.linspace(0, len(names), min(12, len(names)), endpoint=False, dtype=int)]
    thresholds = {row["capture"]: row["threshold"] for row in json.loads((out / "photo-map.json").read_text())["photos"]}
    width = 460
    height = round(width * mask.shape[0] / mask.shape[1])
    sheet = Image.new("RGB", (4 * width, ((len(picks) + 3) // 4) * (height + 18)), "#222222")
    draw = ImageDraw.Draw(sheet)
    for n, name in enumerate(picks):
        with Image.open(out / "work/photos" / name) as image:
            rgb = np.asarray(image.convert("RGB")).astype(float)
            dark = np.asarray(image.convert("L")) < thresholds[name]
        with Image.open(target / name) as image:
            mask = np.asarray(image) > 0
        rgb = np.clip(rgb * 1.6 + 25, 0, 255)
        rgb[mask] = rgb[mask] * 0.6 + np.array([0, 255, 0]) * 0.4
        rgb[dark & ~mask] = [255, 0, 0]
        rgb[mask ^ ndimage.binary_erosion(mask)] = [0, 255, 255]
        tile = Image.fromarray(rgb.astype(np.uint8)).resize((width, height), Image.LANCZOS)
        sheet.paste(tile, (n % 4 * width, n // 4 * (height + 18) + 18))
        draw.text((n % 4 * width + 4, n // 4 * (height + 18) + 3), name, fill="white")
    sheet.save(out / "mask-contact-sheet.png")


def step_contrast(out, cfg):
    from PIL import Image

    target = out / "work/contrast"
    target.mkdir(parents=True)
    for index in range(cfg["photo_count"]):
        with Image.open(out / "work/photos" / capture_name(index)) as image:
            contrast_image(image, cfg["contrast_gamma"], cfg["clahe_clip"], cfg["clahe_grid"]).save(
                target / capture_name(index), format="PNG", compress_level=3)


def scene_cameras(scene):
    """[(capture index, name, world-to-camera rotation, centre)] of the registered views."""
    import numpy as np

    poses = {str(p["poseId"]): p["pose"]["transform"] for p in scene.get("poses", [])}
    rows = []
    for view in scene["views"]:
        pose = poses.get(str(view["poseId"]))
        if pose is not None:
            name = Path(view["path"]).name
            rows.append((int(re.fullmatch(r"capture_(\d+)\.png", name)[1]), name,
                         np.asarray(pose["rotation"], float).reshape(3, 3, order="F"), np.asarray(pose["center"], float)))
    return sorted(rows, key=lambda row: row[0])


def step_audit(out, cfg):
    from .alicevision_cameras import audit_scene

    sfm = out / "sfm"
    scene = json.loads((sfm / "final.sfm").read_text())
    expected = json.loads((sfm / "expected-calibration.json").read_text())
    limits = cfg["gates"]
    audit = audit_scene(sfm / "final.sfm", expected_names=[capture_name(i) for i in range(cfg["photo_count"])],
                        expected_calibration=expected, minimum_coverage=limits["minimum_registered_fraction"],
                        minimum_observations_per_view=limits["minimum_observations_per_view"],
                        maximum_reprojection_p95=limits["maximum_view_reprojection_p95_pixels"])
    (sfm / "camera-audit.json").write_text(json.dumps(audit, indent=1) + "\n")
    cameras = scene_cameras(scene)
    ring = ring_statistics([c[0] for c in cameras], [c[3] for c in cameras], [c[2] for c in cameras],
                           limits["duplicate_step_deg"])
    (sfm / "ring-sanity.json").write_text(json.dumps(ring, indent=1) + "\n")
    passed, reasons = decide_gates(audit, ring, limits)
    per_view = [v["reprojection_pixels"]["p95"] for v in audit["per_view"] if v["reprojection_pixels"]]
    gates = {"passed": passed, "reasons": reasons, "limits": limits, "registered_views": audit["registered_cameras"],
             "input_photos": audit["input_images"], "missing": audit["missing_names"], "landmarks": audit["landmarks"],
             "observations": audit["observations"], "reprojection_pixels": audit["reprojection_pixels"],
             "worst_view_reprojection_p95_pixels": max(per_view) if per_view else None,
             "positive_depth_fraction": audit.get("positive_depth_fraction"), "ring": ring}
    (sfm / "gates.json").write_text(json.dumps(gates, indent=1) + "\n")


def step_verify_prepared(out, cfg):
    from PIL import Image

    scene = json.loads((out / "sfm/final.sfm").read_text())
    poses = {str(p["poseId"]) for p in scene["poses"]}
    intrinsic = scene["intrinsics"][0]
    size = (int(intrinsic["width"]), int(intrinsic["height"]))
    registered = [v for v in scene["views"] if str(v["poseId"]) in poses]
    for view in registered:
        with Image.open(out / "native-prepared" / (str(view["viewId"]) + ".png")) as image:
            if image.size != size:
                raise ValueError(f"undistorted image {view['viewId']} is {image.size}, expected {size}")
    extra = len(list((out / "native-prepared").glob("*.png"))) - len(registered)
    if extra:
        raise ValueError(f"{extra} unexpected undistorted images")


def step_overlay(out, cfg):
    """Sparse points projected into four undistorted photos with their masks: do cameras and masks agree?"""
    import cv2
    import numpy as np

    views = json.loads((out / "inputs/cameras.json").read_text())["views"]
    points = np.load(out / "inputs/sparse_points.npy")
    picks = set(np.linspace(0, len(views), min(4, len(views)), endpoint=False, dtype=int).tolist())
    tiles, inside = [], []
    for n, view in enumerate(views):
        mask_path = Path(view["mask"])
        mask = cv2.imread(str(mask_path if mask_path.is_absolute() else out / "inputs" / mask_path), cv2.IMREAD_GRAYSCALE) > 127
        fx, fy, cx, cy = view["k"]
        camera = points @ np.asarray(view["rotation"]).T + np.asarray(view["translation"])
        front = camera[:, 2] > 1e-9
        x = fx * camera[front, 0] / camera[front, 2] + cx - 0.5
        y = fy * camera[front, 1] / camera[front, 2] + cy - 0.5
        keep = (x >= 0) & (x <= mask.shape[1] - 1) & (y >= 0) & (y <= mask.shape[0] - 1)
        xi, yi = np.rint(x[keep]).astype(int), np.rint(y[keep]).astype(int)
        near = cv2.dilate(mask.astype(np.uint8), np.ones((9, 9), np.uint8)) > 0
        inside.append(float(near[yi, xi].mean()) if len(xi) else 0.0)
        if n in picks:
            image_path = Path(view["image"])
            photo = cv2.imread(str(image_path if image_path.is_absolute() else out / "inputs" / image_path))[..., :3].astype(float)
            photo = np.clip(photo * 1.2, 0, 255)
            photo[mask] = photo[mask] * 0.6 + np.array([0, 255, 0]) * 0.4
            photo = photo.astype(np.uint8)
            for a, b in zip(xi, yi):
                cv2.circle(photo, (int(a), int(b)), 2, (0, 0, 255), -1)
            cv2.putText(photo, view["source"], (12, 34), cv2.FONT_HERSHEY_SIMPLEX, 1, (0, 0, 0), 2)
            tiles.append(cv2.resize(photo, (800, round(800 * photo.shape[0] / photo.shape[1]))))
    while len(tiles) % 2:
        tiles.append(np.zeros_like(tiles[0]))
    cv2.imwrite(str(out / "sparse-overlay.png"), np.vstack([np.hstack(tiles[i:i + 2]) for i in range(0, len(tiles), 2)]))
    (out / "sparse-overlay.json").write_text(json.dumps(
        {"sparse_points_inside_mask_fraction": {"min": min(inside), "median": float(np.median(inside))},
         "mask_dilation_pixels": 4}, indent=1) + "\n")


def internal_step(name, output):
    out = Path(output)
    cfg = json.loads((out / "frontend-config.json").read_text())
    {"coarse": step_coarse, "publish-masks": step_publish_masks, "contrast": step_contrast, "audit": step_audit,
     "verify-prepared": step_verify_prepared, "overlay": step_overlay}[name](out, cfg)


# ----------------------------------------------------------------------------
# Orchestration
# ----------------------------------------------------------------------------


# Default dark-hole budget for SAM masks; silhouette_cleanup.py itself keeps 0.02 as its default.
SAM_HOLE_CLEANUP_BUDGET = 0.05


def resolve(args):
    """Flags, then environment, then defaults, into one JSON-serialisable configuration."""
    def pick(value, variable, default=None):
        return value or os.environ.get(variable) or default

    photos = Path(args.photos).absolute()
    if not photos.is_dir():
        raise ValueError("--photos must be a directory")
    count = len([p for p in photos.iterdir() if p.suffix.lower() in PHOTO_SUFFIXES and p.is_file()])
    if not 3 <= count <= 96:
        raise ValueError(f"found {count} photos; the camera audit supports 3 to 96")
    alicevision = pick(args.alicevision, "CRISPMESH_ALICEVISION")
    sam_python = pick(args.sam_python, "CRISPMESH_SAM_PYTHON")
    sam_source = pick(args.sam_source, "CRISPMESH_SAM_SOURCE")
    sam_checkpoint = pick(args.sam_checkpoint, "CRISPMESH_SAM_CHECKPOINT")
    missing = [flag for flag, value in (("--alicevision / CRISPMESH_ALICEVISION", alicevision),
                                        ("--sam-python / CRISPMESH_SAM_PYTHON", sam_python),
                                        ("--sam-source / CRISPMESH_SAM_SOURCE", sam_source),
                                        ("--sam-checkpoint / CRISPMESH_SAM_CHECKPOINT", sam_checkpoint)) if not value]
    if missing:
        raise ValueError("missing tool locations: " + "; ".join(missing))
    threshold = args.dark_threshold.strip().lower()
    if threshold != "otsu" and not (threshold.isdigit() and 1 <= int(threshold) <= 255):
        raise ValueError("--dark-threshold must be a grey level 1..255 or 'otsu'")
    pythonpath = pick(args.sam_pythonpath, "CRISPMESH_SAM_PYTHONPATH", "")
    cfg = {
        "schema": SCHEMA, "photos": str(photos), "photo_count": count,
        "calibration": str(Path(args.calibration).absolute()), "output": str(Path(args.output).absolute()),
        "python": str(pick(args.python, "CRISPMESH_PYTHON", sys.executable)),
        "alicevision": str(Path(alicevision).absolute()),
        "alicevision_dense": str(Path(pick(args.alicevision_dense, "CRISPMESH_ALICEVISION_DENSE", alicevision)).absolute()),
        "sensor_database": pick(args.sensor_database, "CRISPMESH_ALICEVISION_SENSOR_DB"),
        "alicevision_memory_gib": args.alicevision_memory_gib,
        "sam_python": str(sam_python), "sam_source": str(Path(sam_source).absolute()),
        "sam_checkpoint": str(Path(sam_checkpoint).absolute()),
        "sam_config": pick(args.sam_config, "CRISPMESH_SAM_CONFIG"),
        "sam_pythonpath": [p for p in pythonpath.split(os.pathsep) if p],
        "device": args.device, "threads": args.threads, "minimum_free_gib": args.minimum_free_gib,
        "envelope": args.envelope, "dark_threshold": threshold if threshold == "otsu" else int(threshold),
        "sam_multimask": args.sam_multimask, "sam_preserve_holes": args.sam_preserve_holes,
        "sam_automatic_cues": args.sam_automatic_cues,
        # The masks here always come from SAM; correct SAM 2.1 masks hold rows of small dark holes (up to about 2.5 %
        # of the mask on the test objects), as in crates/dense (SAM_HOLE_CLEANUP_BUDGET).
        "hole_cleanup_budget": SAM_HOLE_CLEANUP_BUDGET if args.hole_cleanup_budget is None else args.hole_cleanup_budget,
        "contrast_gamma": args.contrast_gamma, "clahe_clip": args.clahe_clip, "clahe_grid": args.clahe_grid,
        "initial_field_of_view": args.initial_field_of_view, "describer_types": args.describer_types,
        "describer_preset": args.describer_preset, "matching_method": args.matching_method,
        "random_seed": args.random_seed, "sfm_option": [str(v) for v in args.sfm_option],
        "timeouts": {"small": args.small_step_timeout, "sam": args.sam_timeout, "features": args.features_timeout,
                     "matching": args.matching_timeout, "sfm": args.sfm_timeout, "prepare": args.prepare_timeout},
        "gates": {"minimum_registered_fraction": args.minimum_registered_fraction,
                  "minimum_observations_per_view": args.minimum_observations_per_view,
                  "maximum_view_reprojection_p95_pixels": args.maximum_view_reprojection_p95,
                  "minimum_positive_depth_fraction": args.minimum_positive_depth_fraction,
                  "maximum_radius_spread_percent": args.maximum_radius_spread_percent,
                  "maximum_out_of_plane_percent": args.maximum_out_of_plane_percent,
                  "maximum_angular_gap_deg": args.maximum_angular_gap_deg,
                  "maximum_reversed_steps": args.maximum_reversed_steps,
                  "maximum_optical_axis_miss_percent": args.maximum_optical_axis_miss_percent,
                  "duplicate_step_deg": args.duplicate_step_deg},
        "keep_intermediates": args.keep_intermediates,
    }
    if cfg["device"] not in ("cpu", "mps") and f'"{cfg["device"]}"' not in (REPOSITORY / "scripts/turntable_mesh/segment.py").read_text():
        raise ValueError(f"--device {cfg['device']}: scripts/turntable_mesh/segment.py does not offer that device yet")
    load_calibration(cfg["calibration"])
    return cfg


def run(args):
    from .dense_pipeline import bounded

    cfg = resolve(args)
    out = Path(cfg["output"])
    if out.exists():
        raise FileExistsError(out)
    out.mkdir(parents=True)
    for name in ("logs", "sfm/extra", "work/matches", "work/features"):
        (out / name).mkdir(parents=True)
    (out / "frontend-config.json").write_text(json.dumps(cfg, indent=1) + "\n")
    events_path = Path(args.events).absolute() if args.events else out / "events.jsonl"
    events = EventLog(events_path, "cameras")
    commands = build_commands(cfg)
    base = {**os.environ, "PYTHONPATH": str(REPOSITORY), "PYTORCH_ENABLE_MPS_FALLBACK": "0",
            "OMP_NUM_THREADS": str(cfg["threads"]), "OPENBLAS_NUM_THREADS": str(cfg["threads"]),
            "VECLIB_MAXIMUM_THREADS": str(cfg["threads"])}
    report = {"schema": SCHEMA, "status": "running", "reasons": [], "warnings": [], "configuration": cfg,
              "events": str(events_path), "steps": [], "gates": None, "masks": None,
              "reference_geometry_used": False, "supplied_poses_used": False, "depth_used": False}
    started = time.monotonic()
    count = cfg["photo_count"]

    def save():
        report["seconds"] = time.monotonic() - started
        (out / "frontend.json").write_text(json.dumps(report, indent=1) + "\n")

    def artifact(stage, kind, path, label):
        relative = os.path.relpath(Path(path).resolve(), events_path.parent.resolve()).replace(os.sep, "/")
        events.emit("artifact", stage=stage, kind=kind, path=relative, label=label)

    def cancelled():
        return (events_path.parent / "cancel").exists()

    def step(stage, name, timeout, low, high, message, counter=None):
        """One bounded step; ``counter`` returns a 0..1 completion estimate used between ``low`` and ``high``."""
        free = shutil.disk_usage(out).free / 2**30
        if free < cfg["minimum_free_gib"]:
            raise StepFailed(f"{name}: only {free:.1f} GiB free; need {cfg['minimum_free_gib']} (--minimum-free-gib)")
        events.emit("progress", stage=stage, fraction=round(low, 4), message=message)
        print(f"[{stage}] {message} ...", flush=True)
        last = [low]

        def tick():
            if counter:
                fraction = round(low + (high - low) * min(max(counter(), 0), 1), 2)
                if fraction >= last[0] + 0.05:  # a handful of events per step, not one per photo
                    last[0] = fraction
                    events.emit("progress", stage=stage, fraction=fraction, message=message)

        entry = commands[name]
        log = out / "logs" / f"{len(report['steps']) + 1:02d}-{name}.log"
        result = bounded(entry["command"], log, timeout, {**base, **entry["environment"]}, tick, cancelled)
        result.update(name=name, stage=stage)
        report["steps"].append(result)
        save()
        if result["timed_out"] or result["cancelled"] or result["exit_code"]:
            reason = "cancelled" if result["cancelled"] else f"deadline of {timeout} s" if result["timed_out"] else f"exit code {result['exit_code']}"
            raise StepFailed(f"{name} failed ({reason}): {log.read_text(errors='replace')[-600:]}")
        events.emit("progress", stage=stage, fraction=round(high, 4), message=message + ": done")

    def files(folder, pattern):
        return lambda: len(list((out / folder).glob(pattern))) / count

    stage = "masks"
    try:
        timeouts = cfg["timeouts"]
        stage_started = time.monotonic()
        events.emit("stage_started", stage="masks")
        step("masks", "coarse", timeouts["small"], 0.0, 0.08, "Reading photos, coarse dark-object masks",
             files("work/coarse-masks", "*.png"))
        photo_map = json.loads((out / "photo-map.json").read_text())
        touching = [row["capture"] for row in photo_map["photos"] if row["touches_envelope"]]
        if touching:
            report["warnings"].append(f"coarse object region touches the envelope in {len(touching)} photos "
                                      f"(first {touching[0]}): object clipped by the frame or by --envelope")
        step("masks", "sam", timeouts["sam"], 0.08, 0.85, "Segmenting with SAM 2.1", files("work/sam/masks", "*.png"))
        step("masks", "cleanup", timeouts["small"], 0.85, 0.92, "Filling small dark holes")
        step("masks", "publish-masks", timeouts["small"], 0.92, 1.0, "Writing masks and contact sheet")
        masks = json.loads((out / "masks-report.json").read_text())
        report["masks"] = {k: v for k, v in masks.items() if k != "views"}
        if masks["views_dropping_over_3_percent_of_coarse"]:
            report["warnings"].append(f"SAM dropped more than 3% of the coarse dark region in "
                                      f"{len(masks['views_dropping_over_3_percent_of_coarse'])} photos (thin parts?)")
        events.emit("metric", stage="masks", name="mask_area_median_pixels", value=masks["median_area_pixels"])
        events.emit("metric", stage="masks", name="mask_area_median_fraction", value=round(masks["median_area_fraction"], 5))
        artifact("masks", "mask_sheet", out / "mask-contact-sheet.png", "Masks on photos (red: dark pixels left out)")
        events.emit("stage_finished", stage="masks", seconds=time.monotonic() - stage_started)

        stage = "cameras"
        stage_started = time.monotonic()
        events.emit("stage_started", stage="cameras")
        step("cameras", "contrast", timeouts["small"], 0.0, 0.04, "Contrast images for feature detection",
             files("work/contrast", "*.png"))
        step("cameras", "cameraInit-uncalibrated", timeouts["small"], 0.04, 0.05, "Listing views")
        step("cameras", "featureExtraction", timeouts["features"], 0.05, 0.30, "Detecting features",
             files("work/features", "*.feat"))
        scene = json.loads((out / "sfm/cameraInit-uncalibrated.sfm").read_text())
        intrinsic = scene["intrinsics"][0]
        width, height = int(intrinsic["width"]), int(intrinsic["height"])
        scaled = scale_calibration(load_calibration(cfg["calibration"]), width, height)
        (out / "sfm/calibrated-input.sfm").write_text(json.dumps(calibrated_scene(scene, scaled), indent=2) + "\n")
        (out / "sfm/expected-calibration.json").write_text(json.dumps(
            {"pixels": [scaled["fx"], scaled["fy"], scaled["cx"], scaled["cy"]], "k": scaled["k"]}, indent=1) + "\n")
        report["lens"] = {"width": width, "height": height, **scaled}
        step("cameras", "cameraInit", timeouts["small"], 0.30, 0.31, "Applying the declared lens")
        step("cameras", "imageMatching", timeouts["small"], 0.31, 0.32, "Choosing image pairs")
        step("cameras", "featureMatching", timeouts["matching"], 0.32, 0.60, "Matching features")
        step("cameras", "globalSfM", timeouts["sfm"], 0.60, 0.82, "Recovering cameras (global SfM, locked lens)")
        step("cameras", "audit", timeouts["small"], 0.82, 0.86, "Checking cameras")
        gates = json.loads((out / "sfm/gates.json").read_text())
        report["gates"] = gates
        ring = gates["ring"]
        metrics = {"registered_views": gates["registered_views"], "input_photos": gates["input_photos"],
                   "reprojection_median_pixels": gates["reprojection_pixels"] and round(gates["reprojection_pixels"]["median"], 4),
                   "reprojection_p95_pixels": gates["reprojection_pixels"] and round(gates["reprojection_pixels"]["p95"], 4),
                   "ring_radius_spread_percent": None if ring.get("degenerate") else round(ring["radius_spread_percent"], 3),
                   "ring_largest_gap_deg": None if ring.get("degenerate") else round(ring["largest_angular_gap_deg"], 2)}
        for name, value in metrics.items():
            if value is not None:
                events.emit("metric", stage="cameras", name=name, value=value)
        if not ring.get("degenerate") and ring["duplicate_pose_pairs"]:
            report["warnings"].append(f"consecutive photos with the same pose (turntable did not move?): {ring['duplicate_pose_pairs']}")
        if not gates["passed"]:
            report["status"] = "failed"
            report["reasons"] = gates["reasons"]
            save()
            artifact("cameras", "report", out / "frontend.json", "Front stage report (cameras rejected)")
            events.emit("error", stage="cameras", message="cameras rejected: " + "; ".join(gates["reasons"]))
            return report, 2
        step("cameras", "prepareDenseScene", timeouts["prepare"], 0.86, 0.95, "Undistorting photos",
             lambda: len(list((out / "native-prepared").glob("*.png"))) / max(gates["registered_views"], 1))
        step("cameras", "verify-prepared", timeouts["small"], 0.95, 0.96, "Checking undistorted photos")
        step("cameras", "inputs", timeouts["small"], 0.96, 0.98, "Writing dense inputs")
        step("cameras", "overlay", timeouts["small"], 0.98, 1.0, "Sparse points on photos")
        report["sparse_overlay"] = json.loads((out / "sparse-overlay.json").read_text())
        artifact("cameras", "sparse_overlay", out / "sparse-overlay.png", "Sparse points on undistorted photos and masks")
        report["intermediates_deleted"] = []
        if not cfg["keep_intermediates"]:
            for name in INTERMEDIATES:
                shutil.rmtree(out / name, ignore_errors=True)
                report["intermediates_deleted"].append(name)
            report["warnings"].append("sfm/final.sfm view paths point at deleted contrast images; "
                                      "downstream stages use native-prepared/ and inputs/ instead")
        report["outputs"] = {"inputs": str(out / "inputs"), "masks": str(out / "masks"), "scene": str(out / "sfm/final.sfm"),
                             "prepared": str(out / "native-prepared")}
        report["status"] = "complete"
        save()
        artifact("cameras", "report", out / "frontend.json", "Front stage report")
        events.emit("stage_finished", stage="cameras", seconds=time.monotonic() - stage_started)
        return report, 0
    except (StepFailed, ValueError, OSError, KeyError) as error:
        report["status"] = "failed"
        report["reasons"] = [f"{type(error).__name__}: {error}"]
        save()
        events.emit("error", stage=stage, message=str(error)[-600:])
        return report, 1


def parser():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--photos", type=Path, help="folder of original photos (png/jpg/tif), one turn of a turntable")
    p.add_argument("--calibration", type=Path, help="lens JSON (see docs/PHOTOS-TO-INPUTS.md)")
    p.add_argument("--output", type=Path, help="fresh output directory")
    p.add_argument("--events", type=Path, help="event log to append to (default <output>/events.jsonl)")
    p.add_argument("--internal-step", choices=INTERNAL_STEPS, help=argparse.SUPPRESS)
    tools = p.add_argument_group("tools (flag, then environment variable)")
    tools.add_argument("--python", help="interpreter with NumPy, SciPy, OpenCV, Pillow [CRISPMESH_PYTHON, else this one]")
    tools.add_argument("--alicevision", help="wrapper script or install prefix with the sparse tools [CRISPMESH_ALICEVISION]")
    tools.add_argument("--alicevision-dense", help="wrapper or prefix with prepareDenseScene [CRISPMESH_ALICEVISION_DENSE, else --alicevision]")
    tools.add_argument("--sensor-database", help="cameraSensors.db [CRISPMESH_ALICEVISION_SENSOR_DB, else <prefix>/share/aliceVision/]")
    tools.add_argument("--alicevision-memory-gib", type=float, default=4.0)
    tools.add_argument("--sam-python", help="interpreter with Torch and the SAM 2 dependencies [CRISPMESH_SAM_PYTHON]")
    tools.add_argument("--sam-source", help="SAM 2 source checkout containing sam2/ [CRISPMESH_SAM_SOURCE]")
    tools.add_argument("--sam-checkpoint", help="SAM 2.1 checkpoint file [CRISPMESH_SAM_CHECKPOINT]")
    tools.add_argument("--sam-config", help="SAM model config name [CRISPMESH_SAM_CONFIG, else segment.py's default: hiera tiny]")
    tools.add_argument("--sam-pythonpath", help="extra PYTHONPATH entries for the SAM interpreter [CRISPMESH_SAM_PYTHONPATH]")
    machine = p.add_argument_group("machine")
    machine.add_argument("--device", choices=("mps", "cpu", "cuda"), default="mps", help="SAM device")
    machine.add_argument("--threads", type=int, default=2)
    machine.add_argument("--minimum-free-gib", type=float, default=10.0, help="checked before every step")
    machine.add_argument("--keep-intermediates", action="store_true", help="keep photo copies, contrast images, features, matches")
    masks = p.add_argument_group("masks")
    masks.add_argument("--envelope", default="auto", help="where the object can be: 'auto' (whole frame) or x0,y0,x1,y1 in pixels, or as fractions if all <= 1")
    masks.add_argument("--dark-threshold", default="70", help="grey level below which a pixel is object (1..255), or 'otsu' per photo")
    masks.add_argument("--sam-multimask", action=argparse.BooleanOptionalAction, default=True)
    masks.add_argument("--sam-preserve-holes", action=argparse.BooleanOptionalAction, default=True)
    masks.add_argument("--sam-automatic-cues", action=argparse.BooleanOptionalAction, default=True)
    masks.add_argument("--hole-cleanup-budget", type=float, default=None,
                       help=f"largest fraction of the foreground the dark-hole fill may add (default {SAM_HOLE_CLEANUP_BUDGET}, as for SAM masks in crates/dense)")
    cameras = p.add_argument_group("camera recovery")
    cameras.add_argument("--contrast-gamma", type=float, default=0.5, help="gamma for the feature images; 1 disables")
    cameras.add_argument("--clahe-clip", type=float, default=2.0, help="CLAHE clip limit for the feature images; 0 disables")
    cameras.add_argument("--clahe-grid", type=int, default=8)
    cameras.add_argument("--initial-field-of-view", type=float, default=45.0, help="only for the placeholder lens before the declared one is applied")
    cameras.add_argument("--describer-types", default="sift")
    cameras.add_argument("--describer-preset", default="normal")
    cameras.add_argument("--matching-method", default="Exhaustive", help="AliceVision imageMatching method")
    cameras.add_argument("--random-seed", type=int, default=0)
    cameras.add_argument("--sfm-option", action="append", default=[], metavar="ARG", help="extra argument appended to globalSfM; repeat for each token")
    deadlines = p.add_argument_group("deadlines in seconds (whole process group is stopped)")
    deadlines.add_argument("--sam-timeout", type=int, default=1800)
    deadlines.add_argument("--features-timeout", type=int, default=1800)
    deadlines.add_argument("--matching-timeout", type=int, default=1800)
    deadlines.add_argument("--sfm-timeout", type=int, default=900)
    deadlines.add_argument("--prepare-timeout", type=int, default=600)
    deadlines.add_argument("--small-step-timeout", type=int, default=600, help="every other step")
    gates = p.add_argument_group("quality gates (a failed gate means exit code 2 and no dense inputs)")
    gates.add_argument("--minimum-registered-fraction", type=float, default=0.8)
    gates.add_argument("--minimum-observations-per-view", type=int, default=20)
    gates.add_argument("--maximum-view-reprojection-p95", type=float, default=4.0, help="pixels, per view")
    gates.add_argument("--minimum-positive-depth-fraction", type=float, default=0.999,
                       help="share of sparse observations that must lie in front of their camera; 1 rejects a single stray point")
    gates.add_argument("--maximum-radius-spread-percent", type=float, default=5.0, help="(max - min) camera distance from the ring axis, percent of the radius")
    gates.add_argument("--maximum-out-of-plane-percent", type=float, default=5.0, help="largest camera distance from the ring plane, percent of the radius")
    gates.add_argument("--maximum-angular-gap-deg", type=float, default=30.0, help="largest gap between neighbouring cameras; 360 accepts a partial turn")
    gates.add_argument("--maximum-reversed-steps", type=int, default=0, help="capture steps allowed to run against the turning direction")
    gates.add_argument("--maximum-optical-axis-miss-percent", type=float, default=25.0, help="how far an optical axis may pass from the ring axis, percent of the radius")
    gates.add_argument("--duplicate-step-deg", type=float, default=0.5, help="smaller steps are reported as duplicate frames (warning only)")
    return p


def main(argv=None):
    p = parser()
    args = p.parse_args(argv)
    if args.internal_step:
        internal_step(args.internal_step, args.output)
        return 0
    for name in ("photos", "calibration", "output"):
        if getattr(args, name) is None:
            p.error(f"--{name} is required")
    try:
        report, code = run(args)
    except (ValueError, FileExistsError) as error:
        p.error(str(error))
    gates = report.get("gates") or {}
    print(json.dumps({"status": report["status"], "reasons": report["reasons"], "warnings": report["warnings"],
                      "registered_views": gates.get("registered_views"), "reprojection_pixels": gates.get("reprojection_pixels"),
                      "seconds": report.get("seconds"), "report": str(Path(report["configuration"]["output"]) / "frontend.json")},
                     indent=2))
    return code


if __name__ == "__main__":
    sys.exit(main())
