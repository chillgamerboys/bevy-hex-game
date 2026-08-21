# Changing the game without writing code

Most of how the game looks and feels is controlled by a handful of text files under
`assets/`. You can edit them in any text editor. You do not need to know Rust, and
you do not need to recompile the game.

| File | Controls |
|---|---|
| `world.ron` | Map size, terrain preset and shape, how tall a voxel is |
| `substances.ron` | What the world is made of — including water and metal — plus exact art-palette references and gameplay properties |
| `terrain_damage.ron` | Boolean element × substance damage admission; every missing pair resists |
| `art/palette.ron` | Canonical authored colours for terrain, liquids, structures, units, and authored objects |
| `art/voxel_styles.ron` | Palette-backed opaque, cutout, translucent, additive, and emissive object surfaces |
| `art/object_catalog.ron` + `art/objects/*.ron` | The validated authored plant, effect, and prop catalog; normally edited through `cargo editor` |
| `elements.ron` | The six-basic wheel, opposition, and the six direct pair plus six direct triple fusion recipes |
| `spells.ron` | Spells: what each requires, how it is cast, and what it does |
| `camera.ron` | Initial Map, third-person, and first-person frames, lens, pan speed, zoom, and tilt |
| `combat.ron` | Engagement thresholds, movement budget, height bonus, what a strike costs, and the open design questions as policy knobs that reject unbuilt variants with a reason |
| `perception.ron` | Active Bright/Dim/Dark sight radii |
| `lighting.ron` | Sun brightness, colour and angle, ambient light, the sky gradient and its hex clouds |
| `player.ron` | Player piece size and movement speed |
| `scenarios.ron` | Internal world + lighting + encounter launch definitions used by Campaign, Sandbox, saves, retry, review, and tests |
| `sandbox_maps.ron` | Stable player-facing Sandbox map IDs and scenario/seed contract, plus Party/Enemy hidden actor-staging centers and radii retained for compatibility |
| `encounters/*.ron` | Who is on the map: rosters by archetype, and where each unit starts |
| `lattices.ron` | Who each of them *is*: the gems, fusions and spells an archetype is made of |
| `menu.ron` | How the menu screens look |
| `display.ron` | Authored Vsync / frame-rate default; the local Settings screen owns the persisted player choice |

## Seeing your changes

Run the game with:

```sh
cargo dev
```

Then edit a file and **save it**. The game notices immediately — you never need to
close and reopen it.

How quickly you *see* the change depends on which file:

| File | When it takes effect |
|---|---|
| `camera.ron` | Follow, eye-height, and lens values straight away; entry pitch/radius on the next mode entry; initial Map frame on the next rebuild |
| `display.ron` | Straight away until the player saves a local presentation choice in Settings |
| `world.ron` | On the next world rebuild |
| `substances.ron` | On the next world rebuild |
| `terrain_damage.ron` | On the next world rebuild (re-parsed and cross-file validated on save) |
| `art/palette.ron` | Authored objects after one coherent art-graph reload; substance and unit colours on the next world rebuild |
| `art/voxel_styles.ron`, `art/object_catalog.ron`, `art/objects/*.ron` | Rendered object instances after the complete palette → style → object graph validates; a broken revision keeps the last valid graph |
| `elements.ron` | On the next world rebuild (re-parsed and validated on save) |
| `spells.ron` | On the next world rebuild (re-parsed and validated on save) |
| `perception.ron` | Straight away; observation and knowledge use the new profile on the next frame |
| `lighting.ron` | Straight away, all of it — sun, ambient, sky and clouds |
| `player.ron` | Speed on the next movement started; scale on the next rebuild |
| `scenarios.ron` | On the next world rebuild |
| `encounters/*.ron` | On the next world rebuild |
| `lattices.ron` | On the next world rebuild (re-parsed and re-resolved on save) |
| `menu.ron` | Straight away |

**To rebuild the world**, press `BACKSPACE` to return to the owning Campaign or
Sandbox route, then launch again. It takes under a second and picks up your edit.

The split exists because some values are read continuously while the game runs and
others are read once, when the map and pieces are created. Nothing is lost either
way — the rebuild is quick.

Elements, substances, terrain-damage admission, spells, and lattices form one semantic
revision at the Loading boundary. A bad cross-file edit may leave the last valid
resolved catalogs available for inspection, but Loading does not treat their presence
as readiness. It waits until canonical source fingerprints prove that every raw file,
direct catalog, `TerrainDamageTable`, `ContentIndex`, and `LatticeLibrary` describes
the same accepted revision. Repairing or reverting the edit publishes a new
`AcceptedContentRevision` and allows the rebuild; leaving an invalid edit settled for
several frames never admits a mixed revision.

(`cargo run --release` runs faster but will not reload files at all. Use `cargo dev`
while tuning, and `--release` when you just want to play.)

## Deterministic review captures

Renderer captures are compiled only with the default-off `map-review` feature. Normal
development and release binaries ignore every `HEX_REVIEW_*` variable. A complete
capture names one scenario and one PNG:

```sh
HEX_REVIEW_SCENARIO="Caves" \
HEX_REVIEW_CAPTURE=".context/caves/default.png" \
cargo run -p hex_game --release --features map-review
```

The optional review overrides are:

| Variable | Effect |
|---|---|
| `HEX_REVIEW_SEED` | Replaces the configured seed of a seeded scenario |
| `HEX_REVIEW_VIEW` | Authors the deterministic Map pose as `default`, `rotated` (+120°), `counter-rotated` (-120°), `rear` (180°), or `top-down`; character cameras retain its horizontal azimuth while applying their own pitch, except that `top-down` has no horizontal component and falls back to the original Map heading |
| `HEX_REVIEW_CAMERA` | Uses the `map`, close `character`, or `first-person` camera |
| `HEX_REVIEW_TIME` | Sets a cyclic-lighting hour from `0.0` up to, but not including, `24.0` |
| `HEX_REVIEW_LIQUID_PHASE` | Freezes liquid animation at a finite phase in seconds, wrapped over its visual cycle; captures default to `0.0` |
| `HEX_REVIEW_FOCUS_ANCHOR` | Moves the selected actor to one exact generated map anchor before framing |
| `HEX_REVIEW_CUTAWAY` | `full` hides the complete roof of the selected interior; ordinary gameplay never removes it |
| `HEX_REVIEW_ILLUMINATION` | `overlay` draws exact authored-interior gameplay illumination tiers: charcoal Dark, blue Dim, and cyan-green Bright |

