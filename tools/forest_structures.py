"""Pure exact props and structural assemblies for the Forest expedition.

This module never edits the global recipe or catalog. ``catalog`` returns local
blueprints plus matching V4 intervals and new opaque style definitions. Assembly
DTOs use local level zero immediately above ``support_level`` and explicitly
announce every required foundation and protected opening. The content owner must
survey/grade those supports, apply fixed rotations, and validate the final world.
No healing amounts, consumption state, encounter rosters or closed gates live here.
"""
from __future__ import annotations

from dataclasses import dataclass
import math
from typing import Callable

DIRECTIONS = ((1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1))
STONE = "expedition/stone"
EDGE = "expedition/stone-edge"
DARK = "expedition/stone-recess"
MOSS = "expedition/mossy-stone"
CYAN = "expedition/crystal"
BRIGHT = "expedition/crystal-tip"
STYLE_MATERIALS = {STONE: "stone", EDGE: "stone", DARK: "basalt", MOSS: "moss",
                   CYAN: "stone", BRIGHT: "stone"}
STYLE_SWATCHES = {STONE: "terrain/stone", EDGE: "structure/worked-stone",
                 DARK: "terrain/basalt", MOSS: "plant/foliage-mid",
                 CYAN: "crystal/cyan-body", BRIGHT: "crystal/cyan-glow"}


def distance(a, b=(0, 0)):
    q, r = a[0] - b[0], a[1] - b[1]
    return max(abs(q), abs(r), abs(q + r))


def disk(center, radius):
    q, r = center[:2]
    return {(q + dq, r + dr) for dq in range(-radius, radius + 1)
            for dr in range(max(-radius, -dq - radius), min(radius, -dq + radius) + 1)}


def rotate(point, turns):
    q, r = point[:2]
    for _ in range(turns % 6):
        q, r = -r, q + r
    return (q, r, *point[2:])


@dataclass(frozen=True, order=True)
class Cell:
    q: int
    r: int
    level: int
    style: str


@dataclass(frozen=True)
class Shape:
    """One face-connected prop, normalized to an occupied level-zero origin."""
    name: str
    cells: tuple[Cell, ...]
    # Offset of the normalized origin relative to the shape constructor's frame.
    origin_offset: tuple[int, int, int]

    @classmethod
    def create(cls, name, voxels, origin=None):
        if not voxels:
            raise ValueError("a structure requires occupied cells")
        floor = min(p[2] for p in voxels)
        origin = origin or min(p for p in voxels if p[2] == floor)
        if origin not in voxels or origin[2] != floor:
            raise ValueError("structure origin must be occupied at its lowest level")
        oq, r0, level0 = origin
        shape = cls(name, tuple(Cell(q - oq, r - r0, level - level0, style)
                                for (q, r, level), style in sorted(voxels.items())), origin)
        shape.validate()
        return shape

    @property
    def radius(self):
        return max(distance((cell.q, cell.r)) for cell in self.cells)

    @property
    def height(self):
        return max(cell.level for cell in self.cells) + 1

    @property
    def roots(self):
        """Exact lowest cells, which require the assembly's declared foundations."""
        return tuple((cell.q, cell.r) for cell in self.cells if cell.level == 0)

    def validate(self):
        if self.radius > 32 or self.height > 192 or len(self.cells) > 65536:
            raise ValueError(f"{self.name}: blueprint exceeds bounded object geometry")
        occupied = {(cell.q, cell.r, cell.level) for cell in self.cells}
        if len(occupied) != len(self.cells) or (0, 0, 0) not in occupied:
            raise ValueError(f"{self.name}: duplicate cells or missing grounded origin")
        if any(cell.level < 0 or cell.style not in STYLE_MATERIALS for cell in self.cells):
            raise ValueError(f"{self.name}: invalid cell")
        reached, pending = {(0, 0, 0)}, [(0, 0, 0)]
        while pending:
            q, r, level = pending.pop()
            neighbors = [(q + dq, r + dr, level) for dq, dr in DIRECTIONS]
            neighbors += [(q, r, level - 1), (q, r, level + 1)]
            for neighbor in neighbors:
                if neighbor in occupied and neighbor not in reached:
                    reached.add(neighbor)
                    pending.append(neighbor)
        if reached != occupied:
            raise ValueError(f"{self.name}: disconnected decorative geometry")

    def documents(self, *, raw: Callable[[str], str]):
        blueprint = {
            "schema_version": 1, "id": f"prop/expedition-{self.name}",
            "display_name": self.name.replace("-", " ").title(), "category": raw("Prop"),
            "bounds": {"radius": self.radius, "min_level": 0, "height": self.height},
            "connectivity": raw("Grounded"), "origin": {"q": 0, "r": 0, "level": 0},
            "placements": [{"position": {"q": c.q, "r": c.r, "level": c.level},
                            "style": c.style, "part": raw("Prop(Structure)")} for c in self.cells],
            "blocker_footprint": [{"q": q, "r": r} for q, r in self.roots],
            "canopy_occluders": [],
        }
        intervals = []
        for cell in self.cells:
            offset = {"q": cell.q, "r": cell.r}
            material = STYLE_MATERIALS[cell.style]
            if (intervals and intervals[-1]["offset"] == offset
                    and intervals[-1]["top"] == cell.level
                    and intervals[-1]["material"] == material):
                intervals[-1]["top"] += 1
            else:
                intervals.append({"offset": offset, "bottom": cell.level,
                                  "top": cell.level + 1, "material": material})
        return blueprint, intervals


