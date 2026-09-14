"""First launch builds missing content once; complete packages never invoke Cargo."""
import json
import re
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import forest_package as package


class ExpeditionBootstrap(unittest.TestCase):
    generation = {"world_id": "forest-massif-expedition", "package_fingerprint": "000000000000002a"}

    def test_reviewed_art_verification_is_available_from_candidate_history(self):
        generation = json.loads((package.CONTENT / "generation.json").read_text())
        revision = generation.get("art_verification_revision", generation["source_revision"])
        subprocess.run(["git", "merge-base", "--is-ancestor", revision, "HEAD"],
                       cwd=package.ROOT, check=True)
        source = (package.CONTENT / "world.ron").read_text()
        paths = set(re.findall(r'source_path:"([^"]+)"', source))
        self.assertTrue(paths, "the published package must identify its exact artwork")
        for path in paths:
            committed = subprocess.check_output(["git", "show", f"{revision}:{path}"], cwd=package.ROOT)
            self.assertEqual(committed, (package.ROOT / path).read_bytes(), path)

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
                    working = Path(command[command.index("--scratch") + 1])
                    self.assertEqual(working.parent, (root / "scratch").resolve())
                    self.assertNotEqual(working, root / "scratch")
                    self.assertTrue(working.is_dir())
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

    def test_simultaneous_first_launches_compile_once_then_recheck_readiness(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "generation.json").write_text(json.dumps(self.generation))
            output = root / "compiled"
            entered, release, second_started = threading.Event(), threading.Event(), threading.Event()
            def complete(command, **kwargs):
                entered.set()
                self.assertTrue(release.wait(3), "release the bounded synthetic compiler")
                self.publish(output)
            def second():
                second_started.set()
                return package.ensure_package(root / "compiler", output, root / "scratch")
            with patch.object(package, "CONTENT", root), patch.object(package.world_tool, "checked_binary"), patch.object(package.subprocess, "run", side_effect=complete) as run:
                with ThreadPoolExecutor(max_workers=2) as workers:
                    first = workers.submit(package.ensure_package, root / "compiler", output, root / "scratch")
                    try:
                        self.assertTrue(entered.wait(3))
                        other = workers.submit(second)
                        self.assertTrue(second_started.wait(3))
                        time.sleep(.05)
                        self.assertFalse(other.done(), "another launcher waits for publication")
                    finally:
                        release.set()
                    self.assertTrue(first.result(timeout=3))
                    self.assertFalse(other.result(timeout=3), "waiting launcher reuses the completed package")
                self.assertEqual(run.call_count, 1)

    def test_failed_preparation_releases_lock_and_next_attempt_gets_new_scratch(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "generation.json").write_text(json.dumps(self.generation))
            output = root / "compiled"
            workspaces = []
            def complete(command, **kwargs):
                workspaces.append(command[command.index("--scratch") + 1])
                if len(workspaces) == 1:
                    raise OSError("compiler interrupted")
                self.publish(output)
            with patch.object(package, "CONTENT", root), patch.object(package.world_tool, "checked_binary"), patch.object(package.subprocess, "run", side_effect=complete):
                with self.assertRaisesRegex(OSError, "interrupted"):
                    package.ensure_package(root / "compiler", output, root / "scratch")
                self.assertTrue(package.ensure_package(root / "compiler", output, root / "scratch"))
            self.assertEqual(len(set(workspaces)), 2)

    def test_companion_replacement_never_exposes_partial_text_and_failed_write_retains_old(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            destination = root / "arena-sites.ron"
            destination.write_text("old complete companion")
            replace = package.os.replace
            def inspect_then_replace(source, target):
                self.assertEqual(destination.read_text(), "old complete companion")
                self.assertEqual(Path(source).read_text(), "new complete companion")
                replace(source, target)
            with patch.object(package.os, "replace", side_effect=inspect_then_replace):
                package.atomic_text(destination, "new complete companion")
            self.assertEqual(destination.read_text(), "new complete companion")
            with patch.object(package.os, "replace", side_effect=OSError("publication interrupted")):
                with self.assertRaisesRegex(OSError, "interrupted"):
                    package.atomic_text(destination, "incomplete candidate")
            self.assertEqual(destination.read_text(), "new complete companion")
            self.assertEqual(list(root.iterdir()), [destination], "temporary writes are cleaned up")


if __name__ == "__main__":
    unittest.main()