`HEX_REVIEW_VIEW`, `HEX_REVIEW_CAMERA`, `HEX_REVIEW_FOCUS_ANCHOR`,
`HEX_REVIEW_CUTAWAY`, and `HEX_REVIEW_ILLUMINATION` require `HEX_REVIEW_CAPTURE`.
The focus override resolves the anchor's full `TilePos`, not just its horizontal
coordinate, so it can target an underground floor beneath a surface. It also applies
the selected actor's normal solidity and headroom rules. An unknown anchor or one the
actor cannot stand on fails the review process instead of silently capturing the
wrong place. The full cutaway still requires the selected actor to occupy an exact
interior surface and affects only that interior; ordinary gameplay retains the opaque
roof and resolves tight views through camera collision instead. The illumination
overlay reads `ResolvedIllumination`. Its diagnostic caps are physically separated
and render-ordered above tactical fog caps, so both translucent projections compose
deterministically; it never changes gameplay light, physical lights, faction
knowledge, ordinary fog behavior, or picking.

For example, this exposes the complete generated cave network for a top-down overview:

```sh
HEX_REVIEW_SCENARIO="Caves" \
HEX_REVIEW_CAPTURE=".context/caves/full-overview.png" \
HEX_REVIEW_FOCUS_ANCHOR="conflict_center" \
HEX_REVIEW_VIEW="top-down" \
HEX_REVIEW_CUTAWAY="full" \
HEX_REVIEW_ILLUMINATION="overlay" \
cargo run -p hex_game --release --features map-review
```

Use the unoccupied `conflict_center` anchor for a neutral cave overview.
`deep_chamber` is also the configured enemy position, so relocating the player there
can start combat before capture. Omit `HEX_REVIEW_ILLUMINATION` for the ordinary
untinted cave overview.

## The format

These are RON files. Three rules cover almost everything:

**Text after `//` is a comment.** It is ignored by the game, so you can leave notes
for yourself.

**Every value needs a comma after it**, including the last one in a list. This is
the single most common mistake.

**Decimal numbers need a decimal point.** Write `1.0`, not `1`. Whole numbers like
`grid_radius: 20` are the exception — those are counts, and are written plainly.

Substances name an exact entry in the canonical art palette:

```ron
"grass": (
    swatch: Some("terrain/grass"),
    solid: true,
    diggable: true,
),
```

Palette colours use named channels from `0.0` to `1.0`:

```ron
"terrain/grass": (
    display_name: "Grass Terrain",
    color: (red: 0.35, green: 0.62, blue: 0.30),
    tags: ["ground", "terrain", "world"],
),
```

`0.0` is no light in that channel and `1.0` is full intensity. Use `cargo editor`
for normal palette edits so validation and near-colour warnings run before saving.

## If something goes wrong

See [troubleshooting.md](troubleshooting.md) — the single list of symptoms,
including the ones that produce no log output at all.

## Things worth trying

**Tune the showcase map.** The default `Showcase((...))` preset is deterministic.
Its important controls are all grouped in `world.ron`:

```ron
valley_level: 15,
gentle_max_level: 19,
gentle_terrace_width: 2,
```

The river waypoints, bridge lanes, summit, and ordered switchback are cube
coordinates. Every coordinate must sum to zero and stay inside `grid_radius`.
The file is validated as one map: the river must reach both boundaries, the bridge
must clear the water, and the switchback must be contiguous and long enough to climb
one level at a time. An invalid save is reported in the terminal and the previous
valid settings remain active.

**Reproduce a frozen procedural map.** Generator version 1 separates the broad
landform, its materials, and its tactical structure:

```ron
terrain: Procedural((
    generator_version: 1,
    landform: Hills((
        valley_level: 15,
        max_relief: 8,
        hills_per_bank: 3,
    )),
    environment: TemperateGrassland,
    tactical: Crossing((
        barrier_half_width: 1,
        bed_level: 12,
        hazard_bottom: 13,
        hazard_top: 14,
        bridge_level: 16,
    )),
)),
```

This combination generates grassland hills around an edge-to-edge river, with a
bridge, an alternate crossing, and scenario anchors. Its reproducible seed belongs
to the scenario, as described in **Configuring a scenario** below. Version 1 is
frozen: keep `generator_version: 1` to reproduce an existing seed with the original
algorithm and fields.

**Use current procedural terrain.** Generator version 3 places each typed recipe
inside a patch. A `Single` layout owns the complete map footprint:

```ron
terrain: Procedural((
    generator_version: 3,
    layout: Single((
        environment: TemperateGrassland,
        recipe: Hills((
            valley_level: 15,
            max_relief: 8,
            hills_per_bank: 3,
        )),
        overlays: [],
        mask: WholeWorld,
        edges: (
            east: WorldBoundary,
            south_east: WorldBoundary,
            south_west: WorldBoundary,
            west: WorldBoundary,
            north_west: WorldBoundary,
            north_east: WorldBoundary,
        ),
    )),
)),
```

Native V3 Hills derives its edge-to-edge three-wide hazard, direct two-wide metal
bridge, separated two-wide alternate crossing, bed, fill bounds, and bridge level
from `valley_level`; those invariants are intentionally not editable. Temperate,
Frozen, and Volcanic Hills use this recipe in the shipped scenario library. V2 keeps
its frozen external shape and implementation only as a development reference while
the V3 review corpus is approved.

`SkyIslands` finalizes the same Hills ground before sampling any independent
`sky.*` stream, then adds three primary islands, one or two satellites, and a two-wide
upper bridge network:

```ron
recipe: SkyIslands((
    ground: (
        valley_level: 15,
        max_relief: 8,
        hills_per_bank: 3,
    ),
    min_clearance: 22,
    upper_coverage_percent: 24,
)),
```

The upper layer covers 15–25% of map columns, remains an exact special-movement
region, and cannot replace the finalized ground, its anchors, or its protected
crossing approaches. The selected scenario uses 22 clear levels and 24% coverage for
a distinct high city layer, varied walkable terraces, and tapered stone undersides.
The original eight-level-clearance construction remains deterministic and loadable
only as a V3 comparison oracle. The recipe supports `TemperateGrassland` and
`Frozen`.

`Mountains` builds a broad, sharp frozen massif with explicit ordinary-walker routes
instead of projecting the whole map into gentle slopes:

```ron
terrain: Procedural((
    generator_version: 2,
    environment: Frozen,
    recipe: Mountains((
        base_level: 15,
        relief: 24,
        peak_count: 7,
    )),
)),
```

It publishes a two-wide elevated saddle and a separated low-valley bypass, with no
river or crossing structures. Inaccessible summit components are exact
special-movement regions; naturally walkable peaks remain ordinary terrain. Valid
settings use relief `14..=24` and three through seven peaks. The low-relief
compatibility path remains deterministic. The selected scenario uses the upper bounds
to form a branched massif across most of the map and distribute peaks across multiple
heights. Its player-facing edge includes a substantial, three-level walker-connected
foothill apron, so ordinary access extends into the range instead of ending at the two
through routes.

`Caves` carves one ordinary-walkable underground network beneath a varied rocky
surface:

```ron
terrain: Procedural((
    generator_version: 2,
    environment: Rocky,
    recipe: Caves((
        surface_level: 17,
        cave_floor_level: 6,
        chamber_count: 12,
    )),
)),
```