def styles(*, raw):
    """Styles use accepted palette swatches; crystal collision stays visibly opaque."""
    result = {}
    for name, swatch in STYLE_SWATCHES.items():
        result[name] = {"display_name": name.replace("expedition/", "").replace("-", " ").title(),
                        "base_swatch": swatch, "surface_mode": raw("Opaque"), "opacity": 1.0,
                        "emission": raw("None") if name not in (CYAN, BRIGHT) else {
                            "swatch": swatch, "strength": .8 if name == CYAN else 1.2}}
    # RON's Option wrapper is supplied by the caller just like enum wrappers.
    for name in (CYAN, BRIGHT):
        value = result[name]["emission"]
        result[name]["emission"] = raw(f'Some((swatch:"{value["swatch"]}",strength:{value["strength"]}))')
    return result


def _column(voxels, q, r, bottom, top, style=STONE):
    for level in range(bottom, top):
        voxels[q, r, level] = style


def rock(kind="ridge", seed=0):
    """Distinct low slab, split pillar, elongated ridge and walk-through arch."""
    voxels = {}
    if kind == "arch":
        for r in (-3, 3):
            for q in (-1, 0, 1):
                _column(voxels, q, r, 0, 16 - abs(q), DARK if q == -1 else STONE)
        for r in range(-3, 4):
            bottom = 9 + (3 - abs(r))
            for q in (-1, 0, 1):
                _column(voxels, q, r, bottom, 17 - abs(q), STONE)
        return Shape.create("rock-arch", voxels)
    settings = {"slab": (3, 6), "pillar": (3, 22), "ridge": (5, 13)}
    if kind not in settings:
        raise ValueError(f"unknown rock silhouette {kind}")
    radius, height = settings[kind]
    for q, r in sorted(disk((0, 0), radius)):
        if kind == "ridge":
            radial = (q * q / 28 + r * r / 7 + q * r / 18)
        else:
            radial = (q * q + q * r + r * r) / (radius * radius + 1)
        if radial >= 1:
            continue
        variation = ((q * 17 + r * 31 + seed * 13) % 5) - 2
        top = max(1, round(height * math.sqrt(1 - radial)) + variation)
        if kind == "pillar" and q == 0:
            top = min(top, 3 + (r % 2))  # Deep, asymmetric cleft between tall faces.
        for level in range(top):
            material = MOSS if level == top - 1 and (q + 2 * r + seed) % 3 == 0 else STONE
            if level % 7 == 0 and (q - r + seed) % 4 == 0:
                material = DARK
            voxels[q, r, level] = material
    return Shape.create(f"rock-{kind}", voxels, (0, 0, 0))


def crystal(kind="cluster"):
    """Large angular opaque emissive crystals rooted into a shared stone plinth."""
    groups = {
        "cluster": ((0, 0, 13, 1), (2, -1, 8, 1), (-1, 2, 6, 1)),
        "needle": ((0, 0, 22, 1), (1, 1, 7, 0), (-2, 1, 11, 0)),
        "fan": ((0, 0, 12, 1), (-2, 1, 9, 1), (2, -1, 7, 1), (0, 2, 5, 0)),
    }
    if kind not in groups:
        raise ValueError(f"unknown crystal silhouette {kind}")
    voxels = {(q, r, 0): DARK for q, r in disk((0, 0), 3)}
    for q, r, height, radius in groups[kind]:
        for level in range(1, height):
            width = radius if level < height - 3 else 0
            for x, y in disk((q, r), width):
                voxels[x, y, level] = BRIGHT if level >= height - 2 else CYAN
    return Shape.create(f"crystal-{kind}", voxels, (0, 0, 0))


def fountain_frame():
    """An open C-shaped rim, worn back niche and a wide southern entry.

    Pool radius2 and entrance stay physically empty. The ornament has no emission:
    consumed-water glow can stop without a permanently shining healing marker.
    """
    voxels = {}
    for q, r in sorted(disk((0, 0), 3) - disk((0, 0), 2)):
        if r == -3:
            continue
        height = 1 if r <= -1 else 2
        _column(voxels, q, r, 0, height, MOSS if (q - r) % 4 == 0 else STONE)
    for q, r in ((0, 3), (-1, 3)):
        _column(voxels, q, r, 2, 7 - abs(q), STONE)
    voxels[0, 3, 5] = EDGE
    return Shape.create("fountain-rim", voxels)


