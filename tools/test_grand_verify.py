"""Typed walking-receipt regressions; no Cargo, package, or game window."""
from __future__ import annotations

import copy
import math
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import grand_verify


def valid_receipt() -> dict:
    # Deliberately synthetic physics facts: verification checks the derivation,
    # while the Rust producer reads the actual controller-owned constants.
    limits = dict(voxel_height=0.35, automatic_step_height=0.4, gravity=18.0,
                  ground_snap_distance=0.001, collision_skin=0.0001, tick_seconds=1 / 120)
    limits["maximum_unsupported_descent"] = (min(limits["voxel_height"], limits["automatic_step_height"])
                                             + limits["ground_snap_distance"] + limits["collision_skin"])
    limits["maximum_airborne_ticks"] = math.ceil(
        math.sqrt(2 * limits["maximum_unsupported_descent"] / limits["gravity"])
        / limits["tick_seconds"]) + 1
    names = ["forest_north", "forest_south", "forest_east", "forest_west",
             "river_bank_escape", "river_bank_along", "valley_crossing",
             "grand-west-foothill-crossing", "grand-west-foothill-uphill",
             "grand-lake-foothill-uphill", "crystal_ascent"]
    grounding = dict(contract="one-voxel-grounded-walk-v1", limits=limits,
                     observed_simulation_ticks=120, total_airborne_ticks=20,
                     maximum_airborne_ticks=21, maximum_unsupported_descent=0.35,
                     completed_airborne_episodes=1, airborne_at_end=False, failure=None)
    return dict(kind="grand-ordinary-walking-r04", status="PASS", selected_route=None,
                expected_routes=len(names), route_names=names,
                routes=[dict(name=name, category="authored_connection" if name == "crystal_ascent" else "cross_country",
                             status="PASS", completed_segments=1, required_segments=1,
                             simulation_ticks=120, grounding=copy.deepcopy(grounding)) for name in names])


class GrandWalkingReceiptTests(unittest.TestCase):
    def test_current_bounded_step_receipt_is_accepted(self):
        grand_verify.validate_walking_receipt(valid_receipt())

    def test_old_or_missing_grounding_receipt_is_rejected(self):
        old = valid_receipt()
        old["kind"] = "grand-ordinary-walking-r02"
        missing = valid_receipt()
        missing["routes"][0].pop("grounding")
        for receipt in (old, missing):
            with self.subTest(kind=receipt["kind"]), self.assertRaises(RuntimeError):
                grand_verify.validate_walking_receipt(receipt)

    def test_drop_airtime_missing_ticks_and_unsettled_endpoint_cannot_pass(self):
        corruptions = ({"maximum_unsupported_descent": 1.4}, {"maximum_airborne_ticks": 120},
                       {"observed_simulation_ticks": 119}, {"airborne_at_end": True},
                       {"failure": "excessive fall"}, {"maximum_unsupported_descent": float("nan")})
        for change in corruptions:
            with self.subTest(change=change), self.assertRaises(RuntimeError):
                receipt = valid_receipt()
                receipt["routes"][0]["grounding"].update(change)
                grand_verify.validate_walking_receipt(receipt)

    def test_allowance_cannot_be_loosened_without_matching_physical_derivation(self):
        for change in ({"maximum_unsupported_descent": 2.0}, {"maximum_airborne_ticks": 999}):
            with self.subTest(change=change), self.assertRaises(RuntimeError):
                receipt = valid_receipt()
                receipt["routes"][0]["grounding"]["limits"].update(change)
                grand_verify.validate_walking_receipt(receipt)


