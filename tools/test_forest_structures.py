"""Independent geometry contract checks for expedition scenery and structures."""
import unittest
from copy import deepcopy

import forest_structures as structures


class Raw(str):
    pass


class StructureContracts(unittest.TestCase):
    @staticmethod
    def bridge_surfaces():
        return {(q, r): 58 - abs(q) // 2 for q in range(-30, 31) for r in range(-7, 8)}

    @staticmethod
    def arena_fixture():
        center = (100, 64)
        walls = structures.disk(center, 15) - structures.disk(center, 12)
        gate = {(q, r) for q, r in walls if q - center[0] <= -11 and 3 <= r - center[1] <= 11}
        walls -= gate
        surfaces = {p: (92 if p in walls else 56) for p in structures.disk(center, 19)}
        return center, walls, gate, surfaces

    def test_every_visible_cell_has_exact_matching_solid_occupancy(self):
        for shape in structures.catalog(raw=Raw)["shapes"].values():
            blueprint, intervals = shape.documents(raw=Raw)
            visible = {(row["position"]["q"], row["position"]["r"], row["position"]["level"]):
                       structures.STYLE_MATERIALS[row["style"]] for row in blueprint["placements"]}
            collision = {}
            for row in intervals:
                for level in range(row["bottom"], row["top"]):
                    key = row["offset"]["q"], row["offset"]["r"], level
                    self.assertNotIn(key, collision)
                    collision[key] = row["material"]
            self.assertEqual(visible, collision, shape.name)
            self.assertLessEqual(len(visible), 65536)
            self.assertLessEqual(shape.radius, 32)
            self.assertLessEqual(shape.height, 192)

    def test_all_shapes_are_face_connected_to_an_actual_ground_contact(self):
        for shape in structures.catalog(raw=Raw)["shapes"].values():
            cells = {(c.q, c.r, c.level) for c in shape.cells}
            seen, todo = set(), [(0, 0, 0)]
            while todo:
                at = todo.pop()
                if at in seen:
                    continue
                seen.add(at)
                q, r, level = at
                adjacent = {(q + dq, r + dr, level) for dq, dr in structures.DIRECTIONS}
                adjacent |= {(q, r, level - 1), (q, r, level + 1)}
                todo.extend((adjacent & cells) - seen)
            self.assertEqual(seen, cells, shape.name)
            self.assertTrue(shape.roots)

    def test_rocks_and_crystals_have_distinct_repeatable_silhouettes(self):
        rocks = [structures.rock(kind, seed=13) for kind in ("slab", "pillar", "ridge", "arch")]
        self.assertEqual(rocks, [structures.rock(kind, seed=13) for kind in ("slab", "pillar", "ridge", "arch")])
        self.assertEqual(len({shape.cells for shape in rocks}), 4)
        crystals = [structures.crystal(kind) for kind in ("cluster", "needle", "fan")]
        self.assertEqual(len({shape.cells for shape in crystals}), 3)
        self.assertGreater(max(shape.height for shape in rocks), 2 * min(shape.height for shape in rocks))

    def test_structures_are_opaque_and_crystals_use_emission_without_invisible_collision(self):
        styles = structures.styles(raw=Raw)
        self.assertTrue(all(style["surface_mode"] == "Opaque" and style["opacity"] == 1 for style in styles.values()))
        for name in (structures.CYAN, structures.BRIGHT):
            self.assertTrue(styles[name]["emission"].startswith("Some("))
        for cell in structures.fountain_frame().cells:
            self.assertEqual(styles[cell.style]["emission"], "None")

    def test_fountain_rim_never_occupies_pool_or_southern_entry(self):
        shape = structures.fountain_frame()
        placed = structures.placement(shape, "forest_fountain_01-rim", (20, -10, 40), raw=Raw)
        pool = structures.disk((20, -10), 2)
        self.assertFalse(pool & set(placed["reserved_columns"]))
        for q, r, _ in placed["occupied_world"]:
            self.assertNotEqual(r, -13)
        self.assertTrue(all(level == 40 for level in placed["foundation_levels"].values()))

    def test_rotated_placement_retains_exact_blueprint_offsets_and_grounding(self):
        shape = structures.rock("arch")
        original = {(c.q, c.r, c.level) for c in shape.cells}
        for turn in range(6):
            placed = structures.placement(shape, f"arch-{turn}", (-40, 25, 47), raw=Raw, rotation=turn)
            root, floor = placed["root"], placed["support_level"]
            expected = {(root[0] + q, root[1] + r, floor + 1 + level)
                        for q, r, level in (structures.rotate(cell, turn) for cell in original)}
            self.assertEqual(expected, set(placed["occupied_world"]))
            self.assertEqual(len(expected), len(shape.cells))

    def test_bridge_parapets_follow_grades_and_portals_keep_the_whole_walk_open(self):
        surfaces = self.bridge_surfaces()
        pieces = structures.bridge_assembly(surfaces, raw=Raw)
        self.assertEqual(len(pieces), 10)
        self.assertEqual(sum("portal" in piece["id"] for piece in pieces), 2)
        footings = {p: level for piece in pieces for p, level in piece["foundation_levels"].items()}
        self.assertEqual(set(footings), {(q, r) for q in range(-27, 28) for r in (-5, 5)})
        self.assertGreater(max(footings.values()) - min(footings.values()), 10)
        for piece in pieces:
            for q, r, z in piece["occupied_world"]:
                if -4 <= r <= 4:
                    self.assertIn(q, (-24, -23, -22, 22, 23, 24))
                    self.assertGreater(z - surfaces[q, r], 15)
                else:
                    self.assertIn(r, (-5, 5))
        structures.validate_assembly(pieces, surfaces=surfaces, clearance=4)

    def test_arena_ornaments_preserve_radius12_floor_and_an_open_massive_gate(self):
        center, walls, gate, surfaces = self.arena_fixture()
        pieces = structures.arena_assembly(center, walls, gate, 92, surfaces, raw=Raw)
        self.assertEqual(len(pieces), 8)
        occupied = {v for piece in pieces for v in piece["occupied_world"]}
        self.assertFalse({(q, r) for q, r, z in occupied} & structures.disk(center, 12))
        for q, r in gate:
            self.assertTrue(all((q, r, z) not in occupied for z in range(57, 93)))
        self.assertTrue(any((86, 71, z) in occupied for z in range(93, 108)), "lintel spans above the open portal")
        self.assertGreater(max(z for _, _, z in occupied), 100)
        # No pedestal or extra wall is added where a mountain already reaches it.
        buried = structures.disk((84, 80), 1) - structures.disk(center, 15)
        for p in buried:
            surfaces[p] = 98
        pieces = structures.arena_assembly(center, walls, gate, 92, surfaces, raw=Raw)
        self.assertEqual(len(pieces), 7)
        self.assertFalse(any(piece["id"] == "arena-buttress-2" for piece in pieces))

    def test_arena_assembly_accepts_public_json_coordinate_arrays(self):
        center, walls, gate, surfaces = self.arena_fixture()
        expected = structures.arena_assembly(center, walls, gate, 92, surfaces, raw=Raw)
        actual = structures.arena_assembly(list(center), [list(p) for p in walls],
                                          [list(p) for p in gate], 92, surfaces, raw=Raw,
                                          clear_columns=[list(center)])
        self.assertEqual(expected, actual)

    def test_assembly_blueprints_intervals_and_world_occupancy_are_identical(self):
        pieces = structures.bridge_assembly(self.bridge_surfaces(), raw=Raw)
        center, walls, gate, surfaces = self.arena_fixture()
        pieces += structures.arena_assembly(center, walls, gate, 92, surfaces, raw=Raw)
        for piece in pieces:
            visual = {(p["position"]["q"], p["position"]["r"], p["position"]["level"])
                      for p in piece["blueprint"]["placements"]}
            collision = {(p["offset"]["q"], p["offset"]["r"], z)
                         for p in piece["intervals"] for z in range(p["bottom"], p["top"])}
            self.assertEqual(visual, collision, piece["id"])
            root, floor = piece["root"], piece["support_level"]
            self.assertEqual({(root[0] + q, root[1] + r, floor + 1 + z) for q, r, z in visual}, set(piece["occupied_world"]))
            self.assertLessEqual(piece["blueprint"]["bounds"]["radius"], 32)
            self.assertLessEqual(piece["blueprint"]["bounds"]["height"], 192)

    def test_assembly_validation_rejects_overlap_wrong_support_and_blocked_travel(self):
        surfaces = self.bridge_surfaces()
        pieces = structures.bridge_assembly(surfaces, raw=Raw)
        with self.assertRaisesRegex(ValueError, "overlapping"):
            structures.validate_assembly(pieces + pieces[:1], surfaces=surfaces, clearance=4)
        changed = dict(surfaces)
        changed[pieces[0]["root"]] -= 1
        with self.assertRaisesRegex(ValueError, "footing"):
            structures.validate_assembly(pieces, surfaces=changed, clearance=4)
        blocked = deepcopy(pieces)
        blocked[0]["occupied_world"] += ((0, 0, surfaces[0, 0] + 4),)
        with self.assertRaisesRegex(ValueError, "movement clearance"):
            structures.validate_assembly(blocked, surfaces=surfaces, clearance=4)
        del surfaces[-27, 5]
        with self.assertRaisesRegex(ValueError, "support survey"):
            structures.bridge_assembly(surfaces, raw=Raw)


if __name__ == "__main__":
    unittest.main()
