"""Full-footprint authoring invariants for the undecorated expedition proxy."""
from collections import deque
import json
import unittest

import forest_expedition as world
from forest_world import Raw, ron


class ExpeditionProxy(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.recipe, cls.metadata = world.recipe(raw=Raw)

    def test_full_scale_exact_roster_and_dry_spacious_deployments(self):
        meta = self.metadata
        self.assertEqual((meta["radius"], meta["columns"]), (187, 105469))
        camps = [meta["encounters"][f"forest_camp_{i:02}"] for i in range(1, 15)]
        self.assertEqual([c["goblins"] for c in camps], [3, 3, 3, 3, 3, 5, 5, 5, 9, 9, 11, 13, 15, 20])
        self.assertEqual(sum(c["goblins"] for c in camps), 107)
        self.assertEqual([i for i, c in enumerate(camps, 1) if c["shamans"]], [12, 13])
        self.assertEqual(sum(c["goblins"] for c in camps if c["profile"] == "baby"), 15)
        self.assertEqual(meta["roster"]["actors_including_player"], 115)
        for operator in self.recipe["biomes"] + self.recipe["basins"] + self.recipe["overrides"]:
            mask = operator["mask"]
            self.assertLessEqual(world.distance((mask["center"]["q"], mask["center"]["r"])) + mask["radius"], 187)
        wet = set(meta["river"]["wet_columns"]) | {p[:2] for pool in meta["fountains"].values() for p in pool["cells"]}
        for name, site in meta["encounters"].items():
            with self.subTest(site=name):
                self.assertIn(site["preferred"], site["surfaces"])
                self.assertTrue(all(world.distance(p) <= 187 and p[:2] not in wet for p in site["surfaces"]))
                self.assertGreaterEqual(len(site["surfaces"]), 10 * site.get("goblins", 1))
                if site["rally_entry"]:
                    self.assertIn(meta["route_nodes"][site["rally_entry"]], site["surfaces"])

    def test_curved_river_spans_the_region_and_has_only_one_arched_crossing(self):
        meta = self.metadata
        wet = set(meta["river"]["wet_columns"])
        self.assertEqual({r for _, r in wet}, set(range(-187, 188)))
        self.assertTrue(all(world.distance(p) <= 187 for p in wet))
        remaining = set(wet)
        queue = [remaining.pop()]
        while queue:
            q, r = queue.pop()
            for dq, dr in world.DIRECTIONS:
                p = q + dq, r + dr
                if p in remaining:
                    remaining.remove(p)
                    queue.append(p)
        self.assertFalse(remaining)
        center = meta["river"]["centerline"]
        self.assertGreater(max(2*q+r for q, r in center) - min(2*q+r for q, r in center), 25)
        self.assertEqual(len(self.recipe["bridges"]), 1)
        controls = meta["bridge"]["controls"]
        self.assertEqual(meta["anchors"]["party_start"], (0, 0, 58))
        self.assertGreater(controls[len(controls)//2][2], controls[0][2] + 10)
        self.assertEqual(controls[0][2], controls[-1][2])
        for a, b in zip(controls, controls[1:]):
            self.assertLessEqual(abs(a[2] - b[2]), world.distance(a, b))

    def test_complete_route_ribbons_are_dry_connected_and_preserve_camps(self):
        meta = self.metadata
        wet = set(meta["river"]["wet_columns"])
        all_levels = {}
        for name, route in meta["routes"].items():
            with self.subTest(route=name):
                path, ribbon = route["supports"], route["ribbon"]
                self.assertEqual(path[0], meta["route_nodes"][route["from"]])
                self.assertEqual(path[-1], meta["route_nodes"][route["to"]])
                self.assertGreaterEqual(route["clearance_levels"], 3)
                levels = {p[:2]: p[2] for p in ribbon}
                for a, b in zip(path, path[1:]):
                    self.assertEqual(world.distance(a, b), 1)
                    self.assertLessEqual(abs(a[2] - b[2]), 1)
                for p, level in levels.items():
                    self.assertNotIn(p, wet)
                    self.assertLessEqual(world.distance(p), 187)
                    self.assertEqual(all_levels.setdefault(p, level), level)
                    for dq, dr in world.DIRECTIONS:
                        n = p[0] + dq, p[1] + dr
                        if n in levels:
                            self.assertLessEqual(abs(level - levels[n]), 1)
                self.assertTrue(set(path) <= set(ribbon))
        for name, site in meta["encounters"].items():
            for q, r, level in site["surfaces"]:
                if (q, r) in all_levels:
                    self.assertEqual(all_levels[q, r], level, f"road changes {name} support {(q, r)}")
        for p in world.disk(world.HEART, 28):
            if p in all_levels:
                self.assertEqual(all_levels[p], world.HEART[2], f"road changes Heart clearing {p}")

    def test_forest_graph_reconnects_and_every_camp_can_rally_to_troll(self):
        graph = {}
        edges = [r for r in self.metadata["routes"].values() if r["purpose"] == "rally"]
        for edge in edges:
            graph.setdefault(edge["from"], set()).add(edge["to"])
            graph.setdefault(edge["to"], set()).add(edge["from"])
        reached, queue = {"forest_troll"}, deque(["forest_troll"])
        while queue:
            for neighbor in graph[queue.popleft()] - reached:
                reached.add(neighbor)
                queue.append(neighbor)
        self.assertTrue({f"forest_camp_{i:02}" for i in range(1, 15)} <= reached)
        self.assertIn("bridge_west", reached)
        self.assertGreaterEqual(len(edges), len(graph))  # At least one reconnection loop.
        self.assertTrue(all(world.distance(p[:2], world.HEART) > 6
                            for edge in edges for p in edge["ribbon"]))

    def test_dragon_altitudes_and_intended_switchbacks_preserve_encounter_order(self):
        anchors = self.metadata["anchors"]
        self.assertEqual([anchors[n][2] for n in ("dragon_lower", "dragon_middle", "dragon_upper")], [80, 160, 260])
        routes = [self.metadata["routes"][f"mountain-path-{i:02}"] for i in range(1, 8)]
        self.assertEqual(routes[0]["from"], "bridge_east")
        self.assertEqual(routes[-1]["to"], "dragon_upper")
        self.assertEqual([r["to"] for r in routes if r["to"].startswith("dragon")],
                         ["dragon_lower", "dragon_middle", "dragon_upper"])
        for previous, following in zip(routes, routes[1:]):
            self.assertEqual(previous["to"], following["from"])
        self.assertGreater(sum(len(r["supports"]) - 1 for r in routes), 300)
        self.assertEqual({r["half_width"] for r in routes}, {1, 2, 3})

    def test_arena_matches_duel_interior_and_has_a_walkable_open_gate(self):
        arena = self.metadata["arena"]
        self.assertEqual(len(arena["interior"]), 469)
        self.assertGreaterEqual((arena["wall_top"] - arena["floor_level"]) * .35, 12)
        self.assertTrue(arena["gate"])
        walls = set(arena["walls"])
        for name in ("mountain-path-09", "mountain-path-10"):
            gate_route = self.metadata["routes"][name]
            self.assertFalse(walls & {p[:2] for p in gate_route["ribbon"]})
        self.assertIn(self.metadata["anchors"]["shadow_gate"][:2], arena["gate"])

    def test_six_hidden_fountains_are_finite_and_cover_forest_and_mountains(self):
        pools = self.metadata["fountains"]
        self.assertEqual(len(pools), 6)
        self.assertEqual(sum(name.startswith("forest") for name in pools), 4)
        self.assertEqual(sum(name.startswith("mountain") for name in pools), 2)
        all_cells = set()
        for pool in pools.values():
            self.assertEqual((pool["heal"], pool["uses"], pool["discovery"]), (40, 1, "unmarked"))
            self.assertEqual(len(pool["cells"]), 38)
            self.assertFalse(all_cells & set(pool["cells"]))
            self.assertTrue(all(world.distance(p) <= 187 for p in pool["cells"]))
            all_cells.update(pool["cells"])
        self.assertGreater(max(p["center"][0] for p in pools.values()) - min(p["center"][0] for p in pools.values()), 300)

    def test_landmark_reservations_keep_local_clearings_and_full_forest_mask(self):
        forest = self.metadata["forest"]
        self.assertEqual(len(forest["landmarks"]), 36)
        self.assertEqual(forest["heart"]["clearing_radius"], 28)
        columns = set(forest["columns"])
        roots = set(forest["understory_root_columns"])
        self.assertGreater(len(columns), 45000)
        self.assertGreater(len(roots), 30000)
        self.assertTrue(roots <= columns)
        for landmark in forest["landmarks"]:
            clearing = world.disk(landmark["center"], landmark["clearing_radius"])
            self.assertFalse(roots & clearing)
            self.assertTrue(clearing <= columns)
        self.assertEqual((forest["minimum_canopy_fraction"], forest["target_canopy_fraction"]), (.5, .6))
        self.assertEqual(self.recipe["features"], [])

    def test_source_and_metadata_are_deterministic_and_explicitly_provisional(self):
        source, metadata = world.document(raw=Raw, ron=ron)
        source_again, metadata_again = world.document(raw=Raw, ron=ron)
        self.assertEqual(source, source_again)
        self.assertEqual(json.dumps(metadata, sort_keys=True), json.dumps(metadata_again, sort_keys=True))
        self.assertEqual(metadata["status"], "proxy-awaiting-compiled-grounding")
        self.assertIn("radius:187", source)
        self.assertIn("features:[]", source)
        self.assertIn('id:"spring-water",solid:false', source)


if __name__ == "__main__":
    unittest.main()
