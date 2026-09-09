"""Native Golem receipt admission, without Cargo, windows, or visual claims."""
import argparse
from copy import deepcopy
import math
import unittest

from arena import GOLEM_OBSERVER_PRESETS, GOLEM_VIEWS, MENU_VIEWS, battle_environment, validate_capture_setup, validate_golem_state, validate_terminal_menu_state


def specimen():
    width = math.sqrt(3)
    return {"actors": [{"id": 0, "species": "Golem", "hp": 500,
                        "body_dimensions": [3 * width, 2, 5], "body_rotation": [0, 0, 0, 1],
                        "idle_mouth": [0, 1.4, 2.5],
                        "body_hex_prisms": [{"offset": list(offset), "height": 2} for offset in
                                            ((0, 0, 0), (width, 0, 0), (width/2, 0, 1.5),
                                             (-width/2, 0, 1.5), (-width, 0, 0),
                                             (-width/2, 0, -1.5), (width/2, 0, -1.5))],
                        "attack": {"kind": "GolemLaser", "phase": "Windup", "progress": .4},
                        "beam": {"origin": [0, 1.4, 2.5], "direction": [0, 0, 1],
                                 "end": [0, 1.4, 20], "radius": .12, "tracking": False}}],
            "golem_render_prisms": 7, "phase_reached_frame": 100, "frame": 104}


