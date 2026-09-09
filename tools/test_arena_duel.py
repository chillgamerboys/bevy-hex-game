"""Player Duel captures must identify and publish the requested full party."""
from copy import deepcopy
import unittest

from arena import ENCOUNTER_LABELS, PLAYER_OVERRIDES, PRESET_MEMBERS, validate_capture_setup


class DuelPartyReceipts(unittest.TestCase):
    def test_all_selectable_parties_require_the_exact_accepted_recipe_and_bodies(self):
        for slug in ("shadow", "dragon", "goblins", "shaman-party", "golem", "wisps-4", "worm"):
            with self.subTest(party=slug):
                recipe = PLAYER_OVERRIDES.get(slug, {"dragon": "Dragon", "goblins": "Goblins", "shaman-party": "ShamanParty"}.get(slug))
                state = {"selection": {"map": "Duel", "encounter": ENCOUNTER_LABELS["dragon" if slug in PLAYER_OVERRIDES else slug]},
                         "battle_setup": {"control": "Player", "player_recipe": recipe, "seed": 1, "tick_limit": 14400,
                                          "rosters": [{"team": 1, "parties": [["Shadow"]]}, {"team": 2, "parties": [["Dragon"]]}]},
                         "actors": [{"species": species} for species in ["Human", *PRESET_MEMBERS[slug]]]}
                validate_capture_setup(state, "duel", slug, {})
                missing = deepcopy(state)
                missing["actors"].pop()
                with self.assertRaises(RuntimeError):
                    validate_capture_setup(missing, "duel", slug, {})
                wrong = deepcopy(state)
                wrong["battle_setup"]["player_recipe"] = "Dragon" if recipe is None else None
                with self.assertRaises(RuntimeError):
                    validate_capture_setup(wrong, "duel", slug, {})


if __name__ == "__main__":
    unittest.main()
