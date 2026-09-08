"""Fail-closed receipt guards; these tests never invoke Cargo or a native window."""

import argparse
from copy import deepcopy
import unittest

from arena_battles import DEFAULT_MATCHUPS, PRESET_MEMBERS, validate_rounds


def request(matchups=DEFAULT_MATCHUPS):
    return argparse.Namespace(map="fort", matchups=matchups, first_seed=3, seeds=1, seconds=2)


def paired_rows(args):
    rows = []
    for seed in range(args.first_seed, args.first_seed + args.seeds):
        for first, second in (pair.split(":") for pair in args.matchups.split(",")):
            for swap in (False, True):
                left, right = (second, first) if swap else (first, second)
                actors, teams = [], []
                for team, preset in ((1, left), (2, right)):
                    members = PRESET_MEMBERS[preset]
                    for species in members:
                        actors.append({"id": len(actors), "team": team, "species": species,
                                       "hp": 50.0 if team == 1 else 0.0})
                    teams.append({"team": team, "initial": len(members),
                                  "living": len(members) if team == 1 else 0,
                                  "hp": len(members) * (50.0 if team == 1 else 0.0),
                                  "max_hp": len(members) * 50.0})
                rows.append({
                    "map": "Fort", "first": first, "second": second, "left": left, "right": right,
                    "side_and_initiative_swapped": swap,
                    "setup": {"control": "Spectator", "seed": seed, "tick_limit": args.seconds * 120,
                              "rosters": [{"team": team, "parties": [PRESET_MEMBERS[preset]]}
                                          for team, preset in ((1, left), (2, right))]},
                    "setup_source": "accepted_battle_setup", "actors": actors,
                    "stats": [{"id": a["id"], "species": a["species"]} for a in actors],
                    "summary": {"seed": seed, "ticks": 120, "seconds": 1.0,
                                "teams": teams, "result": {"TeamWinner": 1}},
                    "terminal_publication": {"cpu_ms": 0.1, "revision_before": 8, "revision_after": 9,
                                             "terrain_published": True, "tick_before": 120,
                                             "tick_after": 120, "battle_state_unchanged": True},
                })
    return rows


class BattleReceiptGuards(unittest.TestCase):
    def setUp(self):
        self.args = request()
        self.rows = paired_rows(self.args)

    def rejected(self, mutate):
        rows = deepcopy(self.rows)
        mutate(rows[0])
        with self.assertRaises(RuntimeError):
            validate_rounds(rows, self.args)

    def test_complete_paired_original_rosters_are_admitted(self):
        validate_rounds(self.rows, self.args)

    def test_requested_map_control_members_limit_and_seed_are_checked(self):
        mutations = [
            lambda r: r.update(map="Duel"),
            lambda r: r["setup"].update(control="Player"),
            lambda r: r["setup"].update(rosters=[]),
            lambda r: r["setup"].update(tick_limit=1),
            lambda r: r["setup"].update(seed=99),
            lambda r: r["summary"].update(seed=99),
        ]
        for index, mutate in enumerate(mutations):
            with self.subTest(mutation=index):
                self.rejected(mutate)

    def test_claiming_a_swap_without_swapping_actual_sides_or_rosters_is_rejected(self):
        for field in ("labels", "rosters"):
            rows = deepcopy(self.rows)
            if field == "labels":
                rows[1]["left"], rows[1]["right"] = rows[0]["left"], rows[0]["right"]
            else:
                rows[1]["setup"]["rosters"] = rows[0]["setup"]["rosters"]
            with self.subTest(field=field), self.assertRaises(RuntimeError):
                validate_rounds(rows, self.args)

    def test_actual_bodies_must_match_members_teams_and_unique_ids(self):
        for mutate in (
            lambda r: r["actors"].pop(),
            lambda r: r["actors"][0].update(species="Human"),
            lambda r: r["actors"][0].update(team=3),
            lambda r: r["actors"][1].update(id=r["actors"][0]["id"]),
            lambda r: r["actors"][0].update(hp=-1),
            lambda r: r["actors"][0].update(hp=float("nan")),
            lambda r: r["stats"].pop(),
        ):
            self.rejected(mutate)

    def test_summary_teams_counts_hp_and_time_must_match_actual_bodies(self):
        for mutate in (
            lambda r: r["summary"]["teams"].pop(),
            lambda r: r["summary"]["teams"][0].update(team=9),
            lambda r: r["summary"]["teams"][0].update(initial=9),
            lambda r: r["summary"]["teams"][0].update(living=9),
            lambda r: r["summary"]["teams"][0].update(hp=1),
            lambda r: r["summary"]["teams"][0].update(max_hp=1),
            lambda r: r["summary"].update(seconds=90),
            lambda r: r["summary"].update(ticks=999999),
        ):
            self.rejected(mutate)

    def test_result_requires_a_consistent_terminal_condition(self):
        for result in (None, {"InvalidSetup": "bad spawn"}, {"TeamWinner": 2},
                       {"TeamWinner": 99}, "Draw", "Timeout", "unknown"):
            with self.subTest(result=result):
                self.rejected(lambda r: r["summary"].update(result=result))

    def test_real_timeout_and_elimination_draw_conditions_are_admitted(self):
        for result in ("Timeout", "Draw"):
            rows = deepcopy(self.rows)
            for row in rows:
                for actor in row["actors"]:
                    actor["hp"] = 50.0 if result == "Timeout" else 0.0
                for team in row["summary"]["teams"]:
                    team["living"] = team["initial"] if result == "Timeout" else 0
                    team["hp"] = team["max_hp"] if result == "Timeout" else 0.0
                row["summary"].update(result=result, ticks=240, seconds=2.0)
                row["terminal_publication"].update(tick_before=240, tick_after=240)
            with self.subTest(result=result):
                validate_rounds(rows, self.args)

    def test_duplicates_missing_rows_and_malformed_shapes_fail(self):
        for rows in (self.rows[:-1], self.rows + self.rows[:1], [{}], [None]):
            with self.subTest(rows=len(rows)), self.assertRaises(RuntimeError):
                validate_rounds(rows, self.args)

    def test_boolean_identity_values_are_not_integer_receipts(self):
        self.rejected(lambda r: r.update(side_and_initiative_swapped=0))
        self.rejected(lambda r: r["setup"]["rosters"][0].update(team=True))
        self.rejected(lambda r: r["actors"][0].update(id=False))
        self.rejected(lambda r: r["stats"][0].update(id=False))
        self.rejected(lambda r: r["summary"]["teams"][0].update(living=True))

    def test_terminal_flush_must_be_separate_frozen_and_measured(self):
        for mutate in (
            lambda r: r.pop("terminal_publication"),
            lambda r: r.update(setup_source="requested_resource"),
            lambda r: r["terminal_publication"].update(cpu_ms=-1),
            lambda r: r["terminal_publication"].update(tick_after=121),
            lambda r: r["terminal_publication"].update(battle_state_unchanged=False),
            lambda r: r["terminal_publication"].update(terrain_published=False),
        ):
            self.rejected(mutate)

    def test_historical_audit_opt_out_only_relaxes_new_timing_metadata(self):
        for row in self.rows:
            row.pop("terminal_publication")
            row.pop("setup_source")
        with self.assertRaises(RuntimeError):
            validate_rounds(self.rows, self.args)
        validate_rounds(self.rows, self.args, require_terminal_publication=False)
        self.rows[0]["actors"] = []
        with self.assertRaises(RuntimeError):
            validate_rounds(self.rows, self.args, require_terminal_publication=False)


if __name__ == "__main__":
    unittest.main()
