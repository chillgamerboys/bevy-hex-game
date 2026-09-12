"""Pure, exact voxel tree recipes for the Forest–Massif expedition.

No files are read or written. ``generate_tree`` returns immutable occupied cells
and exact root/canopy footprints in the tree's local frame. World composition
must rotate those cells and support *every* root on the final terrain; the
nominal radius is a reservation bound, not a substitute for that validation.

``tree(name, height, radius, seed=..., raw=forest_world.Raw)`` adapts the result
to the existing forest_world.tree blueprint/half-open-interval pair. Passing
the caller's RON enum wrapper avoids a dependency on the file-writing helper.
"""
from __future__ import annotations

from dataclasses import dataclass
import math
import random
from typing import Callable

LEVEL_HEIGHT = .35
STYLES = {"plant/trunk": "timber", "plant/foliage-dark": "foliage",
          "plant/foliage-mid": "foliage", "plant/foliage-light": "foliage"}
DIRECTIONS = ((1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1))


@dataclass(frozen=True)
class TreeSpec:
    height: int
    radius: int
    pine: bool = False


# Heights are voxel levels, radii hex columns. These are shape presets, not a
# distribution or a canopy-coverage claim. The Heart is uniquely tallest/widest.
PRESETS = {
    "understory-broadleaf-1": TreeSpec(18, 3),
    "understory-broadleaf-2": TreeSpec(25, 4),
    "understory-broadleaf-3": TreeSpec(34, 5),
    "understory-pine-1": TreeSpec(23, 3, True),
    "understory-pine-2": TreeSpec(28, 4, True),
    "understory-pine-3": TreeSpec(34, 5, True),
    "landmark-1": TreeSpec(40, 5),
    "landmark-2": TreeSpec(51, 6),
    "landmark-3": TreeSpec(62, 7),
    "ancient-1": TreeSpec(80, 8),
    "ancient-2": TreeSpec(97, 9),
    "ancient-3": TreeSpec(114, 11),
    "heart": TreeSpec(172, 14),
}


@dataclass(frozen=True, order=True)
class Cell:
    q: int
    r: int
    level: int
    style: str
    part: str


@dataclass(frozen=True)
class TreeGeometry:
    height: int
    radius: int
    cells: tuple[Cell, ...]

    @property
    def roots(self) -> tuple[tuple[int, int], ...]:
        """All ground contacts; use these for the V4 grounding-support record."""
        return tuple((cell.q, cell.r) for cell in self.cells if cell.part == "Root")

    @property
    def canopy_columns(self) -> tuple[tuple[int, int], ...]:
        return tuple(sorted({(cell.q, cell.r) for cell in self.cells if cell.part == "Foliage"}))

    def documents(self, name: str, *, raw: Callable[[str], str]) -> tuple[dict, list]:
        """Return current catalog shape and exact matching V4 occupancy runs."""
        def position(cell):
            return {"q": cell.q, "r": cell.r, "level": cell.level}

        blueprint = {
            "schema_version": 1, "id": f"plant/{name}",
            "display_name": name.removeprefix("forest-").replace("-", " ").title(),
            "category": raw("Plant"),
            "bounds": {"radius": self.radius, "min_level": 0, "height": self.height},
            "connectivity": raw("Grounded"), "origin": {"q": 0, "r": 0, "level": 0},
            "placements": [{"position": position(cell), "style": cell.style,
                            "part": raw(f"Plant({cell.part})")} for cell in self.cells],
            "blocker_footprint": [{"q": q, "r": r} for q, r in self.roots],
            "canopy_occluders": [position(cell) for cell in self.cells if cell.part == "Foliage"],
        }
        intervals = []
        for cell in self.cells:
            offset = {"q": cell.q, "r": cell.r}
            material = STYLES[cell.style]
            if (intervals and intervals[-1]["offset"] == offset
                    and intervals[-1]["top"] == cell.level
                    and intervals[-1]["material"] == material):
                intervals[-1]["top"] += 1
            else:
                intervals.append({"offset": offset, "bottom": cell.level,
                                  "top": cell.level + 1, "material": material})
        return blueprint, intervals


def _distance(q, r):
    return max(abs(q), abs(r), abs(q + r))


def _disk(q, r, radius):
    for dq in range(-radius, radius + 1):
        for dr in range(max(-radius, -dq - radius), min(radius, -dq + radius) + 1):
            yield q + dq, r + dr


def _path(start, end):
    """A face-connected staircase, including joins at changing column/level."""
    previous = start
    yield previous
    steps = max(_distance(end[0] - start[0], end[1] - start[1]), abs(end[2] - start[2]))
    for index in range(1, steps + 1):
        target = tuple(round(a + (b - a) * index / steps) for a, b in zip(start, end))
        q, r, level = previous
        while level != target[2]:
            level += 1 if level < target[2] else -1
            yield q, r, level
        while (q, r) != target[:2]:
            q, r = min(((q + dq, r + dr) for dq, dr in DIRECTIONS),
                       key=lambda p: (_distance(p[0] - target[0], p[1] - target[1]), p))
            yield q, r, level
        previous = target