It publishes a two-wide descending entrance, a rooted network of six to twelve
chambers, at least three clear levels in critical corridors, at least four in
chambers, and at least three solid roof levels. The selected scenario uses twelve
chambers, three `+0/+2/+4` floor terraces, loop corridors, varied chamber heights,
and the most developed rocky surface. Six-through-eight-room settings retain their
deterministic compatibility geometry.
Exact floor, entrance, and cutaway-roof memberships remain keyed by `TilePos`, so the
underground floor cannot be confused with the surface above it. The hostile remains
inside the deepest chamber on the floor with the greatest minimum horizontal
separation from the complete ramp and entry connector. A live scenario regression
checks that placement against the loaded combat policy so combat through rock cannot
interrupt entry. Recipe and environment combinations are validated together:
Mountains requires `Frozen`, Caves requires `Rocky`, and invalid combinations leave
the previous valid hot-reloaded settings active.

`CrystalAscent` authors one complete vertical landmark in a radius-40 `Single` patch:

```ron
terrain: Procedural((
    generator_version: 3,
    layout: Single((
        environment: TemperateGrassland,
        recipe: CrystalAscent((
            base_level: 6,
            rise_levels: 144,
        )),
        overlays: [],
        mask: WholeWorld,
        edges: (
            east: WorldBoundary,
            south_east: WorldBoundary,
            south_west: WorldBoundary,
            west: WorldBoundary,
            north_west: WorldBoundary,
            north_east: WorldBoundary,
        ),
    )),
)),
```

`rise_levels` accepts 100 through 200. `base_level`, the rise, the crown, and its
reserved headroom must all fit at or below V3's level-256 ceiling. That ceiling does
not change the retained V1/V2 validation limits. The landmark's three stair circuits,
four-wide routes, entrance, chamber, shaft, oculus, lights, and summit clearing are
recipe invariants rather than additional tuning fields. Crystal Ascent requires
`TemperateGrassland`, rejects overlays, and varies only crystal presentation and
summit trees with the scenario seed. The standalone world still owns the complete
radius-40 footprint. Macro may dispatch the recipe only through Crystal Mountain's
specialized landmark mask: it must contain the complete radius-32 authored site and
may retain the protruding fringe inherited from the landmark's central seven atomic
cells. In composite mode that outside-site fringe is authored at the level-150 crown
rather than clipped or filled at the standalone base level. An arbitrary Macro
instance still cannot stretch, clip, or relocate the landmark.

**Compose Crystal Mountain with a cross-biome tunnel.** The selectable configuration
uses a radius-three `Macro` layout over a radius-77 map. Four logical instances
own all 37 atomic cells: the central seven-cell Crystal Ascent landmark, five
consecutive radius-two Forest cells at level 149–151, the remaining radius-two inner
mountain wall, and the complete radius-three outer ridge. Crystal Ascent keeps
`base_level: 6`, `rise_levels: 144`, and `rotation_turns: 0`.

Macro's defaulted extensions separate surface connectivity from a feature that crosses
several biomes:

- `walker_connections` declares ordinary surface ports without making them part of the
  legacy `critical_route`; Crystal Mountain aligns the exact authored four-wide
  Crystal summit terminal and inward trail with the Forest approach at level 150;
- `spanning_features` declares the ordered instance route and exact dimensions of the
  level-6, four-wide tunnel from the world boundary to Crystal Ascent's lower aperture;
  and
- `anchor_aliases` publishes stable world-level names for an instance anchor.

An empty `critical_route` is valid only when exactly one spanning feature supplies the
canonical route. Existing Macro files omit all three defaulted collections and keep
their former settings fingerprint byte-for-byte. The tunnel's instance order is
`outer-mountain`, `inner-mountain`, then `crystal-ascent`; its six clear levels and
three solid roof levels are validated independently from the surface walker port. See
the shipped `config/worlds/procedural-crystal-mountain.ron` for the complete cell
roster. The new fields have this exact shape:

```ron
walker_connections: [
    (first_instance: "crystal-ascent", second_instance: "summit-forest", width: 4, level: 150),
],
spanning_features: [
    Tunnel((
        name: "crystal_mountain.tunnel",
        canonical_route: true,
        instance_route: ["outer-mountain", "inner-mountain", "crystal-ascent"],
        boundary_terminal: (instance: "outer-mountain", side: West),
        destination_anchor: (instance: "crystal-ascent", anchor: "crystal_ascent.lower_entry"),
        floor_level: 6,
        width: 4,
        clearance: 6,
        roof_thickness: 3,
    )),
],
anchor_aliases: [
    (alias: "crystal_mountain.ascent_threshold", instance: "crystal-ascent", anchor: "crystal_ascent.lower_entry"),
],
critical_route: [],
```

Walker widths are `2..=4`. The initial tunnel implementation is deliberately fixed at
width `4`, floor level `6`, and clearance `6`, with a roof thickness of at least `3`.
A tunnel route contains at least two unique, pairwise-adjacent non-aquatic instances.
Its boundary terminal belongs to the first route instance, its destination to the
last, and the complete vertical reservation must remain inside V3's level-256 ceiling.

The configuration resolves into exact geometry before ordinary fragments are
decorated. The authored destination is constructed first; the global planner reserves
the tunnel ribbon, widened mouth, and crystal alcoves in every crossed patch so local
liquids and vegetation cannot occupy them. Composition then merges the fragments,
carves the passage once, retains the authored summit approach, publishes review
anchors, and runs the normal final classification and validation pass. At each crossed
biome boundary, only the declared four subsurface lane pairs remain ordinary;
incidental level-6 seam contacts beside the ribbon are closed under a world-owned
special-movement region.

The eight-wide open boundary apron is exterior. Its first roofed four-wide row and the
four-wide summit threshold are the eight registered entrances of one world-owned
interior containing the tunnel and Crystal Ascent. Crystal Ascent's lower aperture is
therefore an internal handoff, not a second light domain or exterior entrance.

**Use V3 Arid terrain.** `Arid` admits four recipes and no overlays. The first three
shipped `Single` configurations are radius 12; Oasis is the central patch of the
Ring19 map below. Their authored settings are:

```ron
recipe: DesertTransition((
    base_level: 15,
    max_relief: 3,
    transition_width: 8,
    dry_coverage_percent: 55,
)),

recipe: DesertPlain((base_level: 15, max_relief: 2)),

recipe: Dunes((
    base_level: 15,
    ridge_height: 6,
    ridge_spacing: 12,
    ridge_count: 5,
)),

recipe: Oasis((
    base_level: 15,
    pool_radius: 5,
    palm_count: 12,
    grass_ring_width: 3,
)),
```

Desert Transition accepts relief `1..=4`, transition width `5..=12`, and dry
coverage `40..=70` percent. Desert Plain accepts relief `1..=4`. Dunes accepts ridge
height `3..=8`, spacing `8..=16`, and count `3..=7`. Oasis accepts pool radius
`3..=6`, exact palm count `8..=18`, and green-ring width `2..=4`; its pool and ring
must leave four dry approach columns. Each recipe requires `base_level >= 5`, must
remain under the V3 ceiling, and rejects unknown fields.