def placement(shape, id, anchor, *, raw, rotation=0, clear_columns=()):
    """Adapt a shape at its constructor-frame anchor to the agreed placement DTO.

    ``anchor[2]`` is the supporting level below constructor local level0. Global
    integration must satisfy all ``foundation_levels`` against its final survey.
    """
    if rotation not in range(6):
        raise ValueError("fixed object rotation must be0..5")
    blueprint, intervals = shape.documents(raw=raw)
    oq, r0, z = rotate(shape.origin_offset, rotation)
    root = anchor[0] + oq, anchor[1] + r0
    support = anchor[2] + z
    occupied = []
    lowest = {}
    for cell in shape.cells:
        q, r, level = rotate((cell.q, cell.r, cell.level), rotation)
        global_cell = root[0] + q, root[1] + r, support + 1 + level
        occupied.append(global_cell)
        key = global_cell[:2]
        lowest[key] = min(lowest.get(key, global_cell[2]), global_cell[2])
    foundations = {p: level - 1 for p, level in lowest.items()
                   if level == anchor[2] + 1}
    return {"id": id, "asset": blueprint["id"], "root": root, "support_level": support,
            "desired_rotation": rotation, "blueprint": blueprint, "intervals": intervals,
            "occupied_offsets": tuple((c.q, c.r, c.level) for c in shape.cells),
            "occupied_world": tuple(occupied), "reserved_columns": tuple(sorted(lowest)),
            "clear_columns": tuple(sorted(clear_columns)), "foundation_levels": foundations,
            "overhead_clearance": 4}


def _world_piece(name, voxels, foundations, *, raw, clear_columns=()):
    """Normalize an exact world assembly without flattening its graded footings."""
    floor = min(p[2] for p in voxels)
    origin = min(p for p in voxels if p[2] == floor)
    local = {(q - origin[0], r - origin[1], z - floor): style
             for (q, r, z), style in voxels.items()}
    shape = Shape.create(name, local, (0, 0, 0))
    placed = placement(shape, name, (*origin[:2], floor - 1), raw=raw,
                       clear_columns=clear_columns)
    for (q, r), level in foundations.items():
        if (q, r, level + 1) not in voxels or (q, r, level) in voxels:
            raise ValueError(f"{name}: declared footing is not immediately below geometry")
    if origin[:2] not in foundations or foundations[origin[:2]] != floor - 1:
        raise ValueError(f"{name}: origin requires a real terrain footing")
    placed["foundation_levels"] = dict(sorted(foundations.items()))
    return placed


def bridge_assembly(surfaces, *, raw):
    """Two open portal gates and graded parapets on the nontravel bridge ledges.

    ``surfaces`` must come from the compiled bridge, including q=-27..27,
    r=-5..5. The V4 deck has half_width5 and walkway_half_width4. No geometry
    occupies the four-level movement ribbon; no support enters the river.
    """
    travel = {(q, r) for q in range(-27, 28) for r in range(-4, 5)}
    required = travel | {(q, r) for q in range(-27, 28) for r in (-5, 5)}
    if not required <= surfaces.keys():
        raise ValueError("bridge assembly requires the final deck support survey")
    pieces = []
    # Each rail fits the radius32 artifact bound and stops at the portal piers.
    segments = ((-27, -25), (-21, -1), (0, 21), (25, 27))
    for side in (-5, 5):
        for index, (start, end) in enumerate(segments):
            voxels, footings = {}, {}
            for q in range(start, end + 1):
                support = surfaces[q, side]
                footings[q, side] = support
                height = 7 if q % 4 == 0 else 4
                for z in range(support + 1, support + 1 + height):
                    voxels[q, side, z] = EDGE if z in (support + 1, support + 4) else STONE
            pieces.append(_world_piece(f"bridge-rail-{side:+}-{index}", voxels, footings,
                                       raw=raw, clear_columns=travel))
    for gate in (-23, 23):
        voxels = {}
        footings = {(q, r): surfaces[q, r] for q in range(gate - 1, gate + 2) for r in (-5, 5)}
        top = max(footings.values()) + 27
        for (q, r), support in footings.items():
            for z in range(support + 1, top):
                voxels[q, r, z] = EDGE if z % 7 == 0 else (DARK if q == gate - 1 else STONE)
        for q in range(gate - 1, gate + 2):
            for r in range(-5, 6):
                bottom = top - 9 + (5 - abs(r)) // 2
                crown = top + (3 if abs(r) in (0, 5) else 0)
                for z in range(bottom, crown):
                    voxels[q, r, z] = EDGE if z == bottom or r == 0 else STONE
        pieces.append(_world_piece(f"bridge-portal-{gate:+}", voxels, footings,
                                   raw=raw, clear_columns=travel))
    validate_assembly(pieces, surfaces=surfaces, clearance=4)
    return pieces


