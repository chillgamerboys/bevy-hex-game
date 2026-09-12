"""Capture admission accepts either complete authored package, never a partial roster."""
from copy import deepcopy
import unittest
from unittest.mock import patch

import arena


def state(expedition):
    members = [("Human", None, 1)]
    members += [("Goblin", "BabyGoblin", 15), ("Goblin", "Goblin", 92),
                ("Goblin", "Troll", 1), ("Shaman", "Shaman", 2),
                ("Dragon", "Dragon", 3), ("Shadow", "MountainShadow", 1)] if expedition else [
                    ("Goblin", None, 20), ("Shaman", None, 2), ("Dragon", None, 3)]
    return {
        "selection": {"map": "Forest Massif", "encounter": "Dragon"},
        "battle_setup": {"control": "Player", "rosters": [
            {"team": 1, "parties": [["Shadow"]]}, {"team": 2, "parties": [["Dragon"]]}],
            "seed": 1, "tick_limit": 14400, "player_recipe": None},
        "actors": [{"species": species, "expedition_role": role} for species, role, count in members for _ in range(count)],
        "progress": {"level": 1},
    }


class ExpeditionCapture(unittest.TestCase):
    def test_legacy_and_expedition_packages_require_their_complete_roster(self):
        for expedition in (False, True):
            valid = state(expedition)
            arena.validate_capture_setup(valid, "forest-massif", "dragon", {})
            partial = deepcopy(valid)
            partial["actors"].pop()
            with self.assertRaises(RuntimeError):
                arena.validate_capture_setup(partial, "forest-massif", "dragon", {})

    def test_same_species_counts_cannot_hide_wrong_authored_roles(self):
        changed = state(True)
        next(actor for actor in changed["actors"] if actor["expedition_role"] == "Troll")["expedition_role"] = "Goblin"
        with self.assertRaisesRegex(RuntimeError, "roles"):
            arena.validate_capture_setup(changed, "forest-massif", "dragon", {})

    def test_explicit_package_option_reaches_capture_without_launching(self):
        with patch.object(arena, "capture", return_value=0) as capture:
            self.assertEqual(arena.main(["capture", "--forest-review", "--forest-world", str(arena.ROOT),
                                        "--output", "/tmp/unused-expedition-capture"]), 0)
            self.assertEqual(capture.call_args.args[0].forest_world, arena.ROOT.resolve())
        with patch.object(arena, "capture") as capture:
            self.assertEqual(arena.main(["capture", "--forest-world", "relative", "--output", "/tmp/unused"]), 1)
            capture.assert_not_called()


if __name__ == "__main__":
    unittest.main()
