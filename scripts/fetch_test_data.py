#!/usr/bin/env python3
"""Explicit, bounded download of checksum-pinned measured stereo test data."""

import argparse
import hashlib
import io
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import struct
import subprocess
import tempfile
import zlib
from urllib.parse import urlparse
from urllib.request import HTTPRedirectHandler, build_opener


REPO = Path(__file__).resolve().parents[1]
MANIFEST = REPO / "tests/datasets/middlebury_piano_perfect.json"
DEFAULT_ROOT = REPO / ".local-tools/test-data/middlebury-2014"
DEFAULT_2003_ROOT = REPO / ".local-tools/test-data/middlebury-2003"
MANIFEST_2003 = REPO / "tests/datasets/middlebury_2003.json"
MANIFEST_2001 = REPO / "tests/datasets/middlebury_2001_venus.json"
MANIFEST_ETH3D = REPO / "tests/datasets/eth3d_lowres.json"
MAX_FILE_BYTES = 24_000_000
MAX_TOTAL_BYTES = 40_000_000
PINNED_PREFIX = "https://raw.githubusercontent.com/ztliu62/stereo3D/91a4aef04c9d46955073281f4555278819b6b589/images/Piano-perfect/"
PINNED_2003_PREFIX = "https://raw.githubusercontent.com/beaupreda/semi-global-matching/6643cf4b6a5ab197aace8b766c6ab139e281ae8e/"
PINNED_2001_PREFIX = "https://raw.githubusercontent.com/Soumyabrata/Stereo-Matching/09be6059026e4f831d5360083fcb02afddcccf63/Data/venus/"


def validate_manifest(manifest):
    entries = manifest["files"]
    if not isinstance(entries, list) or len(entries) != 4:
        raise ValueError("expected exactly four pinned fixture files")
    expected = {"calib.txt", "im0.png", "im1.png", "disp0.pfm"}
    names = set()
    total = 0
    for entry in entries:
        path = PurePosixPath(entry["path"])
        if path.parts != ("Piano-perfect", path.name) or path.name not in expected:
            raise ValueError(f"unsafe or unexpected path: {entry['path']}")
        names.add(path.name)
        size = entry["size"]
        if type(size) is not int or not 0 < size <= MAX_FILE_BYTES:
            raise ValueError(f"invalid size for {path}")
        total += size
        if not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]):
            raise ValueError(f"invalid SHA256 for {path}")
        if entry["url"] != PINNED_PREFIX + path.name:
            raise ValueError(f"unapproved URL for {path}")
    if names != expected or total > MAX_TOTAL_BYTES:
        raise ValueError("fixture set is incomplete or exceeds budget")
    return entries