The material bands, requested ridge dimensions, local oasis footprint, and exact
date-palm count are deterministic setting consequences. Named streams vary rolling
relief, bounded dune warp, and palm selection/rotation without changing those
contracts. Oasis uses a connected local Still-water body with no seam connection.

**Compose Desert Oasis Rings.** The selectable radius-55 Ring19 file sets
`profile: DesertOasis`: slot 0 is Oasis; slots 1–6 are the six rotated inner Dunes;
slots 7–18 alternate taller Dunes and Desert Plain. Seam defaults remain level
`15..=19` with preferred level 17, two width-two walker ports, and depth-three
approaches. Every seam is Dry, while both `liquid_connections` and
`boundary_outlets` are empty because the central pool is intentionally local. Only
this profile admits that empty hydrology contract; the original defaulted
`TwoRings` profile and fingerprint remain unchanged.

The four selectable scenario names are **Desert Transition**, **Desert Plain**,
**Dunes**, and **Desert Oasis Rings**. Each ships with configured seed `1592598566`;
Sandbox regeneration changes only the pending resolved seed.

**Use V3 Waterfall terrain.** The first shipped V3 recipe uses an explicit
single-patch layout. Its edge-to-edge three-wide liquid topology, eleven-level fall,
extended plunge basin, upper two-wide metal bridge, meandering escarpment, mid-height
shelves, short two-wide dry bypass, and longer alternate terrace are structural
recipe invariants rather than extra tuning fields:

```ron
terrain: Procedural((
    generator_version: 3,
    layout: Single((
        environment: TemperateGrassland,
        recipe: Waterfall(()),
        overlays: [],
        mask: WholeWorld,
        edges: (
            east: WorldBoundary,
            south_east: WorldBoundary,
            south_west: WorldBoundary,
            west: WorldBoundary,
            north_west: WorldBoundary,
            north_east: WorldBoundary,
        ),
    )),
)),
```

As with earlier procedural scenarios, the reproducible seed belongs in
`scenarios.ron`, not in the world recipe. V3 rejects unsupported
recipe/environment pairs and incomplete edge contracts while deserializing.

**Compare gameplay sight ranges.** `perception.ron` contains three fixed review
profiles without coupling them to renderer brightness:

```ron
active: Expansive,
```

`Expansive` uses Bright/Dim/Dark radii of `36/12/1`, `Focused` uses `24/8/1`, and
`Tight` uses `18/6/1`. Every profile must keep Bright at least Dim, Dim at least Dark,
and Dark exactly one. Each radius feeds the exact upper-dome predicate: horizontal
distance and upward voxel distance combine by `h² + u² <= radius²`, while downward
vertical distance is ignored. Invalid edits and unknown fields are reported while the
last valid settings remain active.

**Use the retained Perlin preset.** Perlin remains a separate, optional terrain
preset; it is not one of the versioned procedural recipes:

```ron
terrain: Perlin((
    // None chooses a new map each launch; Some(number) is reproducible.
    seed: Some(20260725),
    steps: [
        (x_freq: 0.035, y_freq: 0.05, magnitude: 3.0),
    ],
)),
```

Higher frequency means bumpier terrain; magnitude is how much height an octave
contributes. Add another entry with a higher frequency and smaller magnitude for
fine detail:

```ron
steps: [
    (x_freq: 0.035, y_freq: 0.05, magnitude: 3.0),
    (x_freq: 0.20,  y_freq: 0.20, magnitude: 0.6),
],
```

**A substance entry.** First create or deliberately reuse a swatch in the canonical
palette. The shipped Sand material is a complete example of the corresponding entry
in the map-owned `substances.ron`:

```ron
"sand": (
    swatch: Some("terrain/sand"),
    solid: true,
    diggable: true,
    conjurable: false,
    toughness: Some(2),
),
```

Saving the file registers it with the game. It will not appear in generated terrain
until the generation code selects it. A missing palette reference rejects the
cross-file update and retains the previous valid runtime table. The rejected source
pair is retained too, so repairing only the other file retries the complete on-disk
candidate instead of accepting a stale fallback. `air` is never drawn and therefore
uses `swatch: None`; every rendered substance requires `Some(...)`.

Substance ids are a frozen compatibility contract, not RON entry order. `air` is 0;
the original shipped vocabulary remains `basalt=1`, `bedrock=2`, `dirt=3`, `grass=4`,
`gravel=5`, `ice=6`, `lava=7`, `metal=8`, `snow=9`, `stone=10`, `water=11`, and
`worked_stone=12`. The additive tail reserves `limestone=13`, `slate=14`, `timber=15`,
and `terracotta=16` for Outpost, followed by `sand=17`. A reserved but unauthored slot
is inert and does not resolve through `id`, `name`, or `get`. This lets Mountain Range
or Outpost land first without moving the other feature's ids or existing id-derived
map fingerprints. Before adding another name, extend this stable registry and its
exact-id regression; the loader rejects an arbitrary name added only to the RON file
so it cannot shift accepted ids.

`toughness` is maximum voxel health. The initial schema accepts only `Some(1)`,
`Some(2)`, `Some(4)`, `Some(8)`, or `None`; `None` means the material does not
participate in terrain damage. To make an added substance damageable, add explicit
stable-name pairs to `terrain_damage.ron`:

```ron
(element: "Earth", substance: "sand"),
```

The matrix is an allow-list, not a multiplier table. A listed pair permits the
spell's exact authored power, while an absent pair resists. Duplicate pairs, unknown
elements or substances, and references to `toughness: None` are rejected without
replacing the last valid resolved table.

**A bigger procedural map.** `grid_radius: 12` gives 469 columns, `20` gives 1261,
and `40` gives 4921. Procedural recipes accept radii from 12 through 40 and regenerate
their boundary features and exact anchors to fit. The Showcase river endpoints must
move to the new boundary when its radius changes, so changing only `grid_radius` is
intentionally rejected. Perlin has no authored boundary features and can be resized
directly.

**Chunkier terrain.** `level_height` in `world.ron` is how tall one voxel is. The
default `0.4` is quite flat; raising it towards `1.0` gives blockier terrain that reads
better once you are digging into it.

**Time of day.** Lighting files have one of two profiles:

- `Static` keeps the flat values in the file. Omitting `profile` also means `Static`,
  so older files and `lighting/overcast.ron` keep their exact appearance.
- `Cycle((...))` resolves an ordered set of clear-sky keyframes. The shipped
  `lighting.ron` has midnight, pre-dawn, sunrise, noon, golden-hour, sunset, and night
  entries and starts at noon.

Run `cargo dev`, enter gameplay, and edit the reflected `TimeOfDay.hours` resource in
the inspector to scrub the clear cycle. The clock is session-only and does not advance
on its own. A scenario can set `starting_time_hours: Some(18.5)`; an authored hour is
rejected when that scenario selects static lighting.

