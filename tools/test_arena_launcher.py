"""Launch cache selection works on other checkouts as well as this workstation."""
import argparse
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import arena


class LaunchTarget(unittest.TestCase):
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
