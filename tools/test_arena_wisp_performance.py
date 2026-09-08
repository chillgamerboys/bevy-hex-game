"""Synthetic native workload admission only; no Cargo, gameplay or timing evidence."""
from contextlib import redirect_stderr
import io
import unittest
from unittest.mock import patch

from arena import WISP_PERFORMANCE_VIEWS, WISP_VIEWS, main, validate_wisp_performance_state

VIEW = "observer-wisp-stress"


def specimen():
    actors = [{"id": index, "team": 1 + index // 12, "species": "Wisp", "hp": 900,
               "max_hp": 1000, "flying": True, "flight_layer": int(index % 12 >= 7)}
              for index in range(24)]
    rows = [{"tick": tick, "frame": (tick + 1) // 2, "cpu_ms": 200 if tick <= 120 else .8,
             "terrain_publication": False, "damage_outcome": False, "destroyed_voxels": 0,
             "load": {"living_wisps": 24, "flying_wisps": 24, "active_parties": 2,
                      "team_layers": [[7, 5], [7, 5]], "unassigned_layers": 0,
                      "projectiles": 12, "windups": 12}}
            for tick in range(1, 1441)]
    return {"selection": {"map": "Duel"}, "tick": 1440, "human_actor_id": None, "actors": actors,
            "battle_setup": {"control": "Spectator", "player_recipe": None, "seed": 1, "tick_limit": 1440,
                             "rosters": [{"team": team, "parties": [["Wisp"] * 12]} for team in (1, 2)]},
            "battle_summary": {"seed": 1, "ticks": 1440, "result": "Timeout", "teams": [
                {"team": team, "initial": 12, "living": 12, "hp": 10800, "max_hp": 12000} for team in (1, 2)]},
            "wisp_stress": {"fixture": "synthetic-wisp-hp-1000", "loaded_from_config": True, "nominal_hp": 30, "applied_hp": 1000,
                            "warmup_ticks": 120, "requested_ticks": 1440, "actor_hp_mutation": False,
                            "injected_terrain_impacts": False, "rows": rows,
                            "terminal_publication": {"tick": 1440, "terrain_publication": False, "cpu_ms": .2}},
            "app_frame_wall_intervals_ms": [400] * 60 + [16] * 664}