class GolemCaptureGuards(unittest.TestCase):
    def test_matrix_has_fourteen_unique_scoped_recipes(self):
        self.assertEqual(GOLEM_OBSERVER_PRESETS, ("golem", "dragon"))
        self.assertEqual(len(GOLEM_VIEWS), 14)
        self.assertEqual(len({row[0] for row in GOLEM_VIEWS}), 14)
        self.assertEqual(sum(row[2] == "fort" and row[3] == "golem" for row in GOLEM_VIEWS), 8)
        self.assertTrue(all(row[3] == "shadow" for row in GOLEM_VIEWS if row[2] == "duel"))

    def test_partial_charge_accepts_golem_actor_zero_and_independent_moving_mouth(self):
        state = specimen()
        validate_golem_state(state, "encounter-golem-charge")
        actor = state["actors"][0]
        actor["idle_mouth"] = [1, 1.4, 0]
        actor["beam"]["tracking"] = True
        actor["attack"]["progress"] = .9
        validate_golem_state(state, "encounter-golem-charge-late")
        actor["attack"]["phase"] = "Active"
        for view in ("encounter-golem-beam", "encounter-golem-beam-rear"):
            validate_golem_state(state, view)

    def test_wrong_geometry_meshes_or_phases_are_rejected(self):
        for change in (
            lambda s: s["actors"][0]["body_hex_prisms"].pop(),
            lambda s: s["actors"][0]["body_hex_prisms"][0].update(height=1),
            lambda s: s["actors"][0]["body_hex_prisms"][0].update(offset=[1, 0, 0]),
            lambda s: s["actors"][0].update(body_rotation=[0, 1, 0, 0]),
            lambda s: s.update(golem_render_prisms=6),
            lambda s: s["actors"][0]["beam"].update(direction=[0, 0, 2]),
            lambda s: s["actors"][0]["beam"].update(end=[1, 1.4, 20]),
            lambda s: s["actors"][0]["beam"].update(radius=float("nan")),
            lambda s: s["actors"][0]["beam"].update(tracking=None),
            lambda s: s["actors"][0]["attack"].update(kind="Swipe"),
            lambda s: s["actors"][0].update(hp=0),
            lambda s: s.update(frame=103),
        ):
            state = specimen()
            change(state)
            with self.assertRaises(RuntimeError):
                validate_golem_state(state, "encounter-golem-charge")

    def test_slam_requires_published_fresh_effect_with_actual_spherical_extent(self):
        state = specimen()
        state["actors"][0].update(beam=None, attack={"kind": "GolemSlam", "phase": "Active",
                                                  "origin": [0, 1, 0], "range": 6.9282})
        effect = {"spell": "AreaBlast", "center": [0, 1, 0], "radius": 6.9282, "age": 1/120}
        state["effects"] = [effect]
        validate_golem_state(state, "encounter-golem-slam")
        for change in ({"age": 0}, {"age": .2}, {"radius": 2}, {"center": [20, 1, 0]}):
            variant = deepcopy(state)
            variant["effects"][0].update(change)
            with self.assertRaises(RuntimeError):
                validate_golem_state(variant, "encounter-golem-slam")

    def test_stone_swipe_requires_its_own_phase_after_normal_pulse_publication(self):
        state = specimen()
        actor = state["actors"][0]
        actor.update(beam=None, attack={"kind": "GolemSwipe", "phase": "Windup", "progress": .5})
        validate_golem_state(state, "encounter-golem-swipe-windup")
        with self.assertRaises(RuntimeError):
            validate_golem_state(state, "encounter-golem-swipe")
        actor["attack"].update(phase="Active", progress=.2)
        validate_golem_state(state, "encounter-golem-swipe")
        validate_golem_state(state, "encounter-golem-swipe-rear")
        for change in ({"kind": "Swipe"}, {"kind": "GolemSlam"}, {"phase": "Recovery"}, {"progress": 0}):
            variant = deepcopy(state)
            variant["actors"][0]["attack"].update(change)
            with self.assertRaises(RuntimeError):
                validate_golem_state(variant, "encounter-golem-swipe")

    def test_terminal_menu_requires_explicit_fixture_outcome_and_paused_transition(self):
        for view, outcome in (("terminal-win", "win"), ("terminal-defeat", "defeat")):
            self.assertIn(view, MENU_VIEWS)
            state = {"started": True, "paused": True, "terminal_menu_outcome": outcome,
                     "terminal_menu_fixture": "synthetic-knockout-for-menu-presentation",
                     "phase_reached_frame": 20, "frame": 24}
            validate_terminal_menu_state(state, view)
            for change in ({"started": False}, {"paused": False}, {"terminal_menu_outcome": "draw"},
                           {"terminal_menu_fixture": None}, {"phase_reached_frame": None}, {"frame": 23}):
                with self.assertRaises(RuntimeError):
                    validate_terminal_menu_state({**state, **change}, view)

    def test_fort_player_override_and_spectator_rosters_are_distinct_accepted_setups(self):
        state = {"actors": [{"species": "Human"}, {"species": "Golem"}], "selection": {"map": "Fort", "encounter": "Dragon"}, "battle_setup": {
            "control": "Player", "player_recipe": "Golem", "seed": 1, "tick_limit": 14400,
            "rosters": [{"team": 1, "parties": [["Shadow"]]}, {"team": 2, "parties": [["Dragon"]]}]}}
        validate_capture_setup(state, "fort", "golem", {})
        state["battle_setup"]["player_recipe"] = None
        with self.assertRaises(RuntimeError):
            validate_capture_setup(state, "fort", "golem", {})
        args = argparse.Namespace(spectator=True, encounter=None, team_a="golem", team_b="dragon", seed=7, tick_limit=900)
        env = battle_environment(args, "duel")
        state = {"actors": [{"species": "Golem", "team": 1}, {"species": "Dragon", "team": 2}], "selection": {"map": "Duel", "encounter": "Shadow"}, "battle_setup": {
            "control": "Spectator", "player_recipe": None, "seed": 7, "tick_limit": 900,
            "rosters": [{"team": 1, "parties": [["Golem"]]}, {"team": 2, "parties": [["Dragon"]]}]}}
        validate_capture_setup(state, "duel", "shadow", env)
        missing = deepcopy(state)
        missing["actors"].pop(0)
        with self.assertRaises(RuntimeError):
            validate_capture_setup(missing, "duel", "shadow", env)
        state["battle_setup"]["player_recipe"] = "Golem"
        with self.assertRaises(RuntimeError):
            validate_capture_setup(state, "duel", "shadow", env)
        args.spectator, args.encounter = False, "golem"
        args.team_a = args.team_b = args.seed = args.tick_limit = None
        self.assertEqual(battle_environment(args, "duel"), {})
        with self.assertRaises(RuntimeError):
            battle_environment(args, "seven-regions")


if __name__ == "__main__":
    unittest.main()