def arena_assembly(center, walls, gate, wall_top, surfaces, *, raw, clear_columns=()):
    """Crenellated wall crown, open arched gate and surveyed exposed buttresses.

    ``walls`` and ``gate`` are the exact terrain footprints from the global plan.
    The accepted arena keeps interior radius12 and a western gate at offset(-14,7).
    Its terrain jambs remain the load-bearing supports for the elevated arch.
    """
    center = center[:2]
    walls, gate = set(walls), set(gate)
    interior = disk(center, 12)
    clear = set(clear_columns) | interior | gate
    q0, r0 = center
    jambs = {(q0 + q, r0 + r) for q in range(-15, -12) for r in (2, 12)}
    if not jambs <= walls or any(surfaces.get(p) != wall_top for p in walls):
        raise ValueError("arena ornament requires the published wall and jamb supports")
    voxels = {}
    crown_columns = walls - jambs
    for q, r in sorted(crown_columns):
        top = wall_top + (8 if (q - q0 + 2 * (r - r0)) % 4 == 0 else 3)
        for z in range(wall_top + 1, top + 1):
            voxels[q, r, z] = EDGE if z == wall_top + 1 else (MOSS if z == top and (q + r) % 5 == 0 else STONE)
    pieces = [_world_piece("arena-wall-crown", voxels, {p: wall_top for p in crown_columns},
                           raw=raw, clear_columns=clear)]
    voxels = {}
    for q in range(q0 - 15, q0 - 12):
        for r in range(r0 + 2, r0 + 13):
            arch = min(r - (r0 + 2), (r0 + 12) - r)
            bottom = wall_top + 1 + arch
            top = wall_top + 12 + (3 if r == r0 + 7 else 0)
            for z in range(bottom, top):
                voxels[q, r, z] = EDGE if z == bottom or r == r0 + 7 else STONE
    pieces.append(_world_piece("arena-open-gate", voxels, {p: wall_top for p in jambs},
                               raw=raw, clear_columns=clear))
    for index, (dq, dr) in enumerate(DIRECTIONS):
        footing = {p for p in disk((q0 + 16 * dq, r0 + 16 * dr), 1)
                   if distance(p, center) > 15}
        if footing & clear:
            continue
        if not footing <= surfaces.keys():
            raise ValueError(f"arena buttress {index} requires a final terrain support survey")
        top = wall_top + 7
        if any(surfaces[p] >= top - 3 for p in footing):
            # A mountain can already support this corner up to its wall crest.
            # Do not manufacture a tall pedestal or cut that terrain for ornament.
            continue
        voxels = {}
        for q, r in sorted(footing):
            support = surfaces[q, r]
            for z in range(support + 1, top):
                outer = distance((q, r), center) == 17
                if outer and z > top - 9:
                    continue
                voxels[q, r, z] = EDGE if z in (support + 1, top - 10, top - 2) else (DARK if outer else STONE)
        pieces.append(_world_piece(f"arena-buttress-{index}", voxels,
                                   {p: surfaces[p] for p in footing}, raw=raw, clear_columns=clear))
    validate_assembly(pieces, surfaces=surfaces, clearance=4)
    return pieces


def validate_assembly(pieces, *, surfaces, clearance):
    """Reject overlaps, floating footings and terrain/declared-route collisions."""
    occupied = set()
    for piece in pieces:
        for p, level in piece["foundation_levels"].items():
            if surfaces.get(p) != level:
                raise ValueError(f'{piece["id"]}: footing differs from authoritative surface')
        clear = set(piece["clear_columns"])
        for q, r, z in piece["occupied_world"]:
            voxel, p = (q, r, z), (q, r)
            if voxel in occupied:
                raise ValueError(f'{piece["id"]}: overlapping assembly geometry')
            occupied.add(voxel)
            if p not in surfaces:
                raise ValueError(f'{piece["id"]}: occupied column lacks a surface survey')
            if z <= surfaces[p] or (p in clear and z <= surfaces[p] + clearance):
                raise ValueError(f'{piece["id"]}: geometry blocks terrain or required movement clearance')


def catalog(*, raw):
    shapes = [rock(kind) for kind in ("slab", "pillar", "ridge", "arch")]
    shapes += [crystal(kind) for kind in ("cluster", "needle", "fan")]
    shapes.append(fountain_frame())
    return {"shapes": {shape.name: shape for shape in shapes}, "styles": styles(raw=raw),
            "style_materials": STYLE_MATERIALS.copy()}