Cycle directions use explicit azimuth and elevation in degrees. Colours interpolate in
linear RGB, exposure interpolates in EV100, and the last keyframe wraps smoothly back
to midnight. Exactly one directional key light casts shadows: the sun supplies it
during the day and the moon supplies a deliberately brighter-than-physical key at
night.

The legacy flat fields remain the complete static appearance. In a cycle profile,
cloud coverage and shape still come from the flat fields; light, colour, fog, and
exposure come from the keyframes. The shipped noon keyframe deliberately duplicates
the flat compatibility values, but changing a flat `sun_illuminance` does not rewrite
that keyframe. In a static profile, `sun_rotation` is still an XYZ Euler angle in
radians (a full circle is about 6.28). Lower `sun_illuminance` towards `1000.0` for
overcast; `100000.0` is direct noon sun. `sun_color` tints it.

> **For static lighting, change the second rotation number first.** The three values
> are Euler angles rather than "height, compass, roll", and whether the sun ends up
> above or below the horizon depends on the first two together. An invalid combination
> can put the sun underneath the map, lighting nothing. Cycle profiles avoid that
> ambiguity by authoring azimuth and elevation directly.

The sun is worth treating carefully. The terrain has no texture, so **shadows are the
only thing giving it shape** — changes that weaken them, or that light the shadowed
faces back up, tend to read as "flat" rather than "soft".

**Directional fill and haze.** Both live in `lighting.ron`. Their top-level values
form the static/noon compatibility baseline and ship switched off:

```ron
sky_light_intensity: 0.0,   // a soft fill from the sky itself
fog_density:         0.0,   // distance haze
```

Cycle keyframes can enable either value away from noon. `sky_light_intensity` adds
light that varies with which way a surface faces — blue from overhead, a warm bounce
from the ground — so it colours shadows instead of flooding everything equally. It is
the honest way to soften shadows. Try `100`–`200` first; several hundred already starts
washing out the shading that gives terrain its shape. `ground_color` sets the bounce
colour underneath it.

`fog_density` hazes distant terrain, tinted `fog_color` and glowing `fog_sun_color`
towards the key light. At this camera distance it costs more colour than it buys
atmosphere, which is why it is off at the static/noon baseline; cycle twilight uses
only a restrained amount. `0.002` is about as much as is worth trying.

**The sky, celestial bodies, and clouds.** The sky is drawn procedurally: a vertical
colour gradient, visible sun and full-moon discs for cycle profiles, localized halos,
and hexagonal clouds. Every part lives in `lighting.ron`. Like everything else in that
file it updates **straight away**: edit, save, and the sky changes while you watch.

The sun and moon use the same resolved directions as the directional light, with the
moon exactly opposite the sun. Their angular diameters and halo widths are set once on
the cycle profile. Discs are drawn only above the true horizon. A restrained,
azimuth-local lower-dome glow lets sunset remain visible from the downward map camera
without tinting the entire surround orange. Clouds composite last and can obscure both
discs and halos.

The gradient runs between two colours:

```ron
sky_color:    (0.55, 0.80, 0.95),  // at the horizon
zenith_color: (0.25, 0.50, 0.85),  // straight up
```

`sky_color` doubles as the fallback colour behind everything, so keep it a believable
sky tone. Set the two close together for a flat, even sky; push them apart for a
deeper, more dramatic one.

The sky dome is drawn **only during gameplay**. The menus have no camera you can move,
so a view of a sky you cannot look around is a picture that changes for no reason —
and once each scenario brought its own sky, the menu would have changed colour
depending on which map you last played. The menus have their own file instead; see
**The menus** below.

The clouds sit on a hexagonal grid — a nod to the map — but are drawn as soft puffs
that merge where they touch. Six values shape them:

```ron
cloud_color:     (0.97, 0.98, 1.0),  // usually near-white
cloud_coverage:  0.18,  // 0.0 is a clear sky, higher is cloudier
hex_cloud_scale: 16.0,  // bigger = smaller and more numerous clouds
cloud_softness:  0.1,   // extra edge softening on top of the automatic anti-aliasing
cloud_roundness: 0.5,   // 0.0 is hard hexagons, 1.0 is round; the middle hints at hex
cloud_noise:     0.3,   // 0.0 is clean-edged, ~0.5 is wispy and broken up
```

- **`cloud_coverage`** is how much of the sky is clouded. Because neighbouring clouds
  now merge, a little goes a long way — past ~0.35 the sky fills in quickly.
- **`hex_cloud_scale`** sets cloud *size*, inverted: turn it **up** for many small
  clouds, **down** for a few big ones.
- **`cloud_roundness`** morphs each cloud from a hard hexagon (`0.0`) to a round puff
  (`1.0`). The `0.5` default keeps a gentle hex hint.
- **`cloud_noise`** breaks the edges up with fine detail — low is clean and smooth,
  high is wispy.
- **`cloud_softness`** adds extra blur on top; the edges are already kept crisp
  automatically, so this is only for a softer, hazier look.

For a clear blue sky, set `cloud_coverage: 0.0`. For an overcast one, raise coverage
and push `cloud_noise` up. The colours take `(red, green, blue)` from `0.0` to `1.0`,
the same as everywhere else.

## The menus

`menu.ron` is the splash, Main Menu, Campaign, Sandbox, Tools, and loading surfaces.
One value so far:

```ron
background: (0.10, 0.11, 0.14),
```

It is a **flat, opaque panel**, not a view of anything. That is deliberate: the menus
sit outside the world, and the world behind them is different for every scenario. A
dark, desaturated colour works best, because the buttons are drawn as low-alpha white
and need something dim to sit on.

Its own file rather than a corner of `lighting.ron`, so the next thing a menu needs
has somewhere obvious to go.

## Configuring a scenario

A scenario names a world, a sky, and an encounter. The library's `default_game`
names the canonical Campaign started from an empty slot. Players do not browse these
definitions directly: Campaign, Sandbox, saves, Retry Exact, deterministic review,
and tests resolve them internally through the same `ScenarioToLoad` loading contract.
`category` remains inert compatibility metadata while legacy
`resume.ron` data can still refer to this `scenarios.ron`; do not use it to create a
shipping route.

Immutable human creator-format templates live in `creation_presets.ron` and appear as
duplicable Creator choices. Exact automation-only Creator records live instead in
`crates/hex_game/testdata/deterministic_creator_library.ron`; that file and the stable
deterministic fixture manifest are consumed only by the default-off `test-support`
boundary and never enter the shipping asset loader. Local saved creations belong to
the per-user data directory's `creations.ron`, not the shipped asset tree.

