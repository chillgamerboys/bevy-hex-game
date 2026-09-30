"""Pure radius-187 expedition geography proxy; does not modify live game assets.

``recipe(raw=forest_world.Raw)`` returns a V4 recipe and JSON-safe authored site
metadata. ``document(raw=Raw, ron=ron)`` wraps that recipe in a complete source.
Metadata levels are intended, exactly graded authoring facts. The runtime adapter
must validate them against the compiled final world before publishing encounters.
There are deliberately no tree placements, ornaments or asset dependencies yet.
"""
from __future__ import annotations

from collections import deque
import random

RADIUS = 187
SEED = 20260911
LEVEL_HEIGHT = .35
WORLD_ID = "forest-massif-expedition"
DIRECTIONS = ((1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1))
CAMP_COUNTS = (3, 3, 3, 3, 3, 5, 5, 5, 9, 9, 11, 13, 15, 20)
CAMP_CENTERS = ((-46, 13, 44), (-46, -24, 42), (-59, 39, 47),
                (-72, -4, 48), (-59, -55, 44), (-88, 57, 52),
                (-98, 16, 53), (-89, -51, 48), (-116, 75, 54),
                (-145, 62, 56), (-122, -34, 53), (-157, 9, 56),
                (-151, 36, 56), (-125, 12, 56))
HEART = (-143, 18, 56)
ANCHORS = {f"forest_camp_{i:02}": p for i, p in enumerate(CAMP_CENTERS, 1)} | {
    "party_start": (0, 0, 58), "bridge_center": (0, 0, 58), "bridge_west": (-28, 0, 44),
    "bridge_east": (28, 0, 44), "forest_troll": (-142, 4, 56),
    "heart_north": (-135, 39, 56), "ancient_tree": HEART,
    "dragon_lower": (65, 25, 80), "dragon_middle": (120, -18, 160),
    "dragon_upper": (125, -90, 260), "mountain_shadow": (100, 64, 56),
    "ascent_entry": (54, -15, 57), "ascent_rest_01": (130, 30, 125),
    "ascent_rest_02": (85, -45, 195), "ascent_rest_03": (145, -60, 235),
    "shadow_turn": (52, 52, 50), "shadow_gate": (86, 71, 56),
}
PLAIN_GOLEMS = ((15, 72, 40), (5, 115, 40), (35, 137, 40))
PLAIN_WISPS = ((35, 60, 40), (20, 95, 40), (0, 145, 40))
ANCHORS.update({f"plain_golem_{i:02}": p for i, p in enumerate(PLAIN_GOLEMS, 1)})
ANCHORS.update({f"plain_wisp_{i:02}": p for i, p in enumerate(PLAIN_WISPS, 1)})
ANCHORS["plain_path_entry"] = (33, 42, 40)
RIVER = ((87, -174, 34), (77, -128, 34), (28, -75, 34), (30, -38, 34),
         (0, 0, 34), (-8, 45, 34), (-52, 92, 34), (-55, 133, 34), (-87, 174, 34))
BRIDGE = ((-28, 0, 44), (-20, 0, 46), (-10, 0, 54), (0, 0, 58),
          (10, 0, 54), (20, 0, 46), (28, 0, 44))
FOUNTAINS = {
    "forest_fountain_01": (-92, 126, 46), "forest_fountain_02": (-56, -96, 45),
    "forest_fountain_03": (-171, 54, 55), "forest_fountain_04": (-142, -34, 52),
    "mountain_fountain_01": (44, 97, 52), "mountain_fountain_02": (159, -93, 232),
}
FOUNTAIN_ENTRIES = {name: (p[0], p[1] - 3, p[2] - 2) for name, p in FOUNTAINS.items()}
ANCHORS.update({f"{name}_approach": p for name, p in FOUNTAIN_ENTRIES.items()})
ANCHORS.update({f"{name}_turn": (p[0] + (10 if name == "mountain_fountain_02" else 6),
                                       p[1] - (8 if name == "mountain_fountain_02" else 6), p[2] - 2)
                for name, p in FOUNTAINS.items()})


def distance(a, b=(0, 0)):
    q, r = a[0] - b[0], a[1] - b[1]
    return max(abs(q), abs(r), abs(q + r))


def disk(center, radius):
    q, r = center[:2]
    return {(q + dq, r + dr) for dq in range(-radius, radius + 1)
            for dr in range(max(-radius, -dq - radius), min(radius, -dq + radius) + 1)}


