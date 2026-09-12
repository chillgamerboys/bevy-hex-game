"""Independent geometry contract checks for expedition scenery and structures."""
import unittest

import forest_structures as structures


class Raw(str):
    pass


class StructureContracts(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