`sandbox_maps.ron` owns the deployable Sandbox map list. Its `schema_version` is
checked on load. Every distinct supported shipped environment appears at least once;
separate recipe and layout experiences may share an environment. Each entry has a
stable ID, display name, tactical description and tags, renderer-generated preview
asset, scenario, optional generation seed, and one hidden actor-staging region per side. A staging center is
either `Fixed((x, y, z))` for authored terrain or `Anchor("name")` for a generated
exact surface, with a bounded path-cost `radius`. These fields are retained as stable
compatibility metadata so Loading can stage the frozen roster before guided
deployment. They do not restrict the player's exact placement choices: each current
Party or Enemy character may be placed on any canonical legal, unoccupied surface.

Selecting a generated map in Sandbox creates a pending `resolved_seed`. Regenerate
changes only that pending value and displays the exact replacement. **Use Map**
commits it; Back discards it; restarting the process returns to the configured seed.
Authored maps have no regeneration action. None of these operations edits either RON
file.

```ron
(
    default_game: "Party Trial",
    scenarios: [
        // entries...
    ],
)
```

```ron
(
    name: "Rolling Hills",
    category: Map,
    blurb: "Open procedural ground under heavy cloud.",
    world: "config/worlds/rolling-hills.ron",
    lighting: "config/lighting/overcast.ron",
    encounter: "config/encounters/open-ground.ron",
),
```

Leave `lighting` out and the scenario gets `config/lighting.ron`, which is what most
should do. A lighting file is a complete copy of that file's contents — start by
copying it and changing what you want.

An `encounter` is required, and several scenarios may share one — every generated map
ships pointing at the same anchored skirmish. Generated terrain also owns its
reproducible seed here:

```ron
(
    name: "Procedural Hills",
    category: Map,
    blurb: "Seeded temperate hills split by a river.",
    world: "config/worlds/procedural-hills.ron",
    generation_seed: Some(1592598566),
    encounter: "config/encounters/anchored-skirmish.ron",
),
```

It is called `lighting` rather than `sky` because it also sets **the sun's angle and
colour**, so it decides which way the shadows fall. That is most of what makes a
scenario feel different; a changed sky with unchanged shadows just looks like a filter.

**Two things to know before writing one**, both learned the hard way:

`sun_rotation` is **not** "height, compass, roll". It is an XYZ Euler triple that wraps
past 2π, and whether the sun ends up above or below the horizon depends on the first
two numbers *together*. A sun below the horizon lights nothing: the map renders as a
black mass with no error anywhere. Change the second number and leave the first alone,
and a test will tell you if you got it wrong.

`zenith_color` is **almost never on screen**. The gameplay camera looks down at the
map, so you only see the lower band of the sky dome — `sky_color` is effectively the
whole background. That is why the shipped alternative is weather rather than a sunset:
a warm horizon colour fills the screen with terracotta and reads as clay, not evening.

`world`, `lighting` and `encounter` are all paths, and none of them is checked by the
compiler. A typo fails `cargo test` rather than at the loading screen, but only because
tests open every file the scenarios name — keep it that way. `default_game` must also
resolve to one uniquely named entry; it is validated independently from lane contents.

## Writing an encounter

An encounter is a **roster**: one entry per unit, each naming an archetype and one
placement. It replaced a scaffold that could only say "one player here, one enemy
there".

```ron
(
    name: "Bridge Ambush",
    rosters: [
        (
            faction: Player,
            placement: Formation(center: Anchor("party_start"), spread: 2),
            units: [
                (archetype: "hedge-mage"),
                (archetype: "raider"),
            ],
        ),
        (
            faction: Hostile,
            placement: Formation(center: Anchor("hostile_start"), spread: 2),
            units: [
                (archetype: "wolf"),
                (archetype: "wolf"),
                // One unit that has to be somewhere exact, while the rest of its side
                // comes in as a formation.
                (archetype: "raider", placement: Some(Anchor("bridge"))),
            ],
        ),
    ],
)
```

`faction` is `Player` or `Hostile`, and a faction may appear in **more than one
roster** — that is how two hostile groups hold different ground, with no second
mechanism for it.

