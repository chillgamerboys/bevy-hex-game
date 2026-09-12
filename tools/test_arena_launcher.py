"""Launch cache selection works on other checkouts as well as this workstation."""
import argparse
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import arena


class LaunchTarget(unittest.TestCase):
    def test_failed_default_preparation_never_opens_the_native_app(self):
        with patch.object(arena, "prepare_forest_package", side_effect=RuntimeError("compile failed")), patch.object(arena, "run_cargo") as run:
            self.assertEqual(arena.main(["launch"]), 1)
            run.assert_not_called()

    def test_duel_launch_does_not_build_forest_content(self):
        with patch.object(arena, "prepare_forest_package") as prepare, patch.object(arena, "run_cargo", return_value=0) as run:
            self.assertEqual(arena.main(["launch", "--map", "duel"]), 0)
            prepare.assert_not_called()
            self.assertEqual(run.call_args.args[0]["HEX_ARENA_MAP"], "duel")

    def test_default_package_preparation_is_shared_by_launch_and_capture(self):
        env = {"CARGO_TARGET_DIR": "/separate/app-cache"}
        with patch.object(arena.subprocess, "run") as run:
            path = arena.prepare_forest_package(env)
            self.assertEqual(path, arena.ROOT / "assets/config/v4/forest-massif/expedition/compiled")
            self.assertEqual(env["HEX_FOREST_WORLD"], str(path))
            self.assertIn("ensure", run.call_args.args[0])
            self.assertEqual(run.call_args.kwargs["env"]["CARGO_TARGET_DIR"], str(arena.ROOT / "target/v4-authoring"))
            self.assertEqual(env["CARGO_TARGET_DIR"], "/separate/app-cache")
            arena.prepare_forest_package(env)
            self.assertEqual(run.call_count, 1, "the native child receives a prepared explicit package")

    def test_explicit_and_inherited_legacy_packages_bypass_preparation(self):
        env = {"HEX_FOREST_WORLD": "/legacy/compiled"}
        with patch.object(arena.subprocess, "run") as run:
            self.assertEqual(arena.prepare_forest_package(env), Path("/legacy/compiled"))
            run.assert_not_called()
        with patch.dict(os.environ, {"HEX_FOREST_WORLD": "/legacy/compiled", "HEX_ARENA_CAPTURE": "stale-capture"}), patch.object(arena.shutil, "which", return_value="/bin/cargo"):
            clean, removed = arena.environment(arena.ROOT / "target")
            self.assertEqual(clean["HEX_FOREST_WORLD"], "/legacy/compiled")
            self.assertNotIn("HEX_ARENA_CAPTURE", clean)
            self.assertIn("HEX_ARENA_CAPTURE", removed)
            self.assertNotIn("HEX_FOREST_WORLD", removed)

    def test_bootstrap_never_reuses_the_app_cargo_lock(self):
        preferred = arena.ROOT / "target/v4-authoring"
        env = {"CARGO_TARGET_DIR": str(preferred)}
        with patch.object(arena.subprocess, "run") as run:
            arena.prepare_forest_package(env)
            self.assertEqual(run.call_args.kwargs["env"]["CARGO_TARGET_DIR"], str(preferred / "forest-bootstrap"))

    def test_generic_forest_capture_preserves_the_fixed_roster(self):
        args = argparse.Namespace(encounter=None, spectator=False, team_a=None,
                                  team_b=None, seed=None, tick_limit=None)
        entries = arena.player_capture_entries(("overview",), "forest-massif", None)
        self.assertEqual(entries, [("overview", "overview", "forest-massif", "dragon", None)])
        self.assertEqual(arena.battle_environment(args, entries[0][2]), {})
        self.assertEqual(arena.player_capture_entries(("overview",), None, None)[0][3], "shadow")

    def test_configured_cache_takes_precedence_and_relative_paths_use_the_repo(self):
        with tempfile.TemporaryDirectory() as directory:
            absolute = Path(directory).resolve()
            with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(absolute)}):
                self.assertEqual(arena.default_target(), absolute)
        with patch.dict(os.environ, {"CARGO_TARGET_DIR": "../shared-build"}):
            self.assertEqual(arena.default_target(), (arena.ROOT / "../shared-build").resolve())

    def test_retained_cache_is_reused_only_when_it_exists(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory).resolve() / "retained"
            with patch.dict(os.environ, {"CARGO_TARGET_DIR": ""}), patch.object(arena, "LOCAL_TARGET", cache):
                self.assertEqual(arena.default_target(), arena.ROOT / "target")
                cache.mkdir()
                self.assertEqual(arena.default_target(), cache)


if __name__ == "__main__":
    unittest.main()
