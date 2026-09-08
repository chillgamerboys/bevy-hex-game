"""Worm typed capture admission and CLI fail-safes; no Cargo or visual claims."""
from contextlib import redirect_stderr
from copy import deepcopy
import io
import math
import unittest
from unittest.mock import patch

from arena import (GOLEM_VIEWS, WISP_VIEWS, WORM_VIEWS, WORM_OBSERVER_PRESETS,
                   main, validate_capture_setup, validate_worm_state)
from arena_battles import validate_rounds
from test_arena_battles import paired_rows, request


def specimen(count=4):
    parts = [{"offset": [0, .8 if index == 0 else 0, index*1.5], "height": .4} for index in range(count)]
    actor = {"id": 0, "species": "Worm", "hp": 140, "feet": [0, 2, 0],
             "body_hex_prisms": parts, "body_dimensions": [math.sqrt(3), 1.2, (count-1)*1.5+2],
             "body_center": [0, 2.6, (count-1)*.75], "body_rotation": [0, 0, 0, 1],
             "idle_mouth": [0, 3, 0], "worm": {"phase": "Exposed", "head_index": 0, "head_clearance": .4, "exposed": True},
             "attack": {"kind": "WormBoulder", "phase": "Windup", "progress": .4}}
    return {"actors": [actor], "worm_render_prisms": count, "worm_render_parts": [
        {"actor": 0, "index": index, "translation": [p["offset"][0], p["offset"][1]+.2, p["offset"][2]], "scale": [1, .4, 1]}
        for index, p in enumerate(parts)], "boulder_render_count": 1,
        "projectiles": [{"owner": 0, "appearance": "Boulder", "source_ability": "WormBoulder", "collision_radius": .22,
                         "age": .1, "position": [0, 3, -2], "previous_position": [0, 3, -1.8], "velocity": [0, 1, -18]}],
        "frame": 24, "phase_reached_frame": 20}


def conversion_specimen(reset=False):
    state = specimen()
    position = {"coord": {"q": 1, "r": 2}, "level": 5}
    state["worm_capture"] = {
        "accepted_outcomes": 1, "rejected_outcomes": 0, "dirt": 2, "current_generation": 4+int(reset),
        "current_revision": 12+int(reset), "restored_revision": 13 if reset else None,
        "reset_key": {"key": "R", "frame": 19} if reset else None,
        "conversion": {"actor": 0, "generation": 4, "sequence": 2, "frame": 15, "tick": 30, "revision": 12,
                       "changed": [{"position": position, "center": [1, 2, 3], "before": 3,
                                    "health_before": [2, 5], "health_after": [1, 2]}]},
        "current_cells": [{"position": position, "material": 3 if reset else 2, "published_health": None if reset else [1, 2]}]}
    state.update(started=not reset, paused=reset)
    return state