`archetype` is looked up in [`lattices.ron`](#writing-a-lattice), which is where the
unit's gems, fusions and spells come from — and, since a lattice *is* the stat block,
most of what the unit is. It still resolves to no mesh and no body size; every unit is
drawn the same and walks the same.

### The three placements

| Placement | Holds | Use it for |
|---|---|---|
| `Fixed((x: 0, y: 4, z: -4))` | one unit | authored maps, whose landmarks never move. Takes the **lowest** surface at that coordinate the unit fits on — the ground, not a bridge over it. Cube coordinates must sum to zero |
| `Anchor("party_start")` | one unit | generated maps. One exact surface, level included, published by the generator after it validates the map — so rerolling a seed moves the ground and the anchor with it |
| `Formation(center: …, spread: N)` | a group | a party. `center` is a `Fixed` coordinate or an `Anchor`; the first unit stands on it and each one after takes the next free surface, closest first |

A formation's `spread` is in **walking steps**, not hexes. The candidate surfaces come
out of the same flood fill movement uses, so a formation will not spread across a
chasm, onto a ledge the body cannot climb, or under a ceiling it does not fit beneath.
A **named spawn zone** is written exactly this way: the anchor names it, `spread`
bounds it, and the fill order is deterministic — walking distance, then position — so
the same encounter on the same seed always deals the same surfaces.

Two units may not share a `Fixed` or an `Anchor`: those hold exactly one unit each, and
the file is rejected when it parses with a message telling you to use a formation.
Exact placements are resolved *before* formations, so the sentry who must stand on the
bridge keeps his surface and the crowd flows around him.

### When a unit cannot be placed

**Every rostered unit is placed, or loading returns to the Main Menu with a visible
reason** — naming the side, the archetype, and what was wrong. An anchor that the
active map does not publish, an authored coordinate with nothing standable under it, and
a formation with more units than room all fail that way. None of them is a unit that
quietly does not appear, which is the class of bug this repo is worst at noticing.

A scenario is also checked against the world it names: procedural terrain requires every
placement to resolve through an anchor, and authored terrain requires every placement to
be fixed. That pairing is checked once both files have loaded, and a mismatch returns
to the Main Menu rather than starting a fight with a unit missing. The in-memory
Sandbox draft remains available when its route is opened again.

## Writing a lattice

`lattices.ron` is where enemies are designed. **An enemy's lattice is its entire stat
block** — there is no separate stats system, no hit points, and no difficulty slider. A
wolf is four hexes and a bite. A raider is eight around a metal shield. A hedge-mage is
thirteen with direct Lightning and Divination fusions plus Scrying Eye. Difficulty is
the size and complexity of the drawing.

An archetype named here is what `archetype: "raider"` in an encounter roster looks up.

Four kinds of cell:

| Cell | What it does |
|---|---|
| `Gem("Fire")` | Holds mana of one element, and powers **adjacent** spells |
| `Fusion("Lightning")` | Combines *its own* adjacent gems into a higher-order element, which adjacent spells may then draw on |
| `Spell("Ember")` | Castable, if its adjacent cells can pay the requirements |
| `Blank` | Part of the lattice, holds nothing — still takes a hit |

**Adjacency is the entire power mechanism.** There is no action at a distance inside a
lattice: a spell draws only from the six cells touching it. Laying one out *is* the design
problem, and a spell whose neighbours cannot pay is simply offline — it is not an error,
it is a lattice that cannot cast that spell.

That is also the mistake worth knowing about because it is not a load error. A spell
cell one hex too far from the gems meant to fund it parses, loads and spawns perfectly;
the cast panel reports it as blocked when that unit is controlled. **A test catches it
earlier for shipped content** — every shipped archetype must be able to cast everything
it inscribes on a fresh lattice — so a misplaced cell fails `cargo test` rather than
waiting for a playtest.

```ron
"raider": (
    cells: [
        (at: (q: 0, r: 0), kind: Spell("Metal Shield")),
        (at: (q: 1, r: 0), kind: Gem("Metal")),
        (at: (q: 1, r: -1), kind: Gem("Metal")),
        // …five more
    ],
    attunement: {"Metal": 3, "Earth": 2},
    channelling: {"Metal": 2, "Earth": 1},
),
```

Coordinates are axial `(q, r)` and carry no meaning beyond adjacency — the drawing
matters, not where it sits. Every authored archetype must form one contiguous
hex arrangement. A disconnected island is rejected while resolving
`LatticeLibrary`, and the error names the offending archetype; it cannot survive as a
valid-but-unreachable part of a character. Cell order in the RON file does not affect
that check or the semantic content fingerprint.

`attunement` is how much mana one gem of that element holds when full. `channelling` is
how much a channel action puts back per turn. An element with no attunement entry resolves
to **zero**, which makes a gem of it inert — a legal way to say "this thing does not
cast", which is exactly what the wolf does. Every shipped spell costs 1 mana per required
gem, so an attunement of 3 is three casts before that gem needs channelling back up.

Which cells to break is the interesting part of a fight. The two Metal gems touching a
raider's shield are what fund it, so taking either down drops the shield *and* burns the
mana locked in it. A hedge-mage's fusion holds nothing itself, so its feeder gems are
worth more than their own hexes: kill one and everything downstream dies with it.

A name that does not resolve — an element not in `elements.ron`, a spell not in
`spells.ron`, or a `Fusion` naming something with no recipe — fails to load with the
archetype and the name in the message.

## Elements and spells

Two files define the magic system as content. The lattice renderer, cast panel, command
applier, combat log, and knowledge projection all read the resolved catalogs. They also
load, validate, and cross-check together, so a dangling name cannot ship silently.

### `elements.ron`

The six-basic **wheel** and the twelve direct **fusion recipes** form the canonical
18-element catalog.

```ron
(
    wheel: ["Air", "Fire", "Metal", "Earth", "Life", "Water"],
    fusions: {
        "Lightning": [
            (element: "Air", mana: 1),
            (element: "Fire", mana: 1),
        ],
        "Destruction": [
            (element: "Air", mana: 1),
            (element: "Fire", mana: 1),
            (element: "Metal", mana: 1),
        ],
        // The shipped file contains the other ten recipes listed below.
    },
)
```

| Kind | Result | Direct basic inputs |
|---|---|---|
| Pair | Lightning | Air + Fire |
| Pair | Volcano | Fire + Metal |
| Pair | Crystal | Metal + Earth |
| Pair | Transmutation | Earth + Life |
| Pair | Divination | Life + Water |
| Pair | Illusion | Water + Air |
| Triple | Destruction | Air + Fire + Metal |
| Triple | Artifice | Fire + Metal + Earth |
| Triple | Necromancy | Metal + Earth + Life |
| Triple | Wild | Earth + Life + Water |
| Triple | Storm | Life + Water + Air |
| Triple | Space | Water + Air + Fire |

- **`wheel`** lists the basic elements. Opposition is their position on the wheel:
  each element opposes the one halfway round — with six, that is three apart, giving
  Air/Earth, Fire/Life, and Metal/Water. Reorder the wheel and you change *which
  elements oppose which*. The catalog exposes this relationship, but no current
  combat rule grants an opposition-triggered bonus.
- **`fusions`** are higher-order elements. Each names its output and the inputs it
  draws (an element and how much mana). Every canonical recipe above consumes two or
  three **distinct basic elements directly**; none of the triples consumes a pair
  output. The generic loader still rejects unknown inputs, basic/output name
  collisions, and cyclic recipe graphs.

An element's internal **id is assigned from its name in alphabetical order**, *not*
from where it sits in the file or on the wheel. Reordering entries therefore does not
rewrite an unchanged catalog, while adding or removing a stable name deliberately
changes the accepted content revision.

**Life is not a rename or migration target for Light.** They are different stable
names with different meanings. The elemental-grid migration removes Light references
from packaged content and authors Life references where intended; it does not rewrite
local Creator drafts or campaign data. Campaign records bound to the old content
revision remain preserved and incompatible, including legacy digest-bound resumes
that predate explicit content revisions. Creator drafts remain preserved with an
unresolved-Light diagnostic until the user edits them. Neither path silently changes
a build.

### `spells.ron`

Each spell by name:

```ron
(
    spells: {
        "Ember": (
            requirements: [(element: "Fire", mana: 1)],
            casting: Evocation,
            mana: Fixed,
            co_castable: false,
            targeting: (range: 3, reach: Ranged, shape: Single, trajectory: Direct),
            effects: [
                DisableHexes(count: 1, targeted: false),
                Burn(turns: 2),
            ],
        ),
    },
)
```

- **`requirements`** are the adjacent gems the spell draws on — an element and its
  mana. The *number* of requirements is the spell's **tier** (at most six, a full
  ring). Ember needs one Fire gem; Fireball needs six.
- **`casting`** is `Evocation` (spends the mana outright) or `Enchantment(defense: N)`
  (ties mana up while it lasts; `defense` is how much it subtracts from incoming
  disables — `0` for a non-defensive enchantment).
- **`mana`** is `Fixed` (all-or-nothing) or `Variable` (scales with the mana given).
- **`co_castable`** allows casting alongside another spell. A spell that is both
  `Variable` and `co_castable` is what the design calls a **ritual** — you do not write
  "ritual"; it follows from the two flags.
