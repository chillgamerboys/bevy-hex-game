"""Exact deterministic vegetation placement over the public V4 survey export.

Terrain authority stays with worldc. This authoring pass proposes explicit fixed
rotations, complete trunk foundations and collision-free occupied intervals;
compilation and the production site validator must accept the final composition.
"""
from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass
import heapq
import random

from forest_expedition import SEED, distance, disk, line, ribbon
from forest_trees import PRESETS, STYLES, generate_tree


def rotate(p, turns):
    q, r = p[:2]
    for _ in range(turns):
        q, r = -r, q + r
    return q, r


def world_distance(a, b):
    q, r = a[0] - b[0], a[1] - b[1]
    return (3 * (q*q + q*r + r*r)) ** .5


@dataclass(frozen=True)
class TreeShape:
    name: str
    rotation: int
    runs: tuple
    roots: tuple
    canopy: frozenset


class TreeLibrary:
    def __init__(self, raw):
        self.documents = {}
        self.shapes = {}
        self.geometry = {}
        for index, (name, spec) in enumerate(PRESETS.items()):
            geometry = generate_tree(spec.height, spec.radius, spec.pine, seed=index * 17)
            asset = f"forest-expedition-{name}"
            blueprint, runs = geometry.documents(asset, raw=raw)
            self.documents[name] = blueprint, runs
            self.geometry[name] = geometry
            for turn in range(6):
                self.shapes[name, turn] = TreeShape(name, turn, tuple(
                    (*rotate((run["offset"]["q"], run["offset"]["r"]), turn),
                     run["bottom"], run["top"], run["material"]) for run in runs),
                    tuple(sorted(rotate(p, turn) for p in geometry.roots)),
                    frozenset(rotate(p, turn) for p in geometry.canopy_columns))