class WispPerformanceGuards(unittest.TestCase):
    def test_bounded_matrix_keeps_static_views_and_excludes_warmup_from_both_clocks(self):
        self.assertEqual(len(WISP_VIEWS), 12)
        self.assertEqual(WISP_PERFORMANCE_VIEWS, (
            ("duel-wisps-stress", VIEW, "duel", "shadow", None),
            ("fort-wisps-stress", VIEW, "fort", "shadow", None)))
        for nominal in (18, 30):
            state = specimen()
            state["wisp_stress"]["nominal_hp"] = nominal
            summary = validate_wisp_performance_state(state, VIEW)
            self.assertEqual(summary["nominal_hp"], nominal)
            self.assertEqual(summary["tick_cpu"]["samples"], 1320)
            self.assertEqual(summary["tick_cpu"]["max_ms"], .8)
            self.assertEqual(summary["app_frame_wall_interval"]["samples"], 660)
            self.assertEqual(summary["app_frame_wall_interval"]["max_ms"], 16)
            self.assertEqual(summary["publication_ticks"], 0)
            self.assertEqual(summary["publication_tick_cpu"], {"samples": 0})
            self.assertEqual(summary["destroyed_voxels"], 0)
            self.assertEqual(summary["terminal_publication"]["cpu_ms"], .2)
        self.assertIsNone(validate_wisp_performance_state({}, "observer-performance"))

    def test_partial_warmup_frame_is_excluded_and_positive_publications_are_measured(self):
        state = specimen()
        state["wisp_stress"]["rows"][120]["frame"] = 60
        state["wisp_stress"]["rows"][121].update(terrain_publication=True, damage_outcome=True, destroyed_voxels=2, cpu_ms=12)
        summary = validate_wisp_performance_state(state, VIEW)
        self.assertEqual(summary["mixed_warmup_frames_excluded"], 1)
        self.assertEqual(summary["app_frame_wall_interval"]["max_ms"], 16)
        self.assertEqual(summary["publication_ticks"], 1)
        self.assertEqual(summary["publication_tick_cpu"]["max_ms"], 12)
        self.assertEqual(summary["damage_outcome_ticks"], 1)
        self.assertEqual(summary["destroyed_voxels"], 2)
        self.assertEqual(summary["tick_over_8_333_ms"], 1)

    def test_partial_or_mislabeled_synthetic_workloads_fail_closed(self):
        changes = (
            lambda s: s["wisp_stress"].update(loaded_from_config=False),
            lambda s: s["wisp_stress"].update(nominal_hp=None),
            lambda s: s["wisp_stress"].update(applied_hp=100000),
            lambda s: s["wisp_stress"].update(actor_hp_mutation=True),
            lambda s: s["wisp_stress"].update(injected_terrain_impacts=True),
            lambda s: s["battle_setup"].update(seed=2),
            lambda s: s["battle_setup"]["rosters"][0]["parties"][0].pop(),
            lambda s: s["battle_setup"].update(control="Player"),
            lambda s: s["battle_setup"].update(tick_limit=3600),
            lambda s: s["selection"].update(map="Seven Regions"),
            lambda s: s["battle_summary"].update(result={"TeamWinner": 1}),
            lambda s: s["battle_summary"]["teams"][0].update(max_hp=360),
            lambda s: s["battle_summary"]["teams"][0].update(hp=100),
            lambda s: s["actors"].pop(0),
            lambda s: s["actors"][0].update(flying=False),
            lambda s: s["actors"][0].update(hp=0),
            lambda s: s["actors"][0].update(max_hp=30),
            lambda s: s["wisp_stress"]["rows"][120]["load"].update(living_wisps=23),
            lambda s: s["wisp_stress"]["rows"][120]["load"].update(active_parties=1),
            lambda s: s["wisp_stress"]["rows"][120]["load"].update(team_layers=[[12, 0], [7, 5]]),
            lambda s: s["wisp_stress"]["rows"].pop(),
            lambda s: s["wisp_stress"]["rows"][120].update(tick=120),
            lambda s: s["wisp_stress"]["rows"][120].update(cpu_ms=float("nan")),
            lambda s: s["wisp_stress"]["rows"][120].update(terrain_publication=0),
            lambda s: s["wisp_stress"]["rows"][120].update(destroyed_voxels=-1),
            lambda s: s.update(app_frame_wall_intervals_ms=[1] * 719),
            lambda s: s["app_frame_wall_intervals_ms"].__setitem__(60, float("inf")),
            lambda s: s["wisp_stress"].update(terminal_publication=None),
            lambda s: s["wisp_stress"]["terminal_publication"].update(tick=1441),
        )
        for index, change in enumerate(changes):
            state = specimen()
            change(state)
            with self.subTest(change=index), self.assertRaises(RuntimeError):
                validate_wisp_performance_state(state, VIEW)
        for key in ("projectiles", "windups"):
            state = specimen()
            for row in state["wisp_stress"]["rows"]:
                row["load"][key] = 0
            with self.subTest(missing=key), self.assertRaises(RuntimeError):
                validate_wisp_performance_state(state, VIEW)

    def test_conflicting_fixed_matrix_options_fail_before_any_cargo_or_filesystem_mutation(self):
        base = ["capture", "--wisp-performance", "--output", "/nonexistent-wisp-performance-test"]
        for extra in (["--map", "fort"], ["--encounter", "wisp"], ["--spectator"],
                      ["--team-a", "wisps-12"], ["--team-b", "wisps-12"],
                      ["--seed", "1"], ["--tick-limit", "1440"], ["--view", "observer-performance"]):
            with self.subTest(extra=extra), patch("arena.environment") as env, patch("arena.run_cargo") as cargo, redirect_stderr(io.StringIO()):
                self.assertEqual(main(base + extra), 1)
                env.assert_not_called()
                cargo.assert_not_called()


if __name__ == "__main__":
    unittest.main()