- **`targeting`** is `range` (how far away an ordinary target may be, in hexes),
  `reach` (`Ranged` or `Touch`), `shape` (what the spell covers once it gets there),
  and `trajectory`: `Direct`, `Arc(rise: N)`, or `None`. `Direct` tests a straight
  exact-voxel segment, `Arc` rises `N` integer levels above the higher endpoint, and
  `None` deliberately ignores material obstruction. Direct and arc authority fails
  closed if exact terrain occupancy is absent; preview, target cycling, and AI use
  only currently Observed material facts. Both authored `range` and `Arc.rise` have a
  technical maximum of 16.

  `Touch` is not range one. It targets the caster or an Observed occupied unit across
  one exact bidirectional `Footing` edge, receives no high-ground bonus, and must use
  `range: 0`, `shape: Single`, and `trajectory: None`. Omitting `reach` in older authored
  content or Creator saves decodes as `Ranged`; new serialization writes it explicitly,
  and changing it changes the accepted spell-content revision.
  Old creator saves using `needs_los: true/false` migrate on read to `Direct`/`None`;
  newly written content always uses `trajectory`, and defining both is rejected.

  `range` and a shape's own extents are different numbers. Fireball's `range: 4` is
  how far it is thrown; its `Sphere(radius: 2)` is how big the ball is.

  | Shape | What it covers |
  |---|---|
  | `SelfCast` | the caster's own voxel; `range` must be `0` |
  | `Single` | one target voxel |
  | `Sphere(radius: N)` | everything within `N` of the target — `N` hexes out *and* `N` levels up or down |
  | `Column(height: N)` | the target voxel and the `N - 1` voxels above it; a conjured wall is `2` |
  | `Line(length: N, width: W)` | out from the caster; `W` is a half-thickness, so `0` is a single file. Rounded ends mean it reaches `N + W` |
  | `Cone(length: N, spread: S)` | widening out from the caster; `S` is 60° sectors *each side*, so `0` is a ray, `1` the usual cone, `3` a full disc |
  | `Path(offsets: [...])` | a hand-authored voxel list, `(coord: (q: 1, r: 0), level: 2)` each, rotated to the facing |

  **Vertical and horizontal count equally.** A radius-3 sphere reaches three hexes out
  and three levels up or down, so it looks slightly squashed on screen. That is
  deliberate: gameplay is not allowed to know how tall a voxel is drawn, so there is no
  other honest answer.

  `Line`, `Cone` and `Path` point somewhere, so a cast using one has to name a facing;
  the other four look the same in every direction. `Line` and `Cone` never include the
  caster's own voxel.

  Extents are capped at **16**, a `Path` at **64 voxels**, and cone spread at **3** (a
  full disc). That is a guard rail, not balance: a resolved volume is a real list of
  voxels, so a radius typed with an extra digit is tens of millions of them. A file that
  names one fails to load with the spell and the field in the message. `Column.height`,
  `Line.length` and `Cone.length` also have a *minimum* of 1, since a shape with no
  extent is a spell that does nothing.

  **`Line.width` is capped at 1**, lower than the rest, and that one is provisional. The
  spine starts a hex ahead of the caster, so a width-2 line's near end rounds back past
  them and covers every neighbour — including the hex directly behind. A line that burns
  the ally behind you is not what the word means, and choosing between subtracting that
  rear arc and renaming the shape is a design call the ticket that first wants a wide
  line should make. Width 1 stops exactly at the caster's own voxel, which is already
  excluded, so content is held there meanwhile.
- **`effects`** is a **fixed list** of what a spell can do — you cannot invent new ones
  without a programmer, which is deliberate:

  `DisableHexes`, `Burn`, `RestoreHexes`, `ModifyIncomingDisables`, `Reveal`,
  `Illuminate`, `SetTerrain`, `SpawnWall`, `Displace`.

  `SetTerrain` and `SpawnWall` name a substance from `substances.ron`.

  This is the schema vocabulary, not a promise that every primitive has a delivered
  runtime consumer. `ModifyIncomingDisables` remains reserved for a future one-shot
  ward lifecycle and is absent from shipped spell content. Canonical `Heal` is a
  tier-one Life Evocation with fixed mana, `co_castable: false`,
  `targeting: (range: 0, reach: Touch, shape: Single, trajectory: None)` and only
  `RestoreHexes(count: 1)`. `Renewal` remains ranged and carries only
  `RestoreHexes(count: 2)`.

  The E0 content gives `Scrying Eye` one Divination requirement and retains
  the supported single-target `Reveal` behavior. Its later off-sight live-feed
  lifecycle is not implied by this content migration. `Illuminate` remains in the
  schema but still fails closed at runtime; future illumination spells belong to
  Illusion. The former `Daylight` entry is removed rather than shipped under the new
  element catalog, and the separate initial Illusion showcase, Invisibility, is later
  work.

### When a reference is wrong

A spell that requires an element `elements.ron` does not define, or an effect that
names a substance `substances.ron` does not have, is a **dangling reference**. On load
the game logs exactly which one and keeps the last content that was valid — the same
way a broken `lighting.ron` keeps the last good sky. A test also opens every shipped
file and fails if any reference dangles, so the shipped game never carries a broken one.

## Local Settings are not authored config

The in-game Settings screen atomically writes schema-v3 `preferences.ron` beside
`campaigns.ron`, `creations.ron`, and any legacy local files; it never edits
`assets/config/display.ron`. The file stores display/window presentation, semantic UI
scale, volume values, the four ordinary HUD visibility preferences, and only keyboard
bindings that differ from their canonical defaults. Transient master HUD suppression,
Compact temporary surfaces, inspected-unit identity, and the current Main View are
session state and are never persisted.

Reading schema v1 preserves its display and audio values while supplying Auto UI
scale, the minimalist default HUD combination, and no keyboard overrides. Reading
schema v2 also preserves its UI-scale choice while supplying the same new HUD and
binding defaults. A missing file uses the authored display default plus built-in UI,
volume, HUD, and input defaults. A corrupt, invalid, or unknown-version file is
reported on the Settings screen and rejected as a whole rather than partially
applied.

Campaign persistence contains exactly three indexed records and is replaced
atomically. On first load without `campaigns.ron`, a valid legacy `resume.ron` is
copied into slot 1 with zero prior active-play time. Invalid legacy data is preserved
with its refusal, and `resume.ron` is never overwritten or deleted. An existing
obsolete `combat-reports.ron` is likewise never read, modified, or deleted.

Set `HEX_GAME_DATA_DIR` to an explicit directory when a test or review needs isolated
local state. Otherwise the files live under:

- macOS: `~/Library/Application Support/Hex Game/`
- Windows: `%APPDATA%/Hex Game/`
- Linux: `$XDG_DATA_HOME/hex-game/`, or `~/.local/share/hex-game/`

The preferences file remains version-bound pre-alpha state, not a durable
configuration format.

## Frame presentation on macOS

`display.ron` provides the authored presentation default until Settings saves a local
choice. On macOS neither path has a visible effect: the system composites every
window and syncs it to the display regardless of what the game asks for, so the frame
rate stays pinned to your screen's refresh rate either way. On a MacBook with a
ProMotion display that means it moves between 60 and 120 on its own, depending on
whether anything is animating.

This was measured rather than assumed. The setting does work on Windows and Linux,
which is why it is still there.

## What is not in these files

Some values live in the code because changing them without also changing an art
asset would silently break the game rather than produce an error.

The main one is hex tile geometry. The size of a hex is a measurement of the 3D
model in `assets/meshes/hex.glb` — changing the number without changing the model
makes tiles overlap or leaves gaps between them. If you need tiles at a different
size, the model has to change too. Ask a programmer.