class WormCaptureGuards(unittest.TestCase):
    def test_matrix_has_full_fort_two_body_angles_and_preserves_originals(self):
        self.assertEqual(len(WORM_VIEWS), 13)
        self.assertEqual(len({row[0] for row in WORM_VIEWS}), 13)
        self.assertIn(("fort-worm-overview", "overview", "fort", "worm", None), WORM_VIEWS)
        self.assertEqual(WORM_OBSERVER_PRESETS, ("worm", "goblins"))
        self.assertEqual((len(GOLEM_VIEWS), len(WISP_VIEWS)), (12, 12))
        args = request("worm:shadow,worm:dragon")
        validate_rounds(paired_rows(args), args)

    def test_actor_zero_dynamic_four_and_six_prism_bodies_and_released_payload(self):
        for count in (4, 6):
            for phase in ("body", "body-rear", "windup", "boulder", "boulder-rear"):
                validate_worm_state(specimen(count), "encounter-worm-"+phase)
        state = specimen()
        state["actors"][0]["hp"] = 0
        validate_worm_state(state, "encounter-worm-boulder")
        with self.assertRaises(RuntimeError):
            validate_worm_state(state, "encounter-worm-windup")

    def test_dynamic_shape_render_or_payload_mismatch_fails_closed(self):
        mutations = (
            lambda s: s["actors"][0]["body_hex_prisms"].pop(),
            lambda s: s["actors"][0]["body_hex_prisms"][0].update(height=.8),
            lambda s: s["actors"][0].update(idle_mouth=[0, 2, 0]),
            lambda s: s["actors"][0].update(body_center=[0, 2, 0]),
            lambda s: s["actors"][0].update(body_rotation=[0, 1, 0, 0]),
            lambda s: s["actors"][0]["worm"].update(head_index=1),
            lambda s: s["actors"][0]["worm"].update(head_clearance=float("nan")),
            lambda s: s["worm_render_parts"].pop(),
            lambda s: s["worm_render_parts"][0].update(index=1),
            lambda s: s["worm_render_parts"][0].update(translation=[0, .2, 0]),
            lambda s: s["worm_render_parts"][0].update(scale=[1, 1, 1]),
            lambda s: s.update(boulder_render_count=0),
            lambda s: s["projectiles"][0].update(source_ability="Fireball"),
            lambda s: s["projectiles"][0].update(collision_radius=float("nan")),
            lambda s: s["projectiles"][0].update(owner=1),
            lambda s: s["projectiles"][0].update(position=[0, 3, 0]),
            lambda s: s.update(frame=23),
        )
        for index, mutate in enumerate(mutations):
            state = specimen(); mutate(state)
            with self.subTest(index=index), self.assertRaises(RuntimeError):
                validate_worm_state(state, "encounter-worm-boulder")

    def test_natural_physical_phase_and_visible_head_are_required(self):
        state = specimen()
        for phase in ("buried", "emerging"):
            with self.assertRaises(RuntimeError):
                validate_worm_state(state, "encounter-worm-"+phase)
        state["actors"][0]["worm"].update(phase="Travel", exposed=False, head_clearance=0)
        with self.assertRaises(RuntimeError):
            validate_worm_state(state, "encounter-worm-buried")
        state["actors"][0]["head_earth"] = {"position": {"coord": {"q": 0, "r": 0}, "level": 5}, "substance": 2}
        validate_worm_state(state, "encounter-worm-buried")
        state["actors"][0]["worm"].update(phase="Emerging", head_clearance=.2)
        validate_worm_state(state, "encounter-worm-emerging")
        state["battle_setup"] = {"player_recipe": "Worm"}
        state["visible_subjects"] = [0]
        with self.assertRaises(RuntimeError):
            validate_worm_state(state, "encounter-first")
        state["actors"][0]["worm"].update(exposed=True)
        validate_worm_state(state, "encounter-third")

    def test_conversion_and_key_reset_require_matching_published_material_health(self):
        for reset in (False, True):
            view = "encounter-worm-reset" if reset else "encounter-worm-converted-earth"
            validate_worm_state(conversion_specimen(reset), view)
            for mutate in (lambda e: e.update(current_generation=9),
                           lambda e: e["current_cells"][0].update(material=0),
                           lambda e: e["current_cells"][0].update(published_health=[2, 2]),
                           lambda e: e["conversion"].update(actor=1),
                           lambda e: e["conversion"].update(changed=[])):
                state = conversion_specimen(reset); mutate(state["worm_capture"])
                with self.assertRaises(RuntimeError):
                    validate_worm_state(state, view)
        state = conversion_specimen(True);state["worm_capture"]["reset_key"]["key"] = "Enter"
        with self.assertRaises(RuntimeError):
            validate_worm_state(state, "encounter-worm-reset")

    def test_fort_override_and_fixed_matrix_options_are_not_relabelled(self):
        state = {"selection": {"map": "Fort", "encounter": "Dragon"},
                 "battle_setup": {"control": "Player", "player_recipe": "Worm", "seed": 1, "tick_limit": 14400,
                                  "rosters": [{"team": 1, "parties": [["Shadow"]]}, {"team": 2, "parties": [["Dragon"]]}]},
                 "actors": [{"species": "Human"}, {"species": "Worm"}]}
        validate_capture_setup(state, "fort", "worm", {})
        state["battle_setup"]["player_recipe"] = None
        with self.assertRaises(RuntimeError):
            validate_capture_setup(state, "fort", "worm", {})
        base = ["capture", "--worm-review", "--output", "/nonexistent-worm-test"]
        for extra in (["--map", "fort"], ["--encounter", "worm"], ["--spectator"], ["--team-a", "worm"],
                      ["--seed", "1"], ["--tick-limit", "120"], ["--view", "wrong-phase"]):
            with self.subTest(extra=extra), patch("arena.environment") as env, patch("arena.run_cargo") as cargo, redirect_stderr(io.StringIO()):
                self.assertEqual(main(base+extra), 1)
                env.assert_not_called();cargo.assert_not_called()


if __name__ == "__main__":
    unittest.main()