def line(a, b):
    """Exact rational cube rasterization with lexicographic nearest-cell ties."""
    n = distance(a, b)
    if not n:
        return [a[:2]]
    points = []
    for step in range(n + 1):
        q, r = (a[i] * (n - step) + b[i] * step for i in range(2))
        choices = []
        for cq in (q // n, q // n + 1):
            for cr in (r // n, r // n + 1):
                dq, dr = q - cq * n, r - cr * n
                choices.append((dq * dq + dr * dr + (dq + dr) ** 2, cq, cr))
        _, cq, cr = min(choices)
        if not points or points[-1] != (cq, cr):
            points.append((cq, cr))
    return points


def ribbon(points, width):
    return set().union(*(disk(p, width) for p in points))


def graded_segment(a, b, width):
    """Lipschitz-envelope midpoint over a complete ribbon, not linear height.

    This follows V4's public ordinary-step contract. Integration still checks
    the compiled supports, since later operators can reject or alter a proposal.
    """
    path = line(a, b)
    mask = ribbon(path, width)

    def distances(start):
        result, queue = {start: 0}, deque([start])
        while queue:
            q, r = queue.popleft()
            for dq, dr in DIRECTIONS:
                p = q + dq, r + dr
                if p in mask and p not in result:
                    result[p] = result[q, r] + 1
                    queue.append(p)
        return result

    da, db = distances(a[:2]), distances(b[:2])
    if da[b[:2]] < abs(a[2] - b[2]):
        raise ValueError(f"route is too short for an ordinary climb: {a} -> {b}")
    levels = {}
    for p in sorted(mask):
        low = max(a[2] - da[p], b[2] - db[p])
        high = min(a[2] + da[p], b[2] + db[p])
        levels[p] = low + (high - low) // 2
    return [(*p, levels[p]) for p in path], [(*p, level) for p, level in levels.items()]


def _routes():
    camp = lambda n: f"forest_camp_{n:02}"
    edges = [("bridge_west", camp(1), 2), ("bridge_west", camp(2), 2)]
    edges += [(camp(a), camp(b), 1 if a in (3, 8, 10) else 2) for a, b in
              ((1, 3), (3, 6), (6, 9), (9, 10), (10, 13), (2, 4), (4, 7),
               (7, 14), (2, 5), (5, 8), (8, 11), (11, 12), (6, 7), (13, 12))]
    edges += [(camp(13), "heart_north", 2), ("heart_north", camp(14), 2),
              (camp(12), "forest_troll", 2), (camp(14), "forest_troll", 2)]
    mountain = [("bridge_east", "ascent_entry", 2), ("ascent_entry", "dragon_lower", 3),
                ("dragon_lower", "ascent_rest_01", 1), ("ascent_rest_01", "dragon_middle", 2),
                ("dragon_middle", "ascent_rest_02", 1), ("ascent_rest_02", "ascent_rest_03", 2),
                ("ascent_rest_03", "dragon_upper", 3), ("bridge_east", "shadow_turn", 2),
                ("shadow_turn", "shadow_gate", 2), ("shadow_gate", "mountain_shadow", 2)]
    result = [(f"forest-path-{i:02}", a, b, width) for i, (a, b, width) in enumerate(edges, 1)] + [
        (f"mountain-path-{i:02}", a, b, width) for i, (a, b, width) in enumerate(mountain, 1)]
    lowland = ("shadow_turn", "plain_path_entry", "plain_wisp_01", "plain_golem_01",
               "plain_wisp_02", "plain_golem_02", "plain_golem_03", "plain_wisp_03")
    result += [(f"plain-path-{i:02}", a, b, 1 if i % 2 else 2)
               for i, (a, b) in enumerate(zip(lowland, lowland[1:]), 1)]
    # Small unmarked spurs reach dry footing beside each pool's open entry. The
    # final one-column trail stops before water, preserving ordinary pool entry.
    for i, (name, start) in enumerate(zip(FOUNTAINS, (camp(9), camp(5), camp(10), camp(11),
                                                     "shadow_turn", "ascent_rest_03")), 1):
        result.append((f"fountain-path-{i:02}", start, f"{name}_turn", 0))
        result.append((f"fountain-entry-{i:02}", f"{name}_turn", f"{name}_approach", 0))
    return result


def _landmarks(forest, excluded):
    """Reserve roots and local walking clearings before understory distribution."""
    candidates = [p for p in sorted(forest) if distance(p) <= 168 and not p[0] % 4 and not p[1] % 4]
    random.Random(SEED).shuffle(candidates)
    selected = []
    for p in candidates:
        deep = distance(p, HEART) < 65
        radius = 12 if deep else 9
        clearing = disk(p, radius)
        if not clearing <= forest or clearing & excluded:
            continue
        if any(distance(p, other["center"]) < radius + other["clearing_radius"] + 3 for other in selected):
            continue
        selected.append({"id": f"landmark-{len(selected) + 1:02}", "center": p,
                         "clearing_radius": radius, "tier": "ancient" if deep else "landmark"})
        if len(selected) == 36:
            return selected
    raise ValueError(f"only {len(selected)} of 36 landmark clearings fit; do not reduce the target")


def recipe(*, raw):
    """Return the V4 terrain proxy and versioned, JSON-serializable site facts."""
    def xy(p):
        return {"q": p[0], "r": p[1]}

    def grade(p):
        return {"column": xy(p), "level": p[2]}

    def mask(p, radius):
        return {"center": xy(p), "radius": radius}

    def override(name, p, radius, material=None):
        return {"id": name, "mask": mask(p, radius), "surface_level": raw(f"Some({p[2]})"),
                "material": raw(f'Some("{material}")') if material else None}

    sites, overrides = {}, []
    for index, (p, count) in enumerate(zip(CAMP_CENTERS, CAMP_COUNTS), 1):
        name = f"forest_camp_{index:02}"
        radius = 5 if count <= 3 else (6 if count <= 5 else (7 if count <= 11 else (8 if count < 20 else 10)))
        sites[name] = {"preferred": p, "surfaces": [(*at, p[2]) for at in sorted(disk(p, radius))],
                       "rally_entry": name, "goblins": count, "shamans": int(index in (12, 13)),
                       "profile": "baby" if index <= 5 else "adult"}
        overrides.append(override(f"camp-{index:02}", p, radius))
    for name, radius in (("forest_troll", 5), ("dragon_lower", 10), ("dragon_middle", 10),
                         ("dragon_upper", 12), ("mountain_shadow", 12)):
        p = ANCHORS[name]
        sites[name] = {"preferred": p, "surfaces": [(*at, p[2]) for at in sorted(disk(p, radius))],
                       "rally_entry": name if name == "forest_troll" else None}
        overrides.append(override(f"site-{name}", p, radius))
    for kind, centers, radius, clearance in (("golem", PLAIN_GOLEMS, 8, 16), ("wisp", PLAIN_WISPS, 6, 96)):
        for i, p in enumerate(centers, 1):
            name = f"plain_{kind}_{i:02}"
            sites[name] = {"preferred": p, "surfaces": [(*at, p[2]) for at in sorted(disk(p, radius))],
                           "rally_entry": None, "clearance_levels": clearance,
                           "count": 1 if kind == "golem" else (3, 3, 4)[i-1]}
            overrides.append(override(f"site-{name}", p, radius))
    overrides.append(override("heart-clearing", HEART, 28))
    routes, graph, route_levels = [], {}, {}
    for name, start, end, width in _routes():
        a, b = ANCHORS[start], ANCHORS[end]
        path, surface = graded_segment(a, b, width)
        for q, r, level in surface:
            old = route_levels.setdefault((q, r), level)
            if old != level:
                raise ValueError(f"overlapping route grades at {(q, r)}: {old}, {level}, {name}")
        routes.append({"id": name, "points": [grade(a), grade(b)], "half_width": width,
                       "shoulder_width": 4, "material": "moss" if name.startswith("forest") else "gravel"})
        graph[name] = {"from": start, "to": end, "supports": path, "ribbon": surface, "clearance_levels": 4,
                       "half_width": width, "purpose": "rally" if name.startswith("forest") else "travel"}
    # The wall is terrain in the proxy. The broad western gate stays open;
    # buttresses, lintels and ornament blueprints belong to the content pass.
    arena = ANCHORS["mountain_shadow"]
    wall = disk(arena, 15) - disk(arena, 12)
    gate = {(q, r) for q, r in wall if q - arena[0] <= -11 and 3 <= r - arena[1] <= 11}
    wall -= gate
    for q, r in sorted(wall):
        overrides.append(override(f"wall-{q}-{r}", (q, r, arena[2] + 36), 0, "stone"))

    river_path = []
    for a, b in zip(RIVER, RIVER[1:]):
        river_path.extend(line(a, b)[bool(river_path):])
    wet = ribbon(river_path, 13)
    region = disk((0, 0), RADIUS)
    left_bank = {r: min(q for q, z in wet if z == r) for r in range(-RADIUS, RADIUS + 1)}
    forest = {(q, r) for q, r in region if q < left_bank[r] - 4}
    pools = {}
    for name, p in FOUNTAINS.items():
        pools[name] = {"cells": [(*at, level) for at in sorted(disk(p, 2)) for level in (p[2] - 1, p[2])],
                       "center": p, "heal": 40, "uses": 1, "discovery": "unmarked"}
    reserved = set(route_levels) | disk(HEART, 28)
    reserved |= set().union(*(set((p[0], p[1]) for p in site["surfaces"]) for site in sites.values()))
    reserved |= set().union(*(disk(p, 6) for p in FOUNTAINS.values()))
    landmarks = _landmarks(forest, reserved)
    understory_excluded = reserved | set().union(*(disk(p["center"], p["clearing_radius"]) for p in landmarks))

    landforms = []
    def land(name, centers, radius, plateau, rise, relief=0):
        landforms.append({"id": name, "centers": [xy(p) for p in centers], "radius": radius,
                          "plateau_radius": plateau, "rise": rise, "relief": relief})

    for i, (p, radius, rise) in enumerate((((-61, 56), 32, 18), ((-85, -7), 38, 24),
            ((-108, 88), 35, 22), ((-132, -22), 38, 26), ((-160, 51), 27, 16),
            ((-70, -80), 35, 18), ((-121, 28), 43, 28)), 1):
        land(f"forest-hill-{i:02}", [p], radius, 4, rise, 2)
    land("forest-gully-north", [(-82, 76), (-100, 94), (-112, 105)], 12, 1, -9, 1)
    land("forest-gully-south", [(-75, -33), (-91, -24), (-106, -15)], 13, 1, -10, 1)
    land("forest-ground-undulation", [(-95, 20)], 90, 1, 0, 3)
    land("massif-body", [(116, -45), (128, -65)], 75, 10, 215, 8)
    land("massif-crown", [(123, -93), (137, -101), (145, -84)], 37, 2, 108, 7)
    land("massif-cleft", [(114, -57), (108, -45)], 18, 1, -44, 3)
    land("north-companion", [(107, -138), (114, -140)], 29, 2, 133, 7)
    land("east-companion", [(164, -55)], 22, 1, 151, 5)
    land("south-companion", [(66, 86), (71, 96)], 32, 3, 95, 6)
    land("lower-buttress", [(65, 25)], 37, 11, 40, 4)
    land("upper-spring-shelf", [(159, -93)], 12, 3, 155, 2)
    value = {"base_level": 40,
             "strata": {"bedrock": "bedrock", "rock": "basalt", "soil": "soil", "soil_depth": 4, "surface": "grass"},
             "landforms": landforms,
             "biomes": [{"id": name, "mask": mask(p, radius), "priority": priority, "material": material}
                        for name, p, radius, priority, material in (
                            ("pine-uplands", (-99, 24), 66, 2, "pine-floor"),
                            ("ancient-grove", (-138, 18), 48, 3, "moss"),
                            ("massif-rock", (107, -47), 78, 4, "basalt"),
                            ("massif-snow", (126, -94), 34, 5, "snow"))],
             "channels": [{"id": "great-river", "points": [grade(p) for p in RIVER],
                           "half_width": 13, "depth": 12, "material": "water", "bed_material": "sand", "bank_width": 6}],
             "basins": [{"id": name, "mask": mask(p, 2), "water_level": p[2], "depth": 2,
                         "material": "spring-water", "bed_material": "stone", "bank_width": 4}
                        for name, p in FOUNTAINS.items()],
             "routes": routes,
             "bridges": [{"id": "central-crossing", "points": [grade(p) for p in BRIDGE],
                          "half_width": 5, "walkway_half_width": raw("Some(4)"),
                          "thickness": 3, "material": "stone"}],
             "features": [], "overrides": overrides,
             "anchors": [{"id": name, "column": xy(p), "level": raw(f"Some({p[2]})"),
                          "role": raw("Observation" if name == "ancient_tree" else "Gameplay")}
                         for name, p in ANCHORS.items()], "hub": grade(ANCHORS["party_start"])}
    metadata = {"schema_version": 1, "world_id": WORLD_ID, "status": "proxy-awaiting-compiled-grounding",
                "radius": RADIUS, "columns": len(region), "level_height": LEVEL_HEIGHT,
                "anchors": ANCHORS.copy(), "encounters": sites,
                "roster": {"goblins": 107, "shamans": 2, "trolls": 1, "dragons": 3, "shadows": 1,
                           "golems": 3, "wisps": 10, "enemies": 127, "actors_including_player": 128},
                "route_nodes": {name: ANCHORS[name] for edge in graph.values() for name in (edge["from"], edge["to"])},
                "routes": graph, "fountains": pools,
                "river": {"centerline": river_path, "wet_columns": sorted(wet), "water_level": 34},
                "bridge": {"controls": BRIDGE, "half_width": 4, "thickness": 3,
                           "open_portals": [(-23, 0), (23, 0)]},
                "arena": {"interior": sorted(disk(arena, 12)), "walls": sorted(wall), "gate": sorted(gate),
                          "floor_level": arena[2], "wall_top": arena[2] + 36},
                "forest": {"columns": sorted(forest), "understory_root_columns": sorted(forest - understory_excluded),
                           "landmarks": landmarks, "heart": {"center": HEART, "clearing_radius": 28},
                           "minimum_canopy_fraction": .5, "target_canopy_fraction": .6,
                           "initial_understory_count": 700},
                "pending": ["compile and verify all final supports", "tree placement and canopy coverage",
                            "bridge supports, parapets and open portal lintels", "arena ornament and gate lintel",
                            "rendered and native traversal review"]}
    return value, metadata


def document(*, raw, ron):
    """Return complete source text plus metadata; the caller chooses output paths."""
    value, metadata = recipe(raw=raw)
    return source_document(value, raw=raw, ron=ron), metadata


def source_document(value, *, raw, ron, extra_materials=()):
    """Wrap an authored recipe without parsing serialized/private world data."""
    colors = {"bedrock": (33, 38, 42, 255), "basalt": (76, 88, 101, 255), "soil": (98, 76, 49, 255),
              "grass": (105, 141, 72, 255), "moss": (49, 91, 57, 255), "pine-floor": (78, 98, 60, 255),
              "sand": (160, 155, 113, 255), "snow": (219, 231, 235, 255), "gravel": (126, 131, 120, 255),
              "water": (42, 115, 144, 190), "spring-water": (72, 217, 189, 190), "stone": (105, 112, 116, 255)}
    materials = [{"id": name, "solid": name not in ("water", "spring-water"),
                  "diggable": name not in ("bedrock", "water", "spring-water"), "color": raw(str(color))}
                 for name, color in colors.items()] + list(extra_materials)
    source = {"version": 1, "id": WORLD_ID, "seed": SEED, "materials": materials,
              "recipes": raw('{"forest-massif":' + ron(value) + '}'),
              "regions": [{"id": "forest", "recipe": "forest-massif", "origin": {"q": 0, "r": 0},
                           "radius": RADIUS, "rotation": 0}], "connections": []}
    return ron(source) + "\n"


def site_document(metadata, *, world_id, manifest_fingerprint, raw, ron):
    """Serialize only world-owned facts to the strict ``arena-sites.ron`` schema.

    The caller supplies the identity of a successfully compiled package. This
    function does not read packages or certify surfaces: the production world
    loader must validate the emitted facts against that exact compiled world.
    Duplicate positions are preserved so that admission rejects them rather
    than silently changing the proposal. Route supports retain traversal order.
    """
    if world_id != WORLD_ID or metadata.get("world_id") != world_id:
        raise ValueError("site metadata and compiled world identity must match")
    if type(manifest_fingerprint) is not int or not 0 <= manifest_fingerprint < 2**64:
        raise ValueError("manifest fingerprint must be an explicit u64 integer")

    def position(p):
        q, r, level = p
        return {"column": {"q": q, "r": r}, "level": level}

    def positions(values):
        return [position(p) for p in sorted(values)]

    encounters = []
    for name, entry in sorted(metadata["encounters"].items()):
        rally = entry["rally_entry"]
        encounters.append({"id": name, "preferred": position(entry["preferred"]),
                           "surfaces": positions(entry["surfaces"]),
                           "rally_entry": raw("Some(" + ron(rally) + ")") if rally else None})
    value = {"version": 1, "world_id": world_id, "manifest_fingerprint": manifest_fingerprint,
             "encounters": encounters,
             "route_nodes": [{"id": name, "position": position(p)}
                             for name, p in sorted(metadata["route_nodes"].items())],
             "routes": [{"id": name, "from": route["from"], "to": route["to"],
                         "clearance_levels": route["clearance_levels"],
                         "supports": [position(p) for p in route["supports"]],
                         "ribbon": positions(route["ribbon"])}
                        for name, route in sorted(metadata["routes"].items())],
             "fountains": [{"id": name, "cells": positions(pool["cells"])}
                           for name, pool in sorted(metadata["fountains"].items())]}
    encoded = ron(value) + "\n"
    if len(encoded.encode("utf-8")) > 16 * 1024 * 1024:
        raise ValueError("expedition companion exceeds 16 MiB")
    return encoded
