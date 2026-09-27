"""Typed walking-receipt regressions; no Cargo, package, or game window."""
from __future__ import annotations

import copy
import math
from pathlib import Path
import sys
import unittest

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
             "grand-lake-foothill-crossing", "grand-lake-foothill-uphill", "crystal_ascent"]
    grounding = dict(contract="one-voxel-grounded-walk-v1", limits=limits,
                     observed_simulation_ticks=120, total_airborne_ticks=20,
                     maximum_airborne_ticks=21, maximum_unsupported_descent=0.35,
                     completed_airborne_episodes=1, airborne_at_end=False, failure=None)
    return dict(kind="grand-ordinary-walking-r03", status="PASS", selected_route=None,
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


if __name__ == "__main__":
    unittest.main()
