"""Downloads one example object (photos and calibration) from its dataset repository.

    python scripts/ci/fetch-example.py <manifest url> <object name> <out dir>

Writes <out>/rgb/<photo>.png and <out>/<calibration file>, each checked against the manifest's
size and SHA-256 (the same manifest the app reads). Prints the dataset's attribution.
"""

import hashlib
import json
import sys
import urllib.request
from pathlib import Path

manifest_url, name, out = sys.argv[1], sys.argv[2], Path(sys.argv[3])
base = manifest_url.rsplit("/", 1)[0] + "/"


def get(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "crispmesh-app-checks"}), timeout=120) as response:
        return response.read()


manifest = json.loads(get(manifest_url))
assert manifest.get("schema") == "crispmesh_example_objects_v1", "not a list of example objects"
row = next(o for o in manifest["objects"] if o["name"] == name)
calibration = next(c for c in manifest["calibration_files"] if c["path"] == row["calibration"])
(out / "rgb").mkdir(parents=True, exist_ok=True)
for entry, target in [(calibration, out / calibration["path"].rsplit("/", 1)[1])] + [(f, out / "rgb" / f["path"].rsplit("/", 1)[1]) for f in row["files"]]:
    data = get(base + entry["path"])
    if len(data) != entry["size"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
        sys.exit(f"{entry['path']}: size or SHA-256 differs from the manifest")
    target.write_bytes(data)
print(f"{row.get('title', name)}: {len(row['files'])} photos and {calibration['path']} in {out}")
print(manifest["attribution"])
