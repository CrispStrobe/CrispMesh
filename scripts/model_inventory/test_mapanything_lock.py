"""No network or weight download: synthetic lock and metadata contracts."""

import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts.model_inventory import mapanything_lock as inventory


LOCK = json.loads(inventory.LOCK_PATH.read_text())


class MapAnythingLockTests(unittest.TestCase):
    def test_exact_pins_and_license_lanes(self):
        self.assertEqual(inventory.validate_lock(copy.deepcopy(LOCK)), LOCK)
        mutations = [
            ("code", "license", "CC-BY-NC-4.0"),
            ("model", "repo", "facebook/map-anything"),
            ("model", "card_license", "cc-by-nc-4.0"),
            ("model", "revision", "0" * 40),
            ("model", "gated", True),
            ("model", "artifact", {**LOCK["model"]["artifact"], "sha256": "0" * 64}),
        ]
        for section, key, value in mutations:
            with self.subTest(section=section, key=key):
                changed = copy.deepcopy(LOCK)
                changed[section][key] = value
                with self.assertRaises(ValueError):
                    inventory.validate_lock(changed)

    def test_live_check_reads_small_metadata_only(self):
        code, model = LOCK["code"], LOCK["model"]
        code_url = f"https://raw.githubusercontent.com/{inventory.CODE_REPO}/{code['revision']}/LICENSE"
        base = f"https://huggingface.co/{inventory.MODEL_REPO}/raw/{model['revision']}"
        api = f"https://huggingface.co/api/models/{inventory.MODEL_REPO}/revision/{model['revision']}?blobs=true"
        # Synthetic content cannot match reviewed digests. Patch the digest function
        # narrowly to exercise URL selection and metadata rejection separately.
        data = {
            code_url: b"license", f"{base}/README.md": b"card",
            f"{base}/config.json": b'{"pretrained_checkpoint_path":null}',
            api: json.dumps({"sha": model["revision"], "gated": False,
                             "cardData": {"license": "apache-2.0"},
                             "siblings": [{"rfilename": "model.safetensors", "size": model["artifact"]["bytes"],
                                           "lfs": {"sha256": model["artifact"]["sha256"]}}]}).encode(),
        }
        calls = []

        def fetch(url):
            calls.append(url)
            self.assertNotIn("/resolve/", url)
            return data[url]

        from unittest.mock import patch
        digests = {hashlib.sha256(data[u]).hexdigest(): expected for u, expected in
                   ((code_url, code["license_sha256"]), (f"{base}/README.md", model["card_sha256"]),
                    (f"{base}/config.json", model["config_sha256"]))}
        real_sha256 = hashlib.sha256

        class Digest:
            def __init__(self, payload):
                self.payload = payload

            def hexdigest(self):
                return digests.get(real_sha256(self.payload).hexdigest(), real_sha256(self.payload).hexdigest())

        with patch.object(inventory.hashlib, "sha256", Digest):
            self.assertEqual(inventory.verify_live(LOCK, fetch=fetch)["status"],
                             "metadata_verified_no_weights_downloaded")
        self.assertEqual(set(calls), set(data))
        bad = copy.deepcopy(data)
        value = json.loads(bad[api])
        value["cardData"]["license"] = "cc-by-nc-4.0"
        bad[api] = json.dumps(value).encode()
        with patch.object(inventory.hashlib, "sha256", Digest):
            with self.assertRaisesRegex(ValueError, "license"):
                inventory.verify_live(LOCK, fetch=lambda url: bad[url])

    def test_placement_is_read_only_and_reserves_space(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder).resolve()
            target = root / "model.safetensors"
            enough = inventory.ARTIFACT_BYTES + inventory.RESERVE_BYTES
            report = inventory.placement_check(target, profile="vps_storage",
                                               artifact_bytes=inventory.ARTIFACT_BYTES,
                                               free_bytes=enough, storage_root=root)
            self.assertEqual(report["status"], "placement_only_no_download")
            self.assertFalse(target.exists())
            with self.assertRaisesRegex(ValueError, "reserve"):
                inventory.placement_check(target, profile="vps_storage",
                                          artifact_bytes=inventory.ARTIFACT_BYTES,
                                          free_bytes=enough - 1, storage_root=root)
            with self.assertRaisesRegex(ValueError, "exact reviewed artifact size"):
                inventory.placement_check(target, profile="vps_storage",
                                          artifact_bytes=-1, free_bytes=enough, storage_root=root)
            (root / "other").mkdir()
            with self.assertRaisesRegex(ValueError, "canonical storage"):
                inventory.placement_check(target, profile="vps_storage",
                                          artifact_bytes=inventory.ARTIFACT_BYTES,
                                          free_bytes=enough, storage_root=root / "other")

    def test_mac_external_requires_mount_device_exact_path_and_both_reserves(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder).resolve()
            workspace = root / "internal"
            workspace.mkdir()
            mount = root / "backups"
            expected = mount / "ai" / "crispmesh" / "map-anything-apache" / inventory.MODEL_REVISION
            expected.mkdir(parents=True)
            destination = expected / "model.safetensors"
            required = inventory.ARTIFACT_BYTES + inventory.RESERVE_BYTES
            original_stat = Path.stat

            class DeviceStat:
                def __init__(self, result, device):
                    self.result, self.st_dev = result, device

                def __getattr__(self, name):
                    return getattr(self.result, name)

            def separate_external_device(path, *args, **kwargs):
                result = original_stat(path, *args, **kwargs)
                if path == mount or path.is_relative_to(mount):
                    return DeviceStat(result, result.st_dev + 100)
                return result

            options = dict(profile="mac_external", artifact_bytes=inventory.ARTIFACT_BYTES,
                           mac_mount=mount, workspace=workspace, free_bytes=required,
                           internal_free_bytes=inventory.RESERVE_BYTES)
            with patch.object(Path, "is_mount", lambda path: path == mount):
                with self.assertRaisesRegex(ValueError, "shares the internal device"):
                    inventory.placement_check(destination, **options)
                with patch.object(Path, "stat", separate_external_device):
                    report = inventory.placement_check(destination, **options)
                    self.assertEqual(report["observed_free_bytes"], required)
                    self.assertFalse(destination.exists())
                    with self.assertRaisesRegex(ValueError, "external.*reserve|does not fit"):
                        inventory.placement_check(destination, **{**options, "free_bytes": required - 1})
                    with self.assertRaisesRegex(ValueError, "internal disk"):
                        inventory.placement_check(destination, **{**options, "internal_free_bytes": inventory.RESERVE_BYTES - 1})
                    wrong = mount / "ai" / "other" / "model.safetensors"
                    wrong.parent.mkdir()
                    with self.assertRaisesRegex(ValueError, "exact versioned path"):
                        inventory.placement_check(wrong, **options)
            with patch.object(Path, "is_mount", return_value=False):
                with self.assertRaisesRegex(ValueError, "not actually mounted"):
                    inventory.placement_check(destination, **options)


if __name__ == "__main__":
    unittest.main()
