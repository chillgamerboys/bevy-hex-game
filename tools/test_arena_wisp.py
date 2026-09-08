"""Wisp capture and paired-roster admission without Cargo or visual claims."""
from copy import deepcopy
import math
import unittest

from arena import WISP_OBSERVER_RECIPES, WISP_VIEWS, validate_capture_setup, validate_wisp_state
from arena_battles import DEFAULT_MATCHUPS, validate_rounds
from test_arena_battles import paired_rows, request


def state():
    return {"actors": [{"id": 0, "species": "Wisp", "hp": 18, "flying": True, "grounded": False,
                        "feet": [0, 4, 0], "idle_mouth": [0, 4.2, 0],
                        "body_hex_prisms": [{"offset": [0, 0, 0], "height": .4}],
                        "body_dimensions": [math.sqrt(3), .4, 2], "body_rotation": [0, 0, 0, 1],
                        "attack": {"kind": "WispEmber", "phase": "Windup", "progress": .4}}],
            "wisp_render_prisms": 1, "frame": 20, "phase_reached_frame": 16,
            "lighting": {"fixture": "ordinary", "ambient_brightness": 420, "directional_illuminance": [18000]},
            "projectiles": [{"owner": 0, "appearance": "Ember", "source_ability": "WispEmber",
                             "collision_radius": .06, "age": .05, "position": [0, 4.2, 1.6],
                             "previous_position": [0, 4.2, 1.3], "velocity": [0, 0, 32]}]}


class WispCaptureGuards(unittest.TestCase):
    def test_matrix_is_bounded_and_preserves_explicit_swarm_and_single_goblin_recipes(self):
        self.assertEqual(len(WISP_VIEWS), 12)
        self.assertEqual(len({row[0] for row in WISP_VIEWS}), 12)
        self.assertEqual(sum(row[3] == "wisps-4" for row in WISP_VIEWS), 3)
        self.assertEqual(WISP_OBSERVER_RECIPES["duel-wisp-windup"], ("wisp", "goblin"))
        self.assertEqual(WISP_OBSERVER_RECIPES["duel-wisps-layers"], ("wisps-12", "wisps-12"))
        self.assertNotIn("wisp", DEFAULT_MATCHUPS)
        args = request("wisp:goblin,wisp:shadow,wisps-2:shadow,wisps-4:shadow,wisps-8:shadow,wisps-12:shadow")
        validate_rounds(paired_rows(args), args)

    def test_actor_zero_one_prism_windup_and_actual_ember_are_admitted(self):
        for view in ("encounter-wisp-body", "encounter-wisp-body-rear", "encounter-wisp-windup", "encounter-wisp-ember", "encounter-wisp-ember-rear"):
            with self.subTest(view=view):
                validate_wisp_state(state(), view)

    def test_body_shape_readiness_phase_and_frozen_payload_fail_closed(self):
        for mutate, view in (
            (lambda s: s["actors"][0]["body_hex_prisms"].append({"offset": [0, 0, 0], "height": .4}), "encounter-wisp-body"),
            (lambda s: s["actors"][0]["body_hex_prisms"][0].update(height=2), "encounter-wisp-body"),
            (lambda s: s["actors"][0]["body_hex_prisms"][0].update(height=float("nan")), "encounter-wisp-body"),
            (lambda s: s["actors"][0].update(body_dimensions=[.5, .8, .5]), "encounter-wisp-body"),
            (lambda s: s["actors"][0].update(body_rotation=[0, 1, 0, 0]), "encounter-wisp-body"),
            (lambda s: s["actors"][0].update(idle_mouth=[0, 6, 0]), "encounter-wisp-body"),
            (lambda s: s["actors"][0].update(flying=False), "encounter-wisp-body"),
            (lambda s: s.update(wisp_render_prisms=0), "encounter-wisp-body"),
            (lambda s: s.update(frame=19), "encounter-wisp-body"),
            (lambda s: s["actors"][0]["attack"].update(kind="Fireball"), "encounter-wisp-windup"),
            (lambda s: s["projectiles"][0].update(source_ability="Fireball"), "encounter-wisp-ember"),
            (lambda s: s["projectiles"][0].update(collision_radius=float("nan")), "encounter-wisp-ember"),
            (lambda s: s["projectiles"][0].update(owner=1), "encounter-wisp-ember"),
            (lambda s: s["projectiles"][0].update(position=[0, 4.2, .5]), "encounter-wisp-ember"),
        ):
            specimen = state()
            mutate(specimen)
            with self.assertRaises(RuntimeError):
                validate_wisp_state(specimen, view)

    def test_dark_views_require_actual_declared_light_settings(self):
        specimen = state()
        for view in ("encounter-wisp-dark", "encounter-wisp-dark-rear"):
            with self.assertRaises(RuntimeError):
                validate_wisp_state(specimen, view)
        specimen["lighting"] = {"fixture": "dim-comparison", "ambient_brightness": 6, "directional_illuminance": [0]}
        for view in ("encounter-wisp-dark", "encounter-wisp-dark-rear"):
            validate_wisp_state(specimen, view)
        specimen["lighting"]["directional_illuminance"] = [18000]
        with self.assertRaises(RuntimeError):
            validate_wisp_state(specimen, "encounter-wisp-dark")

    def test_layered_capture_requires_every_actual_living_flying_body(self):
        specimen = state()
        specimen["actors"] = [dict(deepcopy(specimen["actors"][0]), id=index, team=1+index//12, flight_layer=int(index%12>=7)) for index in range(24)]
        specimen["wisp_render_prisms"] = 24
        validate_wisp_state(specimen, "encounter-wisp-layers")
        one_layer = deepcopy(specimen)
        for actor in one_layer["actors"]:
            actor["flight_layer"] = 0
        with self.assertRaises(RuntimeError):
            validate_wisp_state(one_layer, "encounter-wisp-layers")
        specimen["actors"][0]["hp"] = 0
        with self.assertRaises(RuntimeError):
            validate_wisp_state(specimen, "encounter-wisp-layers")

    def test_fort_provisional_swarm_is_an_exact_accepted_override(self):
        specimen = {"selection": {"map": "Fort", "encounter": "Dragon"},
                    "battle_setup": {"control": "Player", "player_recipe": "Wisps4", "seed": 1, "tick_limit": 14400,
                                     "rosters": [{"team": 1, "parties": [["Shadow"]]}, {"team": 2, "parties": [["Dragon"]]}]},
                    "actors": [{"species": "Human"}] + [{"species": "Wisp"} for _ in range(4)]}
        validate_capture_setup(specimen, "fort", "wisps-4", {})
        specimen["actors"].pop()
        with self.assertRaises(RuntimeError):
            validate_capture_setup(specimen, "fort", "wisps-4", {})


if __name__ == "__main__":
    unittest.main()
