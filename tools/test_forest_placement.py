"""Collision, support and clearance contracts of terrain-aware tree authoring."""
from copy import deepcopy
import unittest

from forest_expedition import disk
from forest_placement import PlacementWorld, TreeShape, TreeLibrary, rotate, world_distance
from forest_world import Raw


def flat_fixture():
    survey = {"version": 1, "world_id": "test", "manifest_fingerprint": 1,
              "materials": [{"id": "stone", "solid": True}, {"id": "water", "solid": False}],
              "columns": [{"position": {"q": q, "r": r}, "runs": [{"bottom": 0, "top": 11, "material": "stone"}]}
                          for q, r in sorted(disk((0, 0), 20))], "objects": [], "liquids": []}
    metadata = {"world_id": "test", "routes": {}, "encounters": {}}
    return survey, metadata


def shape(*, branch_bottom=6):
    return TreeShape("test", 0, ((0,0,0,10,"timber"), (1,0,0,4,"timber"),
                                  (2,0,branch_bottom,10,"foliage")), ((0,0),(1,0)), frozenset({(2,0)}))


class TerrainAwarePlacement(unittest.TestCase):
    def test_multicolumn_foundation_is_exact_and_failed_attempt_is_atomic(self):
        survey, metadata = flat_fixture()
        world = PlacementWorld(survey, metadata)
        world.surface[1,0] = 12
        before = deepcopy(world.__dict__)
        self.assertIsNone(world.try_tree("oak", (0,0), shape()))
        self.assertEqual(before, world.__dict__)
        result = world.try_tree("oak", (0,0), shape(), flatten=True)
        self.assertEqual(result["ground_contacts"], [(0,0,12),(1,0,12)])
        self.assertEqual(world.foundations, {(0,0): 12})
        self.assertTrue(all(world.surface[q,r] == z for q,r,z in result["ground_contacts"]))
        self.assertTrue(all(low > world.surface[q,r] for q,r,low,_,_ in result["occupied_runs"]))

    def test_protected_travel_permits_high_canopy_but_rejects_low_bough_and_root(self):
        survey, metadata = flat_fixture()
        metadata["routes"]["trail"] = {"ribbon": [(2,0,10)], "clearance_levels": 4}
        world = PlacementWorld(survey, metadata)
        self.assertIsNone(world.try_tree("low", (0,0), shape(branch_bottom=3)))
        self.assertIsNone(world.try_tree("on-road", (2,0), shape()))
        self.assertIsNotNone(world.try_tree("high", (0,0), shape(branch_bottom=4)))

    def test_exact_overlap_is_rejected_while_distinct_vertical_volumes_are_allowed(self):
        survey, metadata = flat_fixture()
        world = PlacementWorld(survey, metadata)
        world.occupied[2,0].append((11,15,"low-rock"))
        self.assertIsNotNone(world.try_tree("oak", (0,0), shape()))
        before = deepcopy(world.__dict__)
        self.assertIsNone(world.try_tree("duplicate", (0,0), shape()))
        self.assertEqual(before, world.__dict__)

    def test_roots_reject_liquid_and_reserved_architecture_even_when_center_is_free(self):
        survey, metadata = flat_fixture()
        survey["liquids"] = [{"column": {"q":1,"r":0}, "bottom":11, "top":13}]
        world = PlacementWorld(survey, metadata)
        self.assertIsNone(world.try_tree("wet-buttress", (0,0), shape(), flatten=True))
        world.wet.clear()
        world.root_exclusion.add((1,0))
        self.assertIsNone(world.try_tree("wall-buttress", (0,0), shape(), flatten=True))

    def test_unintended_hillside_branch_contact_is_not_treated_as_a_tree_root(self):
        survey, metadata = flat_fixture()
        world = PlacementWorld(survey, metadata)
        world.surface[2,0] = 17
        self.assertIsNone(world.try_tree("embedded", (0,0), shape()))
        world.surface[2,0] = 16
        self.assertIsNone(world.try_tree("touching", (0,0), shape()))
        world.surface[2,0] = 15
        self.assertIsNotNone(world.try_tree("clear", (0,0), shape()))

    def test_rejected_prop_does_not_change_foundations_or_occupancy(self):
        survey, metadata = flat_fixture()
        world = PlacementWorld(survey, metadata)
        world.occupied[1,0].append((12,15,"existing"))
        dto = {"id": "invalid", "foundation_levels": {(0,0):12,(1,0):13},
               "occupied_world": [(0,0,13),(1,0,14)], "reserved_columns": [], "clear_columns": []}
        before = deepcopy(world.__dict__)
        with self.assertRaisesRegex(ValueError, "buries"):
            world.reserve_structure(dto)
        self.assertEqual(before, world.__dict__)

    def test_six_rotations_preserve_exact_grounding_and_foliage_footprint(self):
        library = TreeLibrary(Raw)
        for name, geometry in library.geometry.items():
            baseline = library.shapes[name,0]
            for turn in range(6):
                rotated = library.shapes[name,turn]
                self.assertEqual(set(rotated.roots), {rotate(p,turn) for p in geometry.roots})
                self.assertEqual(rotated.canopy, {rotate(p,turn) for p in geometry.canopy_columns})
                self.assertEqual(sum(high-low for _,_,low,high,_ in rotated.runs),
                                 sum(high-low for _,_,low,high,_ in baseline.runs))
        # A radius9 local clearing leaves >=9 world units beyond even the widest
        # ordinary landmark root. This is walking room, not a canopy exclusion.
        ordinary_roots = [p for name,g in library.geometry.items() if name.startswith("landmark") for p in g.roots]
        outer = disk((0,0),10) - disk((0,0),9)
        self.assertGreaterEqual(min(world_distance(a,b) for a in ordinary_roots for b in outer), 9)


if __name__ == "__main__":
    unittest.main()