def validate_2003_manifest(manifest):
    expected = {f"{scene}/{name}" for scene in ("cones", "teddy")
                for name in ("im2.png", "im6.png", "disp2.png")}
    entries = manifest["files"]
    if not isinstance(entries, list) or len(entries) != len(expected):
        raise ValueError("expected six Middlebury 2003 files")
    seen = set()
    total = 0
    for entry in entries:
        path = entry["path"]
        if path not in expected or entry["url"] != PINNED_2003_PREFIX + path:
            raise ValueError(f"unsafe or unapproved Middlebury 2003 file: {path}")
        seen.add(path)
        if type(entry["size"]) is not int or not 0 < entry["size"] < 1_000_000:
            raise ValueError(f"invalid file size: {path}")
        total += entry["size"]
        if not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]):
            raise ValueError(f"invalid SHA256: {path}")
    if seen != expected or total > 2_000_000:
        raise ValueError("incomplete or oversized Middlebury 2003 fixture")
    derived = manifest["derived"]
    if {entry["path"] for entry in derived} != {"cones/disp0GT.pfm", "teddy/disp0GT.pfm"}:
        raise ValueError("unexpected derived disparity files")
    for entry in derived:
        scene = entry["path"].split("/")[0]
        if (entry["source"] != f"{scene}/disp2.png" or entry["size"] != 675016
                or (entry["width"], entry["height"], entry["divisor"]) != (450, 375, 4)
                or not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"])):
            raise ValueError("invalid derived disparity manifest")
    return entries


def validate_2001_manifest(manifest):
    leafs = {"venus/im2.png": "imL.png", "venus/im6.png": "imR.png",
             "venus/disp2.png": "groundtruth.png"}
    entries = manifest["files"]
    if len(entries) != 3 or {item["path"] for item in entries} != set(leafs):
        raise ValueError("unexpected Middlebury 2001 file set")
    for item in entries:
        if (item["url"] != PINNED_2001_PREFIX + leafs[item["path"]]
                or type(item["size"]) is not int or not 0 < item["size"] < 500_000
                or not re.fullmatch(r"[0-9a-f]{64}", item["sha256"])):
            raise ValueError("invalid Middlebury 2001 source pin")
    derived = manifest["derived"]
    if len(derived) != 1:
        raise ValueError("unexpected Middlebury 2001 derived set")
    item = derived[0]
    if (item["source"] != "venus/disp2.png" or item["path"] != "venus/disp0GT.pfm"
            or (item["width"], item["height"], item["divisor"]) != (434, 383, 8)
            or item["size"] != 664904
            or not re.fullmatch(r"[0-9a-f]{64}", item["sha256"])):
        raise ValueError("invalid Middlebury 2001 derived pin")
    return entries


def validate_eth3d_manifest(manifest):
    archives = manifest["archives"]
    expected_archives = {"two_view_training.7z", "two_view_training_gt.7z"}
    if {item["name"] for item in archives} != expected_archives or len(archives) != 2:
        raise ValueError("unexpected ETH3D archive set")
    for item in archives:
        if (item["url"] != f"https://www.eth3d.net/data/{item['name']}"
                or type(item["size"]) is not int or not 0 < item["size"] < 20_000_000
                or not re.fullmatch(r"[0-9a-f]{64}", item["sha256"])):
            raise ValueError("invalid ETH3D archive pin")
    scenes = ("delivery_area_1s", "forest_1s", "playground_1s")
    names = ("calib.txt", "im0.png", "im1.png", "disp0GT.pfm", "mask0nocc.png")
    expected = {f"{scene}/{name}" for scene in scenes for name in names}
    files = manifest["files"]
    if len(files) != 15 or {item["path"] for item in files} != expected:
        raise ValueError("unexpected ETH3D extracted file set")
    total = 0
    for item in files:
        name = item["path"].split("/")[1]
        source = "two_view_training_gt.7z" if name in ("disp0GT.pfm", "mask0nocc.png") else "two_view_training.7z"
        if (item["archive"] != source or type(item["size"]) is not int
                or not 0 < item["size"] <= 2_000_000
                or not re.fullmatch(r"[0-9a-f]{64}", item["sha256"])):
            raise ValueError("invalid ETH3D extracted file pin")
        total += item["size"]
    if total > 6_000_000:
        raise ValueError("ETH3D selected data exceeds budget")
    return archives, files


def file_matches(path, entry):
    if path.is_symlink() or not path.is_file() or path.stat().st_size != entry["size"]:
        return False
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest() == entry["sha256"]


def parse_calibration(data):
    values = {}
    for line in data.decode("ascii").splitlines():
        if "=" not in line:
            raise ValueError("malformed calibration line")
        key, value = line.split("=", 1)
        if key in values:
            raise ValueError(f"duplicate calibration key: {key}")
        values[key] = value
    for key in ("cam0", "cam1", "doffs", "baseline", "width", "height", "ndisp"):
        if key not in values:
            raise ValueError(f"missing calibration key: {key}")
    cameras = []
    for key in ("cam0", "cam1"):
        if not (values[key].startswith("[") and values[key].endswith("]")):
            raise ValueError(f"malformed camera matrix: {key}")
        rows = [[float(v) for v in row.split()] for row in values[key][1:-1].split(";")]
        if len(rows) != 3 or any(len(row) != 3 for row in rows):
            raise ValueError(f"malformed camera matrix: {key}")
        if not all(math.isfinite(value) for row in rows for value in row):
            raise ValueError(f"nonfinite camera matrix: {key}")
        if (rows[0][0] <= 0 or abs(rows[0][0] - rows[1][1]) > 0.01
                or abs(rows[0][1]) > 0.01 or abs(rows[1][0]) > 0.01
                or abs(rows[2][0]) > 0.01 or abs(rows[2][1]) > 0.01
                or abs(rows[2][2] - 1) > 0.01):
            raise ValueError(f"nonrectified camera matrix: {key}")
        cameras.append(rows)
    width, height, ndisp = (int(values[key]) for key in ("width", "height", "ndisp"))
    if not (0 < width <= 5000 and 0 < height <= 5000 and 0 < ndisp <= 1000):
        raise ValueError("invalid calibration dimensions")
    baseline = float(values["baseline"])
    doffs = float(values["doffs"])
    if (not math.isfinite(baseline) or not math.isfinite(doffs) or baseline <= 0
            or cameras[0][0][0] <= 0 or cameras[1][0][0] <= 0
            or abs(cameras[0][0][0] - cameras[1][0][0]) > 0.01
            or abs((cameras[1][0][2] - cameras[0][0][2]) - doffs) > 0.01):
        raise ValueError("inconsistent stereo calibration")
    return {"width": width, "height": height, "ndisp": ndisp,
            "baseline_mm": baseline, "doffs": doffs, "fx": cameras[0][0][0]}


def read_pfm_header(path):
    with path.open("rb") as source:
        magic = source.readline(256).strip()
        dimensions = source.readline(256).split()
        scale = source.readline(256).strip()
        if magic != b"Pf" or len(dimensions) != 2:
            raise ValueError("expected single-channel PFM disparity")
        width, height = map(int, dimensions)
        endian_scale = float(scale)
        if not (0 < width <= 5000 and 0 < height <= 5000
                and math.isfinite(endian_scale) and endian_scale != 0):
            raise ValueError("invalid PFM header")
        payload_offset = source.tell()
    if path.stat().st_size - payload_offset != width * height * 4:
        raise ValueError("PFM payload length does not match dimensions")
    # Middlebury uses the PFM scale sign for byte order; magnitude is ignored.
    return {"width": width, "height": height, "little_endian": endian_scale < 0,
            "payload_offset": payload_offset}


def png_dimensions(path):
    with path.open("rb") as source:
        header = source.read(24)
    if header[:16] != b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR":
        raise ValueError("invalid PNG header")
    return struct.unpack(">II", header[16:24])


def read_gray8_png(data, expected_dimensions=(450, 375)):
    """Decode pinned legacy disparity PNGs without a third-party dependency."""
    if len(data) > 100_000 or data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("invalid or oversized disparity PNG")
    cursor = 8
    compressed = bytearray()
    dimensions = None
    seen_end = False
    while cursor + 12 <= len(data):
        length = struct.unpack_from(">I", data, cursor)[0]
        kind = data[cursor + 4:cursor + 8]
        end = cursor + 12 + length
        if length > 100_000 or end > len(data):
            raise ValueError("invalid PNG chunk length")
        payload = data[cursor + 8:cursor + 8 + length]
        crc = struct.unpack_from(">I", data, cursor + 8 + length)[0]
        if zlib.crc32(kind + payload) != crc:
            raise ValueError("PNG chunk CRC mismatch")
        if kind == b"IHDR":
            if dimensions is not None or length != 13:
                raise ValueError("invalid PNG image header")
            width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", payload)
            if (width, height) != expected_dimensions or (depth, color, compression, filtering, interlace) != (8, 0, 0, 0, 0):
                raise ValueError("unexpected gray8 PNG dimensions or encoding")
            dimensions = width, height
        elif kind == b"IDAT":
            compressed.extend(payload)
        elif kind == b"IEND":
            if length != 0:
                raise ValueError("invalid PNG end chunk")
            seen_end = True
            cursor = end
            break
        cursor = end
    if dimensions is None or not compressed or not seen_end or cursor != len(data):
        raise ValueError("missing PNG image data")
    width, height = dimensions
    decoder = zlib.decompressobj()
    raw = decoder.decompress(bytes(compressed), (width + 1) * height + 1)
    if len(raw) != (width + 1) * height or not decoder.eof or decoder.unused_data:
        raise ValueError("invalid decompressed PNG size")
    rows = []
    prior = bytearray(width)
    for y in range(height):
        start = y * (width + 1)
        method = raw[start]
        if method > 4:
            raise ValueError("invalid PNG row filter")
        row = bytearray(raw[start + 1:start + 1 + width])
        for x in range(width):
            left = row[x - 1] if x else 0
            above = prior[x]
            upper_left = prior[x - 1] if x else 0
            if method == 1:
                predictor = left
            elif method == 2:
                predictor = above
            elif method == 3:
                predictor = (left + above) // 2
            elif method == 4:
                base = left + above - upper_left
                distances = (abs(base - left), abs(base - above), abs(base - upper_left))
                predictor = (left, above, upper_left)[distances.index(min(distances))]
            else:
                predictor = 0
            row[x] = (row[x] + predictor) & 255
        rows.append(row)
        prior = row
    return rows


def disparity_pfm_bytes(source_png, dimensions=(450, 375), divisor=4):
    rows = read_gray8_png(source_png, dimensions)
    width, height = dimensions
    output = io.BytesIO()
    output.write(f"Pf\n{width} {height}\n-1.0\n".encode("ascii"))
    for row in reversed(rows):
        output.write(struct.pack(f"<{width}f", *(value / divisor if value else math.inf for value in row)))
    return output.getvalue()


def verify_or_create_derived(root, entry, create=False):
    source = root / entry["source"]
    target = root / entry["path"]
    if target.is_symlink() or source.is_symlink():
        raise ValueError("refusing derived-file symlink")
    expected = disparity_pfm_bytes(source.read_bytes(), (entry["width"], entry["height"]), entry["divisor"])
    checksum = hashlib.sha256(expected).hexdigest()
    if entry.get("sha256") != checksum or entry.get("size") != len(expected):
        raise ValueError("derived PFM does not match pinned manifest")
    if target.exists():
        if target.read_bytes() != expected:
            raise ValueError(f"existing derived PFM differs: {target}")
        return "verified"
    if not create:
        raise ValueError(f"missing derived PFM: {target}")
    fd, temporary = tempfile.mkstemp(prefix=".derived-", dir=target.parent)
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(expected)
        os.replace(temporary, target)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
    return "generated"


def verify_scene(root):
    scene = root / "Piano-perfect"
    calibration = parse_calibration((scene / "calib.txt").read_bytes())
    expected = (calibration["width"], calibration["height"])
    for name in ("im0.png", "im1.png"):
        if png_dimensions(scene / name) != expected:
            raise ValueError(f"image/calibration dimension mismatch: {name}")
    disparity = read_pfm_header(scene / "disp0.pfm")
    if (disparity["width"], disparity["height"]) != expected:
        raise ValueError("disparity/calibration dimension mismatch")
    return calibration


class HttpsOnlyRedirects(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        parsed = urlparse(newurl)
        if parsed.scheme != "https" or parsed.hostname not in ("raw.githubusercontent.com", "www.eth3d.net"):
            raise ValueError("download redirect left pinned HTTPS host")
        return super().redirect_request(request, fp, code, msg, headers, newurl)


def fetch_one(root, entry, opener):
    path = root / entry.get("path", entry.get("name"))
    if path.is_symlink():
        raise ValueError(f"refusing symlink: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.parent.is_symlink():
        raise ValueError(f"refusing symlink directory: {path.parent}")
    if path.exists():
        if file_matches(path, entry):
            return "verified"
        raise ValueError(f"existing file has wrong checksum; inspect or remove it: {path}")
    fd, temporary = tempfile.mkstemp(prefix=".download-", dir=path.parent)
    digest = hashlib.sha256()
    count = 0
    try:
        with os.fdopen(fd, "wb") as output, opener.open(entry["url"], timeout=30) as response:
            while chunk := response.read(1024 * 1024):
                count += len(chunk)
                if count > entry["size"]:
                    raise ValueError(f"download exceeds pinned size: {path}")
                digest.update(chunk)
                output.write(chunk)
        if count != entry["size"] or digest.hexdigest() != entry["sha256"]:
            raise ValueError(f"download does not match pinned checksum: {path}")
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
    return "downloaded"


def extract_pinned_member(root, entry, archive):
    """Stream one named archive member to a checked file; never extract paths."""
    path = root / entry["path"]
    if path.is_symlink():
        raise ValueError(f"refusing symlink: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.parent.is_symlink():
        raise ValueError(f"refusing symlink directory: {path.parent}")
    if path.exists():
        if file_matches(path, entry):
            return "verified"
        raise ValueError(f"existing extracted file differs: {path}")
    seven_zip = shutil.which("7z") or shutil.which("7zz")
    if not seven_zip:
        raise RuntimeError("7z or 7zz is required to unpack selected ETH3D members")
    fd, temporary = tempfile.mkstemp(prefix=".extract-", dir=path.parent)
    process = None
    try:
        digest = hashlib.sha256()
        count = 0
        process = subprocess.Popen([seven_zip, "x", "-so", "-bd", str(archive), entry["path"]],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        with os.fdopen(fd, "wb") as output:
            while chunk := process.stdout.read(64 * 1024):
                count += len(chunk)
                if count > entry["size"]:
                    process.kill()
                    raise ValueError(f"archive member exceeds pinned size: {entry['path']}")
                digest.update(chunk)
                output.write(chunk)
        if process.wait() != 0 or count != entry["size"] or digest.hexdigest() != entry["sha256"]:
            raise ValueError(f"archive member does not match pinned checksum: {entry['path']}")
        os.replace(temporary, path)
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
        if os.path.exists(temporary):
            os.unlink(temporary)
    return "extracted"


def verify_legacy_scene(root, scene, dimensions):
    for name in ("im2.png", "im6.png", "disp2.png"):
        if png_dimensions(root / scene / name) != dimensions:
            raise ValueError(f"Middlebury dimension mismatch: {scene}/{name}")
    header = read_pfm_header(root / scene / "disp0GT.pfm")
    if (header["width"], header["height"]) != dimensions:
        raise ValueError(f"Middlebury PFM dimension mismatch: {scene}")


def verify_eth3d_scene(root, scene):
    directory = root / scene
    calibration = parse_calibration((directory / "calib.txt").read_bytes())
    expected = calibration["width"], calibration["height"]
    for name in ("im0.png", "im1.png", "mask0nocc.png"):
        if png_dimensions(directory / name) != expected:
            raise ValueError(f"ETH3D dimension mismatch: {scene}/{name}")
    header = read_pfm_header(directory / "disp0GT.pfm")
    if (header["width"], header["height"]) != expected:
        raise ValueError(f"ETH3D PFM dimension mismatch: {scene}")
    return calibration


def selected_root(dataset, override=None):
    if override is not None:
        return override.expanduser().resolve()
    base = Path(os.environ.get("CRISPMESH_TEST_DATA_ROOT", REPO / ".local-tools/test-data"))
    suffix = {"piano": "middlebury-2014", "middlebury2003": "middlebury-2003",
              "middlebury2001": "middlebury-2001", "eth3d": "eth3d"}[dataset]
    return (base / suffix).expanduser().resolve()


def check_storage_budget(root, anticipated_bytes):
    existing = root
    while not existing.exists():
        existing = existing.parent
    free = shutil.disk_usage(existing).free
    if free - anticipated_bytes < 10 * 1024 ** 3:
        raise RuntimeError("fixture download would leave less than 10 GiB free")
    print(f"storage check: {free // (1024 ** 3)} GiB free; "
          f"at most {anticipated_bytes // (1024 ** 2) + 1} MiB planned")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fetch", action="store_true", help="explicitly download missing data")
    parser.add_argument("--dataset", choices=("piano", "middlebury2003", "middlebury2001", "eth3d"), default="piano")
    parser.add_argument("--root", type=Path, help="override selected dataset root")
    parser.add_argument("--allow-noncommercial-data", action="store_true",
                        help="acknowledge ETH3D CC BY-NC-SA 4.0 data terms for local use")
    args = parser.parse_args()
    if args.dataset == "eth3d" and not args.allow_noncommercial_data:
        parser.error("ETH3D requires --allow-noncommercial-data; see docs/TEST-DATA.md")
    root = selected_root(args.dataset, args.root)
    if args.dataset == "piano":
        manifest = json.loads(MANIFEST.read_text())
        entries = validate_manifest(manifest)
        anticipated = sum(item["size"] for item in entries)
    elif args.dataset == "middlebury2003":
        manifest = json.loads(MANIFEST_2003.read_text())
        entries = validate_2003_manifest(manifest)
        anticipated = sum(item["size"] for item in entries + manifest["derived"])
    elif args.dataset == "middlebury2001":
        manifest = json.loads(MANIFEST_2001.read_text())
        entries = validate_2001_manifest(manifest)
        anticipated = sum(item["size"] for item in entries + manifest["derived"])
    else:
        manifest = json.loads(MANIFEST_ETH3D.read_text())
        archives, files = validate_eth3d_manifest(manifest)
        anticipated = sum(item["size"] for item in archives + files)
    if args.fetch:
        check_storage_budget(root, anticipated)
    opener = build_opener(HttpsOnlyRedirects())
    source_entries = archives if args.dataset == "eth3d" else entries
    for entry in source_entries:
        path = root / entry.get("path", entry.get("name"))
        result = fetch_one(root, entry, opener) if args.fetch else (
            "verified" if file_matches(path, entry) else "missing or checksum mismatch")
        print(f"{result}: {path}")
        if result == "missing or checksum mismatch":
            raise SystemExit(1)
    if args.dataset == "piano":
        print(f"scene geometry verified: {verify_scene(root)}")
    elif args.dataset in ("middlebury2001", "middlebury2003"):
        for entry in manifest["derived"]:
            result = verify_or_create_derived(root, entry, create=args.fetch)
            print(f"{result}: {root / entry['path']}")
        scenes = (("venus", (434, 383)),) if args.dataset == "middlebury2001" else (
            ("cones", (450, 375)), ("teddy", (450, 375)))
        for scene, dimensions in scenes:
            verify_legacy_scene(root, scene, dimensions)
            print(f"scene geometry verified: {scene}, {dimensions[0]}x{dimensions[1]}, disparity pixels only")
    else:
        for entry in files:
            path = root / entry["path"]
            result = extract_pinned_member(root, entry, root / entry["archive"]) if args.fetch else (
                "verified" if file_matches(path, entry) else "missing or checksum mismatch")
            print(f"{result}: {path}")
            if result == "missing or checksum mismatch":
                raise SystemExit(1)
        for scene in ("delivery_area_1s", "forest_1s", "playground_1s"):
            print(f"scene geometry verified: {scene}, {verify_eth3d_scene(root, scene)}")


if __name__ == "__main__":
    main()