class PlacementWorld:
    def __init__(self, survey, metadata):
        if survey["version"] != 1 or survey["world_id"] != metadata["world_id"]:
            raise ValueError("survey and authoring world identity mismatch")
        self.fingerprint = survey["manifest_fingerprint"]
        solid = {m["id"] for m in survey["materials"] if m["solid"]}
        self.terrain = {(c["position"]["q"], c["position"]["r"]): c["runs"] for c in survey["columns"]}
        self.surface = {p: max(r["top"] - 1 for r in runs if r["material"] in solid)
                        for p, runs in self.terrain.items()}
        self.wet = set()
        for water in survey["liquids"]:
            p = water["column"]["q"], water["column"]["r"]
            if water["bottom"] <= self.surface[p] + 1 < water["top"]:
                self.wet.add(p)
        self.occupied = defaultdict(list)
        for obj in survey["objects"]:
            for column in obj["occupancy"]:
                p = column["position"]["q"], column["position"]["r"]
                self.occupied[p].extend((r["bottom"], r["top"], obj["id"]) for r in column["runs"])
        self.protected = {}
        for route in metadata["routes"].values():
            for q, r, level in route["ribbon"]:
                self.protected[q, r] = max(self.protected.get((q, r), 0), level + 1 + route["clearance_levels"])
        for site in metadata["encounters"].values():
            for q, r, level in site["surfaces"]:
                self.protected[q, r] = max(self.protected.get((q, r), 0), level + 1 + site.get("clearance_levels", 4))
        self.foundations = {}
        self.placements = []
        self.canopy = set()
        self.root_exclusion = set(map(tuple, metadata.get("arena", {}).get("walls", ())))
        self.root_exclusion.update(map(tuple, metadata.get("arena", {}).get("gate", ())))
        if "bridge" in metadata:
            controls = metadata["bridge"]["controls"]
            for start, end in zip(controls, controls[1:]):
                self.root_exclusion.update(ribbon(line(start, end), 5))
        for pool in metadata.get("fountains", {}).values():
            self.root_exclusion.update(disk(pool["center"], 6))

    def reserve_structure(self, dto):
        """Reserve an independently authored exact prop before vegetation."""
        foundations = dto["foundation_levels"]
        for p, level in foundations.items():
            if p not in self.surface or p in self.wet:
                raise ValueError(f'{dto["id"]}: foundation leaves known dry terrain')
            if p in self.protected and level != self.surface[p]:
                raise ValueError(f'{dto["id"]}: foundation changes a protected support')
            if any(low <= level for low, _, _ in self.occupied.get(p, ())):
                raise ValueError(f'{dto["id"]}: foundation buries prior exact occupancy')
        for q, r, level in dto["occupied_world"]:
            p = q, r
            surface = foundations.get(p, self.surface.get(p))
            if surface is None or level <= surface or level < self.protected.get(p, -1):
                raise ValueError(f'{dto["id"]}: structure intersects terrain or protected clearance')
            if any(low <= level < high for low, high, _ in self.occupied.get(p, ())):
                raise ValueError(f'{dto["id"]}: structure intersects prior exact occupancy')
        for q, r, level in dto["occupied_world"]:
            self.occupied[q, r].append((level, level + 1, dto["id"]))
        self.root_exclusion.update(tuple(p) for p in dto["reserved_columns"])
        self.root_exclusion.update(tuple(p) for p in dto["clear_columns"])
        for p, level in dto["foundation_levels"].items():
            if self.surface[p] != level:
                self.foundations[p] = level
                self.surface[p] = level

    def try_tree(self, id, root, shape, *, flatten=False, clearing=None, commit=True):
        if root in self.wet or root in self.protected or root in self.root_exclusion:
            return None
        ground = self.surface[root]
        roots = {(root[0] + q, root[1] + r) for q, r in shape.roots}
        if any(p not in self.surface or p in self.wet or p in self.protected or p in self.root_exclusion for p in roots):
            return None
        if flatten:
            heights = sorted(self.surface[p] for p in roots)
            ground = heights[len(heights)//2]
        elif any(self.surface[p] != ground for p in roots):
            return None
        proposed = {}
        world_runs = []
        for q, r, bottom, top, material in shape.runs:
            p = root[0] + q, root[1] + r
            if p not in self.surface:
                return None
            bottom, top = ground + 1 + bottom, ground + 1 + top
            surface = ground if p in roots else self.surface[p]
            if bottom <= surface or bottom < self.protected.get(p, -1):
                return None
            if bottom == surface + 1 and p not in roots:
                return None  # Do not accidentally root leaves or floating boughs into a hill.
            if any(low < top and bottom < high for low, high, _ in self.occupied.get(p, ())):
                return None
            world_runs.append((*p, bottom, top, material))
        for p in roots:
            if any(low <= ground for low, _, _ in self.occupied.get(p, ())):
                return None
            if self.surface[p] != ground:
                proposed[p] = ground
        canopy = {(root[0] + q, root[1] + r) for q, r in shape.canopy}
        entry = {"id": id, "preset": shape.name, "asset": f"plant/forest-expedition-{shape.name}",
                 "root": root, "support_level": ground, "rotation": shape.rotation,
                 "ground_contacts": sorted((*p, ground) for p in roots),
                 "occupied_runs": world_runs, "canopy_columns": sorted(canopy)}
        if clearing is not None:
            entry["clearing_radius"] = clearing
        if commit:
            for q, r, low, high, _ in world_runs:
                self.occupied[q, r].append((low, high, id))
            for p, height in proposed.items():
                self.surface[p] = height
                self.foundations[p] = height
            self.placements.append(entry)
            self.canopy.update(canopy)
        return entry


def place_forest(survey, metadata, library, structures=(), *, understory_count=700, mountain_count=70):
    """Propose full-cardinality forest plus sparse foothill/slope vegetation."""
    world = PlacementWorld(survey, metadata)
    for structure in structures:
        world.reserve_structure(structure)
    rng = random.Random(SEED)
    heart = metadata["forest"]["heart"]
    if not world.try_tree("heart", tuple(heart["center"][:2]), library.shapes["heart", 2],
                          flatten=True, clearing=heart["clearing_radius"]):
        raise ValueError("the authored Heart cannot be grounded without a protected overlap")
    for index, site in enumerate(metadata["forest"]["landmarks"]):
        preset = f'{site["tier"]}-{index % 3 + 1}'
        root = tuple(site["center"])
        turns = list(range(6))
        rng.shuffle(turns)
        if not any(world.try_tree(site["id"], root, library.shapes[preset, turn], flatten=True,
                                  clearing=site["clearing_radius"]) for turn in turns):
            raise ValueError(f'{site["id"]} at {root} has no exact supported orientation')

    forest_mask = set(map(tuple, metadata["forest"]["columns"]))
    candidate_roots = list(map(tuple, metadata["forest"]["understory_root_columns"]))
    heap = []
    # A lazy coverage queue fills different parts of the forest before filling
    # existing canopy shadows. All candidates still pass exact volume collision.
    for index, root in enumerate(candidate_roots):
        if root in world.root_exclusion or root in world.wet:
            continue
        band = "pine" if distance(root, heart["center"]) < 100 else "broadleaf"
        variant = rng.choices((1, 2, 3), weights=(1, 2, 5))[0]
        preset = f"understory-{band}-{variant}"
        turn = rng.randrange(6)
        shape = library.shapes[preset, turn]
        canopy = {(root[0]+q, root[1]+r) for q, r in shape.canopy} & forest_mask
        score = len(canopy - world.canopy)
        heapq.heappush(heap, (-score, rng.random(), root, preset, turn))
    rejected = defaultdict(int)
    small_roots = set()
    while heap and len(small_roots) < understory_count:
        old_score, tie, root, preset, turn = heapq.heappop(heap)
        if root in world.root_exclusion:
            continue
        shape = library.shapes[preset, turn]
        canopy = {(root[0]+q, root[1]+r) for q, r in shape.canopy} & forest_mask
        score = len(canopy - world.canopy)
        if score < -old_score:
            heapq.heappush(heap, (-score, tie, root, preset, turn))
            continue
        entry = world.try_tree(f"understory-{len(small_roots)+1:03}", root, shape)
        if entry:
            small_roots.add(root)
            world.root_exclusion.update(disk(root, 3))
        else:
            rejected[preset] += 1
            variant = int(preset[-1])
            if variant > 1:
                smaller = preset[:-1] + str(variant - 1)
                canopy = {(root[0]+q, root[1]+r) for q, r in library.shapes[smaller, turn].canopy} & forest_mask
                heapq.heappush(heap, (-len(canopy - world.canopy), tie, root, smaller, turn))
    if len(small_roots) != understory_count:
        raise ValueError(f"only {len(small_roots)}/{understory_count} understory trees fit; rejected {dict(rejected)}")

    mountain_candidates = [p for p, level in world.surface.items()
                           if p not in forest_mask and p not in world.wet and p not in world.protected
                           and distance(p) <= 174 and p[0] + p[1]/2 > 32 and 40 <= level <= 200
                           and p not in world.root_exclusion]
    rng.shuffle(mountain_candidates)
    placed_mountain = 0
    for root in mountain_candidates:
        if root in world.root_exclusion:
            continue
        preset = f"understory-pine-{rng.randrange(1,4)}"
        if world.try_tree(f"mountain-tree-{placed_mountain+1:03}", root, library.shapes[preset, rng.randrange(6)]):
            placed_mountain += 1
            world.root_exclusion.update(disk(root, 8))
        if placed_mountain == mountain_count:
            break
    if placed_mountain != mountain_count:
        raise ValueError(f"only {placed_mountain}/{mountain_count} mountain trees fit")
    covered = world.canopy & forest_mask
    report = {"world_id": metadata["world_id"], "terrain_manifest_fingerprint": world.fingerprint,
              "tree_count": len(world.placements), "landmarks": 36, "heart": 1,
              "understory": len(small_roots), "mountain_trees": placed_mountain,
              "forest_columns": len(forest_mask), "canopy_columns": len(covered),
              "canopy_fraction": len(covered)/len(forest_mask), "minimum_canopy_fraction": .5,
              "target_canopy_fraction": .6, "placements": world.placements,
              "foundations": [(*p, level) for p, level in sorted(world.foundations.items())]}
    if len(covered) < .5 * len(forest_mask):
        raise ValueError(f'forest canopy is only {report["canopy_fraction"]:.1%}; the 50% floor is not met')
    return world, report
