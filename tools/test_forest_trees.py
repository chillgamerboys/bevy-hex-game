"""Structural acceptance for expedition tree authoring, independent of rendering."""
import random
import unittest

import forest_trees as trees
from forest_world import Raw, ron


NEIGHBORS = ((1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0),
             (1, -1, 0), (-1, 1, 0), (0, 0, 1), (0, 0, -1))


def connected(cells):
    remaining = set(cells)
    frontier = [(0, 0, 0)]
    remaining.remove(frontier[0])
    while frontier:
        q, r, level = frontier.pop()
        for dq, dr, dy in NEIGHBORS:
            point = q + dq, r + dr, level + dy
            if point in remaining:
                remaining.remove(point)
                frontier.append(point)
    return not remaining


class ForestTrees(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.corpus = {(name, seed): trees.generate_tree(spec.height, spec.radius, spec.pine, seed=seed)
                      for name, spec in trees.PRESETS.items() for seed in (0, 7, 103)}

    def test_all_voxels_and_woody_supports_connect_to_ground(self):
        for key, geometry in self.corpus.items():
            with self.subTest(tree=key):
                self.assertTrue(connected((c.q, c.r, c.level) for c in geometry.cells))
                self.assertTrue(connected((c.q, c.r, c.level) for c in geometry.cells
                                          if c.part != "Foliage"))

    def test_exact_dimensions_stay_within_catalog_limits(self):
        for (name, seed), geometry in self.corpus.items():
            with self.subTest(tree=name, seed=seed):
                positions = {(c.q, c.r, c.level) for c in geometry.cells}
                self.assertEqual(len(positions), len(geometry.cells))
                self.assertLessEqual(len(positions), 65536)
                self.assertEqual(min(c.level for c in geometry.cells), 0)
                self.assertEqual(max(c.level for c in geometry.cells), geometry.height - 1)
                self.assertLessEqual(geometry.height, 192)
                self.assertLessEqual(geometry.radius, 32)
                self.assertTrue(all(max(abs(c.q), abs(c.r), abs(c.q + c.r)) <= geometry.radius
                                    for c in geometry.cells))

    def test_ground_contacts_are_exact_not_nominal_trunk_disks(self):
        for key, geometry in self.corpus.items():
            with self.subTest(tree=key):
                contacts = {(c.q, c.r) for c in geometry.cells if c.level == 0}
                self.assertEqual(contacts, set(geometry.roots))
                self.assertTrue(all(c.level == 0 for c in geometry.cells if c.part == "Root"))
                self.assertIn((0, 0), contacts)
                if geometry.height >= 40:
                    self.assertGreater(len(contacts), 7)
                    # Sixfold-symmetric cylinders fail this invariant; buttresses
                    # must contribute a visibly irregular ground footprint.
                    rotated = {(-r, q + r) for q, r in contacts}
                    self.assertNotEqual(contacts, rotated)

    def test_understory_leaf_clearance_and_landmark_crown_depth(self):
        for key, geometry in self.corpus.items():
            with self.subTest(tree=key):
                leaves = [c.level for c in geometry.cells if c.part == "Foliage"]
                if geometry.height <= 34:
                    self.assertGreaterEqual(min(leaves) * trees.LEVEL_HEIGHT, 3.5)
                    self.assertLessEqual(min(leaves) * trees.LEVEL_HEIGHT, 5)
                else:
                    self.assertGreaterEqual(max(leaves) - min(leaves) + 1, geometry.height * .38)
                    bottom = sum(c.level == 0 for c in geometry.cells)
                    upper = sum(c.level == int(geometry.height * .40) and c.part == "Trunk"
                                for c in geometry.cells)
                    self.assertGreater(bottom, upper)

    def test_heart_is_uniquely_tall_with_a_narrower_deeper_crown(self):
        heart = self.corpus["heart", 0]
        self.assertAlmostEqual(heart.height * trees.LEVEL_HEIGHT, 60.2)
        self.assertLess(heart.radius, 24)  # Previous plate-like Heart crown radius.
        self.assertGreater(len(heart.canopy_columns), 350)
        self.assertTrue(all(heart.height > spec.height and heart.radius > spec.radius
                            for name, spec in trees.PRESETS.items() if name != "heart"))

    def test_pure_reproducible_generation_does_not_change_global_random_state(self):
        before = random.getstate()
        other = trees.generate_tree(51, 6, seed=99)
        self.assertEqual(before, random.getstate())
        self.assertEqual(other, trees.generate_tree(51, 6, seed=99))
        self.assertNotEqual(other, trees.generate_tree(51, 6, seed=100))
        self.assertEqual(trees.generate_tree(51, 6, seed=0), self.corpus["landmark-2", 0])

    def test_blueprint_masks_and_runs_reconstruct_identical_physical_cells(self):
        geometry = self.corpus["heart", 7]
        blueprint, runs = geometry.documents("forest-heart", raw=Raw)
        expected = {(c.q, c.r, c.level): trees.STYLES[c.style] for c in geometry.cells}
        actual = {}
        for run in runs:
            self.assertLess(run["bottom"], run["top"])
            for level in range(run["bottom"], run["top"]):
                point = run["offset"]["q"], run["offset"]["r"], level
                self.assertNotIn(point, actual)
                actual[point] = run["material"]
        self.assertEqual(actual, expected)
        self.assertEqual({tuple(p["position"].values()) for p in blueprint["placements"]}, set(expected))
        self.assertEqual({tuple(p.values()) for p in blueprint["blocker_footprint"]}, set(geometry.roots))
        self.assertEqual({tuple(p.values()) for p in blueprint["canopy_occluders"]},
                         {(c.q, c.r, c.level) for c in geometry.cells if c.part == "Foliage"})
        # This is the existing helper's actual enum encoder, not a duplicate.
        encoded = ron(blueprint)
        self.assertIn("category:Plant", encoded)
        self.assertIn("part:Plant(Branch)", encoded)
        self.assertNotIn('"Plant(Foliage)"', encoded)
        self.assertEqual(trees.tree("forest-heart", 172, 14, seed=7, raw=Raw), (blueprint, runs))

    def test_invalid_or_oversized_recipes_are_rejected(self):
        for height, radius in ((17, 3), (193, 3), (40, 1), (40, 33), (18.5, 4)):
            with self.subTest(height=height, radius=radius), self.assertRaises(ValueError):
                trees.generate_tree(height, radius)
        with self.assertRaisesRegex(ValueError, "65536"):
            trees.generate_tree(192, 32)


if __name__ == "__main__":
    unittest.main()
