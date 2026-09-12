"""First launch builds missing content once; complete packages never invoke Cargo."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import forest_package as package


class ExpeditionBootstrap(unittest.TestCase):
    generation = {"world_id": "forest-massif-expedition", "package_fingerprint": "000000000000002a"}

    def publish(self, output):
        output.mkdir(exist_ok=True)
        (output / "current.ron").write_text("runtime pointer")
        (output / "arena-sites.ron").write_text("bounded runtime companion")
        (output / "compile-receipt.json").write_text(json.dumps({"strict": True, "package_fingerprint": "000000000000002a"}))
        (output / "content-verification.json").write_text(json.dumps({"version": 1, "world_id": "forest-massif-expedition", "manifest_fingerprint": 42}))

    def test_complete_package_fast_path_never_checks_or_builds_compiler(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "generation.json").write_text(json.dumps(self.generation))
            output = root / "compiled"
            self.publish(output)
            with patch.object(package, "CONTENT", root), patch.object(package.world_tool, "checked_binary") as compiler, patch.object(package.subprocess, "run") as run:
                self.assertFalse(package.ensure_package(root / "compiler", output, root / "scratch"))
                compiler.assert_not_called()
                run.assert_not_called()

    def test_missing_package_builds_verified_compiler_then_reproduces_and_reuses_package(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "generation.json").write_text(json.dumps(self.generation))
            output, target = root / "compiled", root / "authoring"
            def complete(command, **kwargs):
                self.assertEqual(kwargs["env"]["CARGO_TARGET_DIR"], str(target.resolve()))
                if "compile" in command:
                    self.publish(output)
            with patch.object(package, "CONTENT", root), patch.object(package.world_tool, "checked_binary", side_effect=ValueError("missing")), patch.object(package.subprocess, "run", side_effect=complete) as run:
                self.assertTrue(package.ensure_package(target, output, root / "scratch"))
                self.assertEqual(run.call_count, 2)
                self.assertEqual(run.call_args_list[0].args[0][-1], "build")
                self.assertIn("compile", run.call_args_list[1].args[0])
                self.assertFalse(package.ensure_package(target, output, root / "scratch"))
                self.assertEqual(run.call_count, 2)

    def test_partial_stale_and_malformed_receipts_never_count_as_ready(self):
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder)
            self.publish(output)
            self.assertTrue(package.package_ready(output, self.generation))
            (output / "arena-sites.ron").write_text("")
            self.assertFalse(package.package_ready(output, self.generation))
            self.publish(output)
            stale = dict(self.generation, package_fingerprint="000000000000002b")
            self.assertFalse(package.package_ready(output, stale))
            (output / "compile-receipt.json").write_text("{")
            self.assertFalse(package.package_ready(output, self.generation))
            self.publish(output)
            (output / "content-verification.json").unlink()
            self.assertFalse(package.package_ready(output, self.generation))

    def test_successful_process_without_complete_publication_stops_launch(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "generation.json").write_text(json.dumps(self.generation))
            with patch.object(package, "CONTENT", root), patch.object(package.world_tool, "checked_binary"), patch.object(package.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "launch stopped"):
                    package.ensure_package(root / "compiler", root / "missing", root / "scratch")
                self.assertEqual(run.call_count, 1, "verified compiler needs no rebuild")


if __name__ == "__main__":
    unittest.main()