def generate_tree(height: int, radius: int, pine: bool = False, *, seed: int = 0) -> TreeGeometry:
    """Generate one grounded tree, with reproducible local asymmetry.

    The 6–12-unit trees retain 3.5–5-unit leaf clearance. Larger crowns span
    nearly half the tree's height instead of forming a thin horizontal plate.
    Branches and lobes share explicit support paths; no decorative floating
    cells or render-only geometry are introduced.
    """
    if not isinstance(height, int) or not 18 <= height <= 192:
        raise ValueError("tree height must be 18..192 voxel levels")
    if not isinstance(radius, int) or not 2 <= radius <= 32:
        raise ValueError("tree radius must be 2..32 hex columns")
    rng = random.Random(seed)
    voxels = {}
    crown_floor = (10 if height <= 23 else 12) if height <= 34 else round(height * .51)
    crown_depth = height - crown_floor
    base_radius = 0 if height <= 34 else (1 if height < 80 else (2 if height < 150 else 3))
    base_radius = min(base_radius, radius - 2)
    direction = rng.randrange(6)
    bend = DIRECTIONS[direction]
    # Understory trunks remain slender. Landmarks bend progressively; consecutive
    # cross-sections are explicitly joined even after the trunk tapers to one cell.
    drift = 0 if height <= 34 else (1 if height < 80 else 2)
    axes = [(round(bend[0] * drift * level / (height - 1)),
             round(bend[1] * drift * level / (height - 1))) for level in range(height)]

    def wood(q, r, level, part="Trunk"):
        if 0 <= level < height and _distance(q, r) <= radius:
            voxels[q, r, level] = ("plant/trunk", "Root" if level == 0 else part)

    def branch(start, end, thickness=0):
        for q, r, level in _path(start, end):
            for x, z in _disk(q, r, thickness):
                wood(x, z, level, "Branch")

    # The main bole is tapered, with ridges of unequal length rather than six
    # identical vertical walls. All buttresses meet the actual level-zero roots.
    for level in range(height - 2):
        q, r = axes[level]
        width = max(0, math.ceil(base_radius * (1 - level / max(1, height * .78))))
        for x, z in _disk(q, r, width):
            wood(x, z, level)
        if level:
            for x, z, y in _path((*axes[level - 1], level - 1), (q, r, level)):
                wood(x, z, y)
    if base_radius:
        for index, offset in enumerate((0, 2, 3, 5)):
            dq, dr = DIRECTIONS[(direction + offset) % 6]
            length = min(radius - 1, base_radius + 1 + (index % 2))
            root_height = max(3, round(height * (.08 + .015 * index)))
            for level in range(root_height):
                reach = max(0, math.ceil(length * (1 - level / root_height)))
                for step in range(reach + 1):
                    wood(dq * step, dr * step, level)

    # One continuous crown plus differently sized, offset bough crowns. Filling
    # each column through its ellipsoid gives a connected volume at voxel scale.
    middle = (crown_floor + height - 1) / 2
    center = axes[round(middle)]
    lobes = [(center[0], center[1], middle, radius * .77, (crown_depth - 1) / 2)]
    for index, offset in enumerate((0, 2, 4)):
        dq, dr = DIRECTIONS[(direction + offset) % 6]
        reach = max(1, round(radius * (.32 + rng.random() * .10)))
        q, r = center[0] + dq * reach, center[1] + dr * reach
        # Keep every lobe center inside the declared footprint, including bent
        # ancient trunks. Clipping affects only its outer leaves, never support.
        while _distance(q, r) > radius - 1:
            q, r = min(((q + x, r + z) for x, z in DIRECTIONS),
                       key=lambda p: (_distance(*p), p))
        level = round(crown_floor + crown_depth * (.36 + .12 * index))
        lobe_radius = radius * (.46 + rng.random() * .12)
        lobe_depth = min(level - crown_floor, height - 1 - level) * .88
        lobes.append((q, r, level, lobe_radius, max(1, lobe_depth)))
        origin_level = max(crown_floor, level - round(crown_depth * .22))
        branch((*axes[origin_level], origin_level), (q, r, level), 1 if height >= 80 else 0)

    # Pines are tall with overlapping needle tiers; broadleaf trees retain the
    # rounder bough volumes. Both remain asymmetric and supported by the bole.
    if pine:
        lobes = [(q, r, y, width * (.90 if index else .82), depth)
                 for index, (q, r, y, width, depth) in enumerate(lobes)]
    for cq, cr, cy, horizontal, vertical in lobes:
        for q, r in _disk(0, 0, radius):
            dq, dr = q - cq, r - cr
            radial = (dq * dq + dq * dr + dr * dr) / (horizontal * horizontal)
            if radial > 1:
                continue
            extent = vertical * math.sqrt(max(0, 1 - radial))
            bottom = max(crown_floor, math.ceil(cy - extent - 1e-9))
            top = min(height - 1, math.floor(cy + extent + 1e-9))
            for level in range(bottom, top + 1):
                t = (level - crown_floor) / crown_depth
                color = "dark" if t < .32 else ("mid" if t < .82 else "light")
                voxels.setdefault((q, r, level), (f"plant/foliage-{color}", "Foliage"))

    # The main crown center follows the same trunk axis at its midpoint. Stitch
    # its top to the bent trunk so even the last narrow leaf cap has support.
    branch((*axes[height - 3], height - 3), (*center, height - 2))
    if len(voxels) > 65536:
        raise ValueError(f"tree exceeds 65536 occupied cells: {len(voxels)}")
    return TreeGeometry(height, radius, tuple(Cell(*point, style, part)
                       for point, (style, part) in sorted(voxels.items())))


def tree(name, height, radius, pine=False, *, seed=0, raw):
    """Drop-in document adapter; pass ``raw=forest_world.Raw`` when integrating."""
    return generate_tree(height, radius, pine, seed=seed).documents(name, raw=raw)
