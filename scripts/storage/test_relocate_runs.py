import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts.storage.relocate_runs import (ALLOWLIST, inventory, link_points_to_destination,
                                           relocate, sync_files, sync_open_mode)


class RelocationTests(unittest.TestCase):
    def test_allowlist_extension_is_only_completed_bunny_masked_run(self):
        self.assertEqual(ALLOWLIST, frozenset({
            "classical-ycb-native-masked-009",
            "classical-ycb-native-masked-011-continuation",
            "classical-ycb-calibrated-012",
            "classical-bunny-retry-dense-002",
            "classical-bunny-native-masked-005",
        }))

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.workspace = self.root / "workspace"
        self.destination = self.root / "external" / "code" / "crispmesh-data" / "build-opencv"
        self.mount = self.root / "external"
        self.mount.mkdir()
        self.name = sorted(ALLOWLIST)[0]
        self.source = self.workspace / "build-opencv" / self.name
        (self.source / "nested").mkdir(parents=True)
        (self.source / "nested" / "result.json").write_text('{"status":"complete"}')
        self.options = dict(workspace=self.workspace, destination_root=self.destination,
                            mount=self.mount, min_free=0, require_mount=False)

    def test_verified_copy_then_symlink_with_external_audit(self):
        before = inventory(self.source)
        result = relocate(self.name, **self.options)
        self.assertTrue(result["link_verified"])
        self.assertTrue(result["backup_removed"])
        self.assertTrue(self.source.is_symlink())
        self.assertEqual(inventory(self.destination / self.name), before)
        audit = self.destination / ".relocation-audits" / (self.name + ".json")
        self.assertEqual(json.loads(audit.read_text())["inventory"], before)
        self.assertFalse(self.source.with_name(self.name + ".relocation-backup").exists())

    def test_fresh_destination_and_allowlist(self):
        with self.assertRaisesRegex(ValueError, "allowlist"):
            relocate("not-allowed", **self.options)
        (self.destination / self.name).mkdir(parents=True)
        with self.assertRaisesRegex(ValueError, "no overwrite"):
            relocate(self.name, **self.options)
        self.assertFalse(self.source.is_symlink())

    def test_rejects_symlink_without_copy(self):
        (self.source / "nested" / "bad").symlink_to(self.root)
        with self.assertRaisesRegex(ValueError, "symlink"):
            relocate(self.name, **self.options)
        self.assertFalse((self.destination / self.name).exists())

    def test_copy_mutation_preserves_original_and_copy(self):
        import scripts.storage.relocate_runs as mod
        original = mod.shutil.copytree

        def altered(*args, **kwargs):
            result = original(*args, **kwargs)
            (self.source / "nested" / "result.json").write_text("changed")
            return result

        with patch.object(mod.shutil, "copytree", side_effect=altered):
            with self.assertRaisesRegex(ValueError, "inventory changed"):
                relocate(self.name, **self.options)
        self.assertTrue(self.source.is_dir())
        self.assertFalse(self.source.is_symlink())
        self.assertTrue((self.destination / self.name).is_dir())

    def test_final_mount_failure_retains_backup_and_verified_copy(self):
        import scripts.storage.relocate_runs as mod
        device = self.mount.stat().st_dev
        options = dict(self.options, require_mount=True)
        with patch.object(mod, "verify_external_mount", side_effect=[device, ValueError("unmounted")]):
            with self.assertRaisesRegex(ValueError, "unmounted"):
                relocate(self.name, **options)
        backup = self.source.with_name(self.name + ".relocation-backup")
        self.assertTrue(self.source.is_dir())
        self.assertFalse(self.source.is_symlink())
        self.assertFalse(backup.exists())
        self.assertEqual(inventory(self.source), inventory(self.destination / self.name))
        audit = self.destination / ".relocation-audits" / (self.name + ".json")
        self.assertFalse(json.loads(audit.read_text())["backup_removed"])

    def test_flush_uses_writable_handle_without_changing_copied_bytes(self):
        self.assertEqual(sync_open_mode("nt"), "r+b")
        self.assertEqual(sync_open_mode("posix"), "rb")
        before = inventory(self.source)
        sync_files(self.source, ["nested/result.json"])
        self.assertEqual(inventory(self.source), before)

    def test_link_target_comparison_accepts_equivalent_relative_spelling(self):
        link = self.workspace / "relative-link"
        link.symlink_to(self.source.relative_to(self.workspace), target_is_directory=True)
        self.assertNotEqual(link.readlink(), self.source)
        self.assertTrue(link_points_to_destination(link, self.source))
        self.assertFalse(link_points_to_destination(link, self.destination))

    def test_copy_mutation_during_flush_rolls_back_before_deletion(self):
        import scripts.storage.relocate_runs as mod

        def corrupt_copied_file(root, relative_names):
            (root / relative_names[0]).write_bytes(b"changed after first hash")

        with patch.object(mod, "sync_files", side_effect=corrupt_copied_file):
            with self.assertRaisesRegex(ValueError, "post-flush.*differs"):
                relocate(self.name, **self.options)
        self.assertTrue(self.source.is_dir())
        self.assertFalse(self.source.is_symlink())
        self.assertFalse(self.source.with_name(self.name + ".relocation-backup").exists())
        self.assertEqual((self.source / "nested/result.json").read_text(), '{"status":"complete"}')


if __name__ == "__main__":
    unittest.main()
