#!/usr/bin/env python3
"""Regenerate/check the authored Forest–Massif map, then compile using verified worldc.

This offline authoring helper emits ordinary V4 RON and exact catalog blueprints.
It does not participate in runtime generation or scale render-only geometry.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import math
from pathlib import Path
import random
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CONTENT = ROOT / "assets/config/v4/forest-massif"
SOURCE = CONTENT / "world.ron"
LEVEL_HEIGHT = .35
BLUEPRINT_SOURCE_REV = "18493cea8201d80c1d85f5aaf82ff85b0ea6c0ad"
SEED = 20260911
RADIUS = 187
GIANT = (-143, 5)
ANCHORS = {
    "party_start": (-94, 125, 40), "hostile_start": (-109, 100, 40),
    "forest_outer_a": (-109, 100, 40), "forest_outer_b": (-72, 84, 40),
    "forest_middle": (-92, 18, 40), "forest_deep_a": (-130, 55, 40),
    "forest_deep_b": (-110, -25, 40), "ancient_tree": (-132, 4, 40),
    "bridge_west": (-24, 0, 48), "bridge_east": (24, 0, 48),
    "dragon_lower": (68, 25, 80), "dragon_middle": (118, -25, 130),
    "dragon_upper": (113, -77, 190),
}
STYLES = {"plant/trunk": "timber", "plant/foliage-dark": "foliage",
          "plant/foliage-mid": "foliage", "plant/foliage-light": "foliage"}


class Raw(str):
    """An explicitly constructed RON enum/option/tuple, never user text."""


def ron(value):
    if isinstance(value, Raw):
        return str(value)
    if value is None:
        return "None"
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, dict):
        return "(" + ",".join(f"{key}:{ron(item)}" for key, item in value.items()) + ")"
    if isinstance(value, list):
        return "[\n" + ",\n".join(ron(item) for item in value) + ",\n]" if value else "[]"
    return str(value)


def mapping(value):
    return Raw("{" + ",".join(f"{ron(key)}:{ron(item)}" for key, item in value.items()) + "}")


def hexpos(q, r):
    return {"q": q, "r": r}


def dist(a, b=(0, 0)):
    q, r = a[0] - b[0], a[1] - b[1]
    return max(abs(q), abs(r), abs(q + r))


def worlddist(a, b):
    q, r = a[0] - b[0], a[1] - b[1]
    return math.sqrt(3 * (q * q + q * r + r * r))


def disk(center, radius):
    return {"center": hexpos(*center), "radius": radius}


def grade(point):
    return {"column": hexpos(*point[:2]), "level": point[2]}


def tree(name, height, radius, pine=False):
    """Connected solid voxel crowns and branches over a walkable open understory."""
    voxels = {}
    trunk_radius = 2 if name.endswith("heart") else (1 if height >= 80 else 0)
    crown_depth = 18 if height >= 150 else (22 if height >= 80 else max(8, height // 3))
    crown_start = height - crown_depth
    for q in range(-radius, radius + 1):
        for r in range(-radius, radius + 1):
            d = dist((q, r))
            if d <= trunk_radius:
                for level in range(crown_start + crown_depth // 2):
                    voxels[q, r, level] = ("plant/trunk", "Root" if level == 0 else "Trunk")
            if d > radius:
                continue
            # Slightly asymmetric tiers keep the silhouette authored and legible.
            for level in range(crown_start, height):
                t = (level - crown_start) / max(1, crown_depth - 1)
                width = radius * (1 - t * .8) if pine else radius * math.sqrt(max(0, 1 - (2 * t - 1) ** 2))
                if d <= max(.1, width):
                    color = "dark" if t < .3 else ("mid" if t < .72 else "light")
                    voxels.setdefault((q, r, level), (f"plant/foliage-{color}", "Foliage"))
    # A connected wooden branch tier gives every crown column a grounded path.
    branch_level = crown_start + crown_depth // 2
    for (q, r, level), (_, part) in list(voxels.items()):
        if level == branch_level and dist((q, r)) <= max(1, radius - 1):
            voxels[q, r, level] = ("plant/trunk", "Branch")
    assert len(voxels) <= 8192, (name, len(voxels))
    placements = [{"position": {"q": q, "r": r, "level": level}, "style": style,
                   "part": Raw(f"Plant({part})")} for (q, r, level), (style, part) in sorted(voxels.items())]
    blueprint = {"schema_version": 1, "id": f"plant/{name}",
                 "display_name": name.replace("forest-", "").replace("-", " ").title(),
                 "category": Raw("Plant"), "bounds": {"radius": radius, "min_level": 0, "height": height},
                 "connectivity": Raw("Grounded"), "origin": {"q": 0, "r": 0, "level": 0},
                 "placements": placements,
                 "blocker_footprint": [hexpos(q, r) for (q, r, level), (_, part) in sorted(voxels.items()) if part == "Root"],
                 "canopy_occluders": [{"q": q, "r": r, "level": level} for (q, r, level), (_, part) in sorted(voxels.items()) if part == "Foliage"]}
    # Exact source provenance and matching occupied intervals, not a visual approximation.
    intervals = []
    columns = {}
    for (q, r, level), (style, _) in sorted(voxels.items()):
        columns.setdefault((q, r), []).append((level, STYLES[style]))
    for (q, r), entries in columns.items():
        for level, material in entries:
            if intervals and intervals[-1]["offset"] == hexpos(q, r) and intervals[-1]["top"] == level and intervals[-1]["material"] == material:
                intervals[-1]["top"] += 1
            else:
                intervals.append({"offset": hexpos(q, r), "bottom": level, "top": level + 1, "material": material})
    return blueprint, intervals


def route(name, points, width=0, material="gravel"):
    return {"id": name, "points": [grade(p) for p in points], "half_width": width,
            "shoulder_width": 6, "material": material}


def point_segment_distance(point, start, end):
    def xy(p):
        return (math.sqrt(3) * (p[0] + p[1] / 2), 1.5 * p[1])
    x, y = xy(point)
    ax, ay = xy(start)
    bx, by = xy(end)
    t = max(0, min(1, ((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / ((bx - ax) ** 2 + (by - ay) ** 2)))
    return math.hypot(x - ax - t * (bx - ax), y - ay - t * (by - ay))


def documents():
    trees = {}
    for band, heights, radius in [("broadleaf", (18, 23, 28), 2), ("pine", (40, 51, 62), 3), ("ancient", (80, 97, 114), 6)]:
        for index, height in enumerate(heights):
            name = f"forest-{band}-{index + 1}"
            trees[name] = tree(name, height, radius, pine=band == "pine")
    trees["forest-heart"] = tree("forest-heart", 172, 12)
    forest_routes = [route("forest-spine", [ANCHORS[key] for key in ("party_start", "forest_outer_a", "forest_middle", "forest_deep_a")], 2, "moss"),
                     route("outer-clearing-trail", [ANCHORS["forest_outer_a"], ANCHORS["forest_outer_b"]], 2, "grass"),
                     route("ancient-trail", [ANCHORS["forest_middle"], ANCHORS["forest_deep_b"], (-125, -5, 40)], 2, "moss"),
                     route("west-bank-trail", [ANCHORS["forest_outer_a"], (-58, 0, 40)], 2, "grass")]
    routes = forest_routes + [route("bridge-west-approach", [(-54, 0, 40), ANCHORS["bridge_west"]], 3),
        route("massif-lower-ascent", [ANCHORS["bridge_east"], (56, 25, 80)]),
        route("massif-lower-shelf", [(56, 25, 80), (80, 25, 80)], material="limestone"),
        route("massif-middle-ascent", [(80, 25, 80), (106, -25, 130)]),
        route("massif-middle-shelf", [(106, -25, 130), (130, -25, 130)], material="limestone"),
        route("massif-upper-ascent", [(130, -25, 130), (162, -59, 160), (125, -77, 190)]),
        route("massif-upper-shelf", [(125, -77, 190), (101, -77, 190)], material="limestone")]
    candidates = [(q, r) for q in range(-171, 20) for r in range(-122, 163)
                  if dist((q, r)) <= 171 and q + r / 2 < -28]
    random.Random(SEED).shuffle(candidates)
    roots = {name: [] for name in trees}
    roots["forest-heart"] = [GIANT]
    selected = [(GIANT, 48, "forest-heart")]
    segments = [(tuple(p["column"].values()), tuple(n["column"].values()))
                for road in forest_routes for p, n in zip(road["points"], road["points"][1:])]
    for p in candidates:
        band = "ancient" if min(dist(p, (-132, 30)), dist(p, (-123, -9)) + 10) <= 45 else ("pine" if dist(p, (-83, 25)) <= 60 else "broadleaf")
        spacing = {"broadleaf": 10, "pine": 18, "ancient": 30}[band]
        radius = {"broadleaf": 2, "pine": 3, "ancient": 6}[band]
        if any(worlddist(p, anchor) < 15 + radius * 1.74 for anchor in ANCHORS.values() if anchor[0] < 0):
            continue
        if any(point_segment_distance(p, a, b) < 10 + radius * 1.74 for a, b in segments):
            continue
        if point_segment_distance(p, (-54, 0), (-24, 0)) < 15 + radius * 1.74:
            continue
        if any(worlddist(p, other) < max(spacing, other_spacing) for other, other_spacing, _ in selected):
            continue
        index = int(hashlib.sha256(f"{p}/{SEED}".encode()).hexdigest()[:8], 16) % 3 + 1
        name = f"forest-{band}-{index}"
        roots[name].append(p)
        selected.append((p, spacing, name))
    features = []
    outputs = {}
    for name, (blueprint, intervals) in trees.items():
        path = f"assets/art/objects/plant/{name}.ron"
        encoded = "// Authored by tools/forest_world.py; exact voxel geometry, .35 units/level.\n" + ron(blueprint) + "\n"
        outputs[ROOT / path] = encoded
        features.append({"id": name, "kind": "tree", "asset": f"plant/{name}",
                         "provenance": Raw("Some(" + ron({"source_path": path, "source_revision": BLUEPRINT_SOURCE_REV, "style_materials": mapping(STYLES)}) + ")"),
                         "mask": disk((0, 0), 187), "density": 0,
                         "roots": [hexpos(*p) for p in sorted(roots[name])], "voxels": intervals})
    colors = {"bedrock": (33, 38, 42, 255), "basalt": (76, 88, 101, 255), "soil": (98, 76, 49, 255),
              "grass": (105, 141, 72, 255), "moss": (49, 91, 57, 255), "pine-floor": (78, 98, 60, 255),
              "sand": (160, 155, 113, 255), "snow": (219, 231, 235, 255), "gravel": (126, 131, 120, 255),
              "water": (42, 115, 144, 190), "timber": (100, 67, 41, 255), "foliage": (53, 110, 66, 255), "limestone": (167, 170, 158, 255), "stone": (105, 112, 116, 255)}
    materials = [{"id": name, "solid": name != "water", "diggable": name not in ("bedrock", "water"),
                  "color": Raw(str(color))} for name, color in colors.items()]
    recipe = {"base_level": 40,
              "strata": {"bedrock": "bedrock", "rock": "basalt", "soil": "soil", "soil_depth": 4, "surface": "grass"},
              "landforms": [
                  {"id": "massif-body", "centers": [hexpos(120, -55)], "radius": 83, "plateau_radius": 12, "rise": 175, "relief": 4},
                  {"id": "massif-crown", "centers": [hexpos(132, -93), hexpos(145, -90)], "radius": 42, "plateau_radius": 3, "rise": 110, "relief": 7},
                  {"id": "dragon-lower-buttress", "centers": [hexpos(68, 25)], "radius": 34, "plateau_radius": 12, "rise": 40, "relief": 0}],
              "biomes": [
                  {"id": "pine-uplands", "mask": disk((-83, 25), 60), "priority": 2, "material": "pine-floor"},
                  {"id": "ancient-mosswood", "mask": disk((-132, 30), 45), "priority": 3, "material": "moss"},
                  {"id": "ancient-fernwood", "mask": disk((-123, -9), 35), "priority": 3, "material": "moss"},
                  {"id": "massif-stone", "mask": disk((100, -38), 83), "priority": 4, "material": "basalt"},
                  {"id": "massif-snow", "mask": disk((132, -93), 42), "priority": 5, "material": "snow"}],
              "channels": [{"id": "great-river", "points": [grade((87, -174, 34)), grade((-87, 174, 34))],
                            "half_width": 13, "depth": 12, "material": "water", "bed_material": "sand", "bank_width": 6}],
              "routes": routes,
              "bridges": [{"id": "central-crossing", "points": [grade(ANCHORS["bridge_west"]), grade(ANCHORS["bridge_east"])],
                           "half_width": 3, "thickness": 2, "material": "stone"}],
              "features": features,
              "overrides": [{"id": f"clearing-{key}", "mask": disk(value[:2], 9 if key.startswith("dragon") else 7),
                             "surface_level": Raw(f"Some({value[2]})"), "material": Raw('Some("limestone")') if key.startswith("dragon") else None}
                            for key, value in ANCHORS.items() if key.startswith(("forest_", "dragon_"))],
              "anchors": [{"id": key, "column": hexpos(*value[:2]), "level": Raw(f"Some({value[2]})"),
                           "role": Raw("Observation" if key == "ancient_tree" else "Gameplay")} for key, value in ANCHORS.items()],
              "hub": grade(ANCHORS["party_start"])}
    spec = {"version": 1, "id": "forest-massif-battle", "seed": SEED, "materials": materials,
            "recipes": mapping({"forest-massif": recipe}),
            "regions": [{"id": "forest", "recipe": "forest-massif", "origin": hexpos(0, 0), "radius": RADIUS, "rotation": 0}], "connections": []}
    outputs[SOURCE] = "// Forest–Massif Battle. Rebuild with python3 tools/forest_world.py generate.\n" + ron(spec) + "\n"
    report = {"radius": RADIUS, "columns": 1 + 3 * RADIUS * (RADIUS + 1), "level_height": LEVEL_HEIGHT,
              "anchors": {f"forest/anchor/{key}": value for key, value in ANCHORS.items()},
              "tree_count": len(selected), "trees": [{"asset": f"plant/{name}", "height_units": tree_data[0]["bounds"]["height"] * LEVEL_HEIGHT,
                    "voxel_count": len(tree_data[0]["placements"]), "roots": sorted(roots[name])} for name, tree_data in trees.items()],
              "minimum_spacing_units": {"broadleaf": 10, "pine": 18, "ancient": 30, "heart": 48}}
    outputs[CONTENT / "authoring.json"] = json.dumps(report, indent=2) + "\n"
    return outputs, report


def check():
    outputs, report = documents()
    for path, content in outputs.items():
        if not path.exists() or path.read_text() != content:
            raise ValueError(f"authored output differs: {path.relative_to(ROOT)}; run generate")
        if path.parent == ROOT / "assets/art/objects/plant":
            # Read the actual emitted bytes with an independent occupancy parser.
            positions = [tuple(map(int, row)) for row in re.findall(
                r"position:\(q:(-?\d+),r:(-?\d+),level:(\d+)\)", content)]
            occupied = set(positions)
            assert len(occupied) == len(positions) <= 8192, path.name
            reached, frontier = {(0, 0, 0)}, [(0, 0, 0)]
            steps = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0),
                     (1, -1, 0), (-1, 1, 0), (0, 0, 1), (0, 0, -1)]
            while frontier:
                p = frontier.pop()
                for step in steps:
                    neighbor = tuple(a + b for a, b in zip(p, step))
                    if neighbor in occupied and neighbor not in reached:
                        reached.add(neighbor)
                        frontier.append(neighbor)
            assert reached == occupied, f"ungrounded branch or crown: {path.name}"
            committed = subprocess.check_output(["git", "show", f"{BLUEPRINT_SOURCE_REV}:{path.relative_to(ROOT)}"], cwd=ROOT)
            assert committed.decode() == content, f"stock provenance changed: {path.name}"
    assert report["columns"] == 105469
    roots = [(tuple(p), row["asset"].split("/")[1]) for row in report["trees"] for p in row["roots"]]
    for index, (position, asset) in enumerate(roots):
        band = asset.split("-")[1]
        spacing = report["minimum_spacing_units"][band]
        for other, other_asset in roots[index + 1:]:
            required = max(spacing, report["minimum_spacing_units"][other_asset.split("-")[1]])
            assert worlddist(position, other) >= required, (position, other, required)
    print(f"Exact generated files and spacing verified: {report['tree_count']} trees; {report['columns']} terrain columns.")


def verify_package(target, package):
    """Inspect independently loaded runtime queries across the full river barrier."""
    import world
    binary, identity = world.checked_binary(target.resolve())
    points = sorted({point[:2] for point in ANCHORS.values()} |
                    {(round(-r / 2), r) for r in range(-187, 188)} | {GIANT})

    def probe(point):
        output = subprocess.check_output([str(binary), "probe", "--package", str(package.resolve()),
                                          "--at", f"{point[0]},{point[1]}"], cwd=ROOT)
        return point, json.loads(output)

    with ThreadPoolExecutor(max_workers=4) as executor:
        probes = dict(executor.map(probe, points))
    fingerprints = {value["package_fingerprint"] for value in probes.values()}
    assert len(fingerprints) == 1, "package changed during verification"
    for name, point in ANCHORS.items():
        assert any(surface["position"]["level"] == point[2] and
                   (surface["headroom"] is None or surface["headroom"] >= 2)
                   for surface in probes[point[:2]]["surfaces"]), f"anchor support absent: {name}"
    bridge_rows = []
    for r in range(-187, 188):
        value = probes[round(-r / 2), r]
        assert any(water["body_id"] == "forest/great-river" and water["top"] == 35 for water in value["liquids"]), f"river gap at {r}"
        dry = [s for s in value["surfaces"] if s["position"]["level"] >= 34]
        if dry:
            assert all(s["material"] == "stone" and s["position"]["level"] == 48 for s in dry), (r, dry)
            bridge_rows.append(r)
    assert bridge_rows == list(range(-3, 4)), bridge_rows
    heart = probes[GIANT]["root_objects"]
    assert len(heart) == 1 and heart[0]["asset"] == "plant/forest-heart"
    assert max(run["top"] for column in heart[0]["occupancy"] for run in column["runs"]) == 213
    assert world.checked_binary(target.resolve())[1] == identity
    receipt = {"world_id": "forest-massif-battle", "package_fingerprint": next(iter(fingerprints)),
               "runtime_column_probes": len(probes), "river_rows": 375,
               "unique_bridge_rows": bridge_rows, "verified_named_anchors": list(ANCHORS),
               "giant_top_exclusive": 213, "compiler": identity,
               "scope": "Exact runtime geometry and authored assets; no aesthetic, motion, or performance approval."}
    (package / "content-verification.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(f"Verified {len(probes)} runtime columns: 375-row river barrier, one seven-row bridge, every named support, exact giant occupancy.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("generate", "check", "compile", "validate", "verify-package"))
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target/v4-authoring")
    parser.add_argument("--output", type=Path, default=CONTENT / "compiled")
    args = parser.parse_args()
    if args.command == "generate":
        outputs, report = documents()
        for path, content in outputs.items():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        print(f"Authored {report['tree_count']} trees in radius-{RADIUS} forest; wrote {len(outputs)} sources.")
        return 0
    check()
    if args.command == "verify-package":
        verify_package(args.target_dir, args.output)
        return 0
    if args.command in ("compile", "validate"):
        command = [sys.executable, str(ROOT / "tools/world.py"), "--target-dir", str(args.target_dir), args.command, "--source", str(SOURCE)]
        if args.command == "compile":
            command += ["--output", str(args.output)]
        return subprocess.call(command, cwd=ROOT)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