def valid_crossing() -> dict:
    first = copy.deepcopy(valid_receipt()["routes"][0]["grounding"])
    first["observed_simulation_ticks"] = 101
    last = copy.deepcopy(first)
    last["observed_simulation_ticks"] = 40
    transition = dict(terrain_ready=True, solid_body_clear=True)
    points = [[1.0, 2.0, 3.0], [100.0, 2.0, 3.0]]
    trace = dict(contract="ordinary-walk-swim-walk-v1", observed_simulation_ticks=200,
                 swimming_ticks=60, wet_swimming_ticks=60, dry_ticks=140, entry_count=1, exit_count=1,
                 swimming_distance=20.0, maximum_sampled_water_depth=3.0, minimum_oxygen=89.0,
                 all_terrain_ready=True, all_solid_bodies_clear=True, swimming_at_end=False,
                 transitions=[dict(tick=0, mode="walking"), dict(tick=101, mode="swimming", **transition),
                              dict(tick=161, mode="walking", **transition)],
                 dry_segments=[first], current_dry_segment=last, failure=None)
    route = dict(name="grand-lake-foothill-water-crossing", category="walk_swim_walk", status="PASS",
                 completed_segments=1, required_segments=1, waypoints=points, simulation_ticks=200,
                 body_dimensions=[0.5, 1.2, 0.5], mixed_crossing=trace,
                 samples=[dict(settling_ticks=40, remaining=1.0,
                    endpoint=dict(grounded=True, support_valid=True, volume_valid=True))])
    return dict(kind="grand-mixed-water-crossing-v1", status="PASS", selected_route=None,
                source_frame="grand-lake-foothill-crossing", authored_endpoints=points, route=route)


class GrandMixedCrossingReceiptTests(unittest.TestCase):
    def test_complete_distinct_mixed_receipt_passes(self):
        grand_verify.validate_crossing_receipt(valid_crossing())

    def test_missing_modes_water_distance_or_completed_ticks_rejected(self):
        for change in (dict(swimming_ticks=0), dict(wet_swimming_ticks=0), dict(exit_count=0),
                       dict(swimming_at_end=True), dict(swimming_distance=0.2),
                       dict(observed_simulation_ticks=199), dict(all_terrain_ready=False),
                       dict(all_solid_bodies_clear=False), dict(transitions=[])):
            with self.subTest(change=change), self.assertRaises(RuntimeError):
                receipt = valid_crossing()
                receipt["route"]["mixed_crossing"].update(change)
                grand_verify.validate_crossing_receipt(receipt)

    def test_entry_drop_or_missing_dry_tick_cannot_hide_in_water(self):
        for change in (dict(maximum_unsupported_descent=1.4), dict(observed_simulation_ticks=100)):
            with self.subTest(change=change), self.assertRaises(RuntimeError):
                receipt = valid_crossing()
                receipt["route"]["mixed_crossing"]["dry_segments"][0].update(change)
                grand_verify.validate_crossing_receipt(receipt)

    def test_partial_or_changed_endpoint_or_unsettled_exit_cannot_pass(self):
        for case in ("partial", "endpoint", "settling", "wet_end"):
            with self.subTest(case=case), self.assertRaises(RuntimeError):
                receipt = valid_crossing()
                if case == "partial":
                    receipt.update(status="PARTIAL", selected_route="grand-lake-foothill-water-crossing")
                elif case == "endpoint":
                    receipt["route"]["waypoints"] = [[1, 2, 3], [50, 2, 3]]
                elif case == "settling":
                    receipt["route"]["samples"][0]["settling_ticks"] = 0
                else:
                    receipt["route"]["samples"][0]["endpoint"]["support_valid"] = False
                grand_verify.validate_crossing_receipt(receipt)

    def test_walking_gate_collects_and_requires_both_verdicts(self):
        for results in ([valid_receipt(), valid_crossing()],
                        [RuntimeError("dry failed"), valid_crossing()],
                        [valid_receipt(), RuntimeError("crossing failed")]):
            with self.subTest(results=results), patch.object(grand_verify, "execute_traversal", side_effect=results) as run:
                report = {}
                if any(isinstance(result, Exception) for result in results):
                    with self.assertRaises(RuntimeError):
                        grand_verify.execute_walking_bundle(Path("binary"), Path("output"), {}, report)
                else:
                    grand_verify.execute_walking_bundle(Path("binary"), Path("output"), {}, report)
                self.assertEqual([call.args[1] for call in run.call_args_list], ["walking", "crossing"])
                self.assertEqual(set(report), {"walking", "crossing"})


if __name__ == "__main__":
    unittest.main()
