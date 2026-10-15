#!/usr/bin/env python3
"""Validate a pinned MapAnything code/weight lock; never download weights."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
from urllib.request import urlopen


LOCK_PATH = Path(__file__).with_name("mapanything_lock.json")
CODE_REPO = "facebookresearch/map-anything"
MODEL_REPO = "facebook/map-anything-apache"
ARTIFACT = "model.safetensors"
CODE_REVISION = "3d10cf7a3016fc0f9bb13a071ee66c47b10be0d9"
MODEL_REVISION = "00f9c245bbcb60522d1ed7f9e9d88462c6e3f38a"
CODE_LICENSE_SHA256 = "a172097153c8dec07f872bf749f4f856351b0690fde58ab2c4f570fc1d354931"
CARD_SHA256 = "1481978d474172ee1acac213eddaf3572e54dba369190d898c8578f26097f1d2"
CONFIG_SHA256 = "65701d09d99ed37a21d295f0d138978b3d584ab3bccdbcb4a2853da212b676c5"
ARTIFACT_SHA256 = "fa06c0fdccefc5048e072c85935d5789b1e36b307f3859033c17f9dcb9fd5201"
ARTIFACT_BYTES = 4_914_062_480
SHA = re.compile(r"[0-9a-f]{64}\Z")
REV = re.compile(r"[0-9a-f]{40}\Z")
MAX_METADATA_BYTES = 2_000_000
RESERVE_BYTES = 10 * 1024 ** 3


def _exact(value: object, expected: set[str], label: str) -> dict:
    if not isinstance(value, dict) or set(value) != expected:
        raise ValueError(f"{label}: unexpected or missing fields")
    return value


def validate_lock(lock: object) -> dict:
    root = _exact(lock, {"schema", "code", "model"}, "lock")
    if root["schema"] != "crispmesh_mapanything_artifact_lock_v1":
        raise ValueError("unsupported lock schema")
    code = _exact(root["code"], {"repo", "revision", "license", "license_path", "license_sha256"}, "code")
    model = _exact(root["model"], {"repo", "revision", "card_license", "card_sha256", "config_sha256", "gated", "artifact"}, "model")
    artifact = _exact(model["artifact"], {"path", "bytes", "sha256"}, "artifact")
    if (code["repo"] != CODE_REPO or code["license"] != "Apache-2.0" or
            code["license_path"] != "LICENSE" or model["repo"] != MODEL_REPO or
            model["card_license"] != "apache-2.0" or model["gated"] is not False or
            artifact["path"] != ARTIFACT):
        raise ValueError("code/model/license allowlist mismatch")
    for name, value in (("code revision", code["revision"]), ("model revision", model["revision"])):
        if not isinstance(value, str) or REV.fullmatch(value) is None:
            raise ValueError(f"{name} must be a commit SHA")
    for name, value in (("code license", code["license_sha256"]),
                        ("model card", model["card_sha256"]),
                        ("model config", model["config_sha256"]),
                        ("artifact", artifact["sha256"])):
        if not isinstance(value, str) or SHA.fullmatch(value) is None:
            raise ValueError(f"{name} must have SHA-256")
    if (code["revision"] != CODE_REVISION or model["revision"] != MODEL_REVISION or
            code["license_sha256"] != CODE_LICENSE_SHA256 or
            model["card_sha256"] != CARD_SHA256 or model["config_sha256"] != CONFIG_SHA256 or
            artifact["sha256"] != ARTIFACT_SHA256 or type(artifact["bytes"]) is not int or
            artifact["bytes"] != ARTIFACT_BYTES):
        raise ValueError("pin differs from the reviewed exact allowlist")
    return root


def read_small(url: str) -> bytes:
    with urlopen(url, timeout=20) as response:
        if response.headers.get("Content-Length") and int(response.headers["Content-Length"]) > MAX_METADATA_BYTES:
            raise ValueError("remote metadata too large")
        data = response.read(MAX_METADATA_BYTES + 1)
    if len(data) > MAX_METADATA_BYTES:
        raise ValueError("remote metadata too large")
    return data


def verify_live(lock: dict, fetch=read_small) -> dict:
    """Read only HF JSON/card/config and GitHub LICENSE; never resolve weights."""
    lock = validate_lock(lock)
    code, model = lock["code"], lock["model"]
    url = f"https://raw.githubusercontent.com/{CODE_REPO}/{code['revision']}/LICENSE"
    if hashlib.sha256(fetch(url)).hexdigest() != code["license_sha256"]:
        raise ValueError("pinned code license file differs")
    base = f"https://huggingface.co/{MODEL_REPO}/raw/{model['revision']}"
    raw_config = None
    for name, expected in (("README.md", model["card_sha256"]), ("config.json", model["config_sha256"])):
        raw = fetch(f"{base}/{name}")
        if hashlib.sha256(raw).hexdigest() != expected:
            raise ValueError(f"pinned {name} differs")
        if name == "config.json":
            raw_config = raw
    config = json.loads(raw_config)
    if not isinstance(config, dict) or config.get("pretrained_checkpoint_path") is not None:
        raise ValueError("pinned model config has unexpected checkpoint loading path")
    metadata = json.loads(fetch(f"https://huggingface.co/api/models/{MODEL_REPO}/revision/{model['revision']}?blobs=true"))
    if (metadata.get("sha") != model["revision"] or metadata.get("gated") is not False or
            metadata.get("cardData", {}).get("license") != model["card_license"]):
        raise ValueError("HF revision, gate, or model-card license differs")
    matches = [item for item in metadata.get("siblings", []) if item.get("rfilename") == ARTIFACT]
    if len(matches) != 1 or matches[0].get("size") != model["artifact"]["bytes"] or matches[0].get("lfs", {}).get("sha256") != model["artifact"]["sha256"]:
        raise ValueError("HF artifact digest/size differs")
    return {"status": "metadata_verified_no_weights_downloaded", "code_revision": code["revision"],
            "model_revision": model["revision"], "artifact_sha256": model["artifact"]["sha256"]}


def placement_check(destination: Path, *, profile: str, artifact_bytes: int,
                    free_bytes: int | None = None, storage_root: Path = Path("/mnt/akademie_storage"),
                    mac_mount: Path = Path("/Volumes/backups"),
                    workspace: Path = Path(__file__).resolve().parents[2],
                    internal_free_bytes: int | None = None) -> dict:
    """Preflight only; does not create directories or download the artifact."""
    if type(artifact_bytes) is not int or artifact_bytes != ARTIFACT_BYTES:
        raise ValueError("placement requires exact reviewed artifact size")
    destination = Path(destination)
    if destination.exists() or destination.is_symlink():
        raise ValueError("destination must be fresh")
    parent = destination.parent.resolve(strict=True)
    if destination.name != ARTIFACT or not parent.is_dir():
        raise ValueError("destination must be a fresh model.safetensors under an existing directory")
    if profile == "vps_storage":
        if not parent.is_relative_to(storage_root.resolve(strict=True)):
            raise ValueError("VPS weights must be placed on canonical storage, not fast scratch")
        free_root = parent
    elif profile == "mac_local":
        allowed = Path(__file__).resolve().parents[2] / ".local-tools" / "models"
        if not parent.is_relative_to(allowed.resolve(strict=True)):
            raise ValueError("Mac weights must be placed under .local-tools/models")
        free_root = parent
    elif profile == "mac_external":
        mount = Path(mac_mount)
        if not mount.is_mount():
            raise ValueError("Mac external model volume is not actually mounted")
        mount = mount.resolve(strict=True)
        workspace = Path(workspace).resolve(strict=True)
        external_device = mount.stat().st_dev
        if external_device == workspace.stat().st_dev:
            raise ValueError("Mac external model volume shares the internal device")
        expected = mount / "ai" / "crispmesh" / "map-anything-apache" / MODEL_REVISION
        if (destination.parent != expected or parent != expected.resolve(strict=True) or
                not parent.is_relative_to(mount) or parent.stat().st_dev != external_device):
            raise ValueError("Mac weights require exact versioned path on mounted backups volume")
        internal_free = (shutil.disk_usage(workspace).free if internal_free_bytes is None
                         else internal_free_bytes)
        if internal_free < RESERVE_BYTES:
            raise ValueError("Mac internal disk below 10 GiB reserve")
        free_root = mount
    else:
        raise ValueError("unknown placement profile")
    free = shutil.disk_usage(free_root).free if free_bytes is None else free_bytes
    if free < artifact_bytes + RESERVE_BYTES:
        raise ValueError("artifact plus 10 GiB reserve does not fit")
    return {"status": "placement_only_no_download", "profile": profile,
            "required_free_bytes": artifact_bytes + RESERVE_BYTES, "observed_free_bytes": free}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true", help="verify public small metadata without weights")
    parser.add_argument("--destination", type=Path, help="optional placement preflight; never downloads")
    parser.add_argument("--profile", choices=("vps_storage", "mac_local", "mac_external"))
    args = parser.parse_args()
    lock = validate_lock(json.loads(LOCK_PATH.read_text()))
    result = {"status": "local_lock_valid_no_weights_downloaded"}
    if args.live:
        result["live"] = verify_live(lock)
    if args.destination:
        if not args.profile:
            parser.error("--destination requires --profile")
        result["placement"] = placement_check(args.destination, profile=args.profile,
                                               artifact_bytes=lock["model"]["artifact"]["bytes"])
    elif args.profile:
        parser.error("--profile requires --destination")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
