# Changing the game without writing code

Most of how the game looks and feels is controlled by a handful of text files in
`assets/config/`. You can edit them in any text editor. You do not need to know
Rust, and you do not need to recompile the game.

| File | Controls |
|---|---|
| `world.ron` | Map size, terrain preset and shape, how tall a voxel is |
| `substances.ron` | What the world is made of — including water and metal — and its colours |
| `elements.ron` | The six-element wheel, opposition, higher-order elements and fusion recipes |
| `spells.ron` | Spells: what each requires, how it is cast, and what it does |
| `camera.ron` | Initial map and close-character frames, pan speed, zoom and tilt |
| `combat.ron` | Engagement thresholds, movement budget, height bonus, and the open design questions as policy knobs that reject unbuilt variants with a reason |
| `lighting.ron` | Sun brightness, colour and angle, ambient light, the sky gradient and its hex clouds |
| `player.ron` | Player piece size, movement speed and colour |
| `scenarios.ron` | What the title screen offers: a map, a sky and where the units start |
| `menu.ron` | How the menu screens look |
| `display.ron` | Vsync / frame rate behaviour |

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
| `camera.ron` | Movement/follow values straight away; close preset on the next `C`; initial map frame on the next rebuild |
| `display.ron` | Straight away |
| `world.ron` | On the next world rebuild |
| `substances.ron` | On the next world rebuild |
| `elements.ron` | On the next world rebuild (re-parsed and validated on save) |
| `spells.ron` | On the next world rebuild (re-parsed and validated on save) |
| `lighting.ron` | Straight away, all of it — sun, ambient, sky and clouds |
| `player.ron` | Speed on the next movement started; scale and colour on the next rebuild |
| `scenarios.ron` | On the next world rebuild |
| `menu.ron` | Straight away |

**To rebuild the world**, press `BACKSPACE` to return to the title screen, then click
the scenario you want to start again. It takes under a second and picks up your edit.

The split exists because some values are read continuously while the game runs and
others are read once, when the map and pieces are created. Nothing is lost either
way — the rebuild is quick.

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
| `HEX_REVIEW_VIEW` | Uses `default`, `rotated`, or `top-down` map azimuth |
| `HEX_REVIEW_CAMERA` | Uses the `map` or close `character` camera |
| `HEX_REVIEW_TIME` | Sets a cyclic-lighting hour from `0.0` up to, but not including, `24.0` |
| `HEX_REVIEW_FOCUS_ANCHOR` | Moves the selected actor to one exact generated map anchor before framing |
| `HEX_REVIEW_CUTAWAY` | `full` hides the complete roof of the selected interior instead of the local six-hex opening |

`HEX_REVIEW_VIEW`, `HEX_REVIEW_CAMERA`, `HEX_REVIEW_FOCUS_ANCHOR`, and
`HEX_REVIEW_CUTAWAY` require `HEX_REVIEW_CAPTURE`. The focus override resolves the
anchor's full `TilePos`, not just its horizontal coordinate, so it can target an
underground floor beneath a surface. It also applies the selected actor's normal
solidity and headroom rules. An unknown anchor or one the actor cannot stand on fails
the review process instead of silently capturing the wrong place. The full cutaway
still requires the selected actor to occupy an exact interior surface and affects
only that interior; ordinary gameplay retains the local cutaway.

For example, this exposes the complete generated cave network for a top-down overview:

```sh
HEX_REVIEW_SCENARIO="Caves" \
HEX_REVIEW_CAPTURE=".context/caves/full-overview.png" \
HEX_REVIEW_FOCUS_ANCHOR="conflict_center" \
HEX_REVIEW_VIEW="top-down" \
HEX_REVIEW_CUTAWAY="full" \
cargo run -p hex_game --release --features map-review
```

Use the unoccupied `conflict_center` anchor for a neutral cave overview.
`deep_chamber` is also the configured enemy position, so relocating the player there
can start combat before capture.

## The format

These are RON files. Three rules cover almost everything:

**Text after `//` is a comment.** It is ignored by the game, so you can leave notes
for yourself.

**Every value needs a comma after it**, including the last one in a list. This is
the single most common mistake.

**Decimal numbers need a decimal point.** Write `1.0`, not `1`. Whole numbers like
`grid_radius: 20` are the exception — those are counts, and are written plainly.

Colours are written as `(red, green, blue)`, each from `0.0` to `1.0`:

```ron
"grass": (
    color: (0.35, 0.62, 0.30),
    solid: true,
    diggable: true,
),
```

`0.0, 0.0, 0.0` is black, `1.0, 1.0, 1.0` is white.

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

**Use current procedural terrain.** Generator version 2 uses one geometry recipe plus
a separate material environment. Hills is the first shipped V2 recipe:

```ron
terrain: Procedural((
    generator_version: 2,
    environment: TemperateGrassland,
    recipe: Hills((
        valley_level: 15,
        max_relief: 8,
        hills_per_bank: 3,
    )),
)),
```

V2 Hills preserves the approved V1 maps for equivalent Hills settings and seeds while
publishing them through the V2 volume pipeline. It derives its three-wide hazard,
two-wide crossings, bed and hazard bounds, and bridge level from `valley_level`; those
invariants are intentionally not editable. Temperate, Frozen, and Volcanic Hills use
this recipe in the shipped scenario library.

`LayeredSkyIslands` finalizes the same Hills ground before sampling any independent
`sky.*` stream, then adds three primary islands, one or two satellites, and a two-wide
upper bridge network:

```ron
recipe: LayeredSkyIslands((
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
chambers, two floor bands, loop corridors, varied chamber heights, and the most
developed rocky surface. Six-through-eight-room settings retain their deterministic
compatibility geometry.
Exact floor, entrance, and cutaway-roof memberships remain keyed by `TilePos`, so the
underground floor cannot be confused with the surface above it. The hostile remains
inside the deepest chamber on the floor with the greatest minimum horizontal
separation from the complete ramp and entry connector. A live scenario regression
checks that placement against the loaded combat policy so combat through rock cannot
interrupt entry. Recipe and environment combinations are validated together:
Mountains requires `Frozen`, Caves requires `Rocky`, and invalid combinations leave
the previous valid hot-reloaded settings active.

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

**A new substance.** In the map-owned `substances.ron`, copy an entry and change
the name:

```ron
"sand": (
    color: (0.85, 0.78, 0.55),
    solid: true,
    diggable: true,
),
```

Saving the file registers it with the game. It will not appear in generated terrain
until the generation code selects it. `air` must always be present — it means empty
space.

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

`menu.ron` is the splash, title and loading screens. One value so far:

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

`scenarios.ron` entries can name a lighting file and choose how units are placed:

```ron
(
    name: "Rolling Hills",
    world: "config/worlds/rolling-hills.ron",
    lighting: "config/lighting/overcast.ron",
    units: (
        player: Fixed((x: 0, y: 0, z: 0)),
        enemy: Fixed((x: 5, y: -5, z: 0)),
    ),
),
```

Leave `lighting` out and the scenario gets `config/lighting.ron`, which is what most
should do. A lighting file is a complete copy of that file's contents — start by
copying it and changing what you want.

`Fixed(...)` is for authored terrain whose landmarks never move. Generated terrain
instead uses anchors published by the generator and owns its reproducible seed here:

```ron
(
    name: "Procedural Hills",
    world: "config/worlds/procedural-hills.ron",
    generation_seed: Some(1592598566),
    units: (
        player: Anchor("party_start"),
        enemy: Anchor("hostile_start"),
    ),
),
```

The title screen shows the resolved seed beside every generated scenario. Its
`reroll` button changes only the current session, and the exact replacement seed is
shown and logged so a useful or broken map can be reproduced. It never edits
`scenarios.ron`; restarting returns to the configured seed.

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

Both `world` and `lighting` are paths, and neither is checked by the compiler. A typo
fails `cargo test` rather than at the loading screen, but only because a test opens
every file the scenarios name — keep it that way.

## Elements and spells

Two files define the magic system as content. Nothing in the game reads them *yet* —
the lattice that actually casts spells is being built alongside this — but they load,
validate and cross-check now, so authoring can begin.

### `elements.ron`

The six-element **wheel** and the **fusion recipes** that build higher-order elements
from it.

```ron
(
    wheel: ["Light", "Air", "Fire", "Metal", "Earth", "Water"],
    fusions: {
        "Lightning": [(element: "Light", mana: 1), (element: "Fire", mana: 1)],
    },
)
```

- **`wheel`** lists the basic elements. Opposition is their position on the wheel:
  each element opposes the one halfway round — with six, that is three apart, giving
  Light/Metal, Air/Earth, Fire/Water. Reorder the wheel and you change *which elements
  oppose which*; that is the wheel's whole job.
- **`fusions`** are higher-order elements. Each names its output and the inputs it
  draws (an element and how much mana). Lightning is Light + Fire. A fusion output is
  never itself a basic wheel element, every input must be a basic element or another
  fusion's output, and the recipes may not form a loop — all checked when the file
  loads.

One rule worth knowing: an element's internal **id is assigned from its name in
alphabetical order**, *not* from where it sits in the file or on the wheel. So you can
reorder entries freely without silently rewriting anything — and it is why wheel order
(which sets opposition) is written out separately.

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
            targeting: (range: 3, shape: Single, needs_los: true),
            effects: [DisableHexes(count: 1, targeted: false)],
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
- **`targeting`** is `range` (how far away the target may be, in hexes), `shape` (what
  the spell covers once it gets there) and `needs_los` (whether line of sight is
  required — parsed, but not enforced until obstruction lands).

  `range` and a shape's own extents are different numbers. Fireball's `range: 4` is
  how far it is thrown; its `Sphere(radius: 2)` is how big the ball is.

  | Shape | What it covers |
  |---|---|
  | `SelfCast` | the caster's own voxel; `range` must be `0` |
  | `Single` | one target voxel |
  | `Sphere(radius: N)` | everything within `N` of the target — `N` hexes out *and* `N` levels up or down |
  | `Column(height: N)` | the target voxel and the `N - 1` voxels above it; a conjured wall is `2` |
  | `Line(length: N, width: W)` | `N` hexes out from the caster; `W` is a half-thickness, so `0` is a single file |
  | `Cone(length: N, spread: S)` | widening out from the caster; `S` is 60° sectors *each side*, so `0` is a ray, `1` the usual cone, `3` a full disc |
  | `Path(offsets: [...])` | a hand-authored voxel list, `(coord: (q: 1, r: 0), level: 2)` each, rotated to the facing |

  **Vertical and horizontal count equally.** A radius-3 sphere reaches three hexes out
  and three levels up or down, so it looks slightly squashed on screen. That is
  deliberate: gameplay is not allowed to know how tall a voxel is drawn, so there is no
  other honest answer.

  `Line`, `Cone` and `Path` point somewhere, so a cast using one has to name a facing;
  the other four look the same in every direction. `Line` and `Cone` never include the
  caster's own voxel.

  Extents are capped at **16** (and cone spread at 3, a full disc). That is a guard
  rail, not balance: a resolved volume is a real list of voxels, so a radius typed with
  an extra digit is tens of millions of them. A file that names one fails to load with
  the spell and the field in the message.
- **`effects`** is a **fixed list** of what a spell can do — you cannot invent new ones
  without a programmer, which is deliberate:

  `DisableHexes`, `Burn`, `RestoreHexes`, `ModifyIncomingDisables`, `Reveal`,
  `Illuminate`, `SetTerrain`, `ClearTerrain`, `SpawnWall`, `Displace`.

  `SetTerrain` and `SpawnWall` name a substance from `substances.ron`.

### When a reference is wrong

A spell that requires an element `elements.ron` does not define, or an effect that
names a substance `substances.ron` does not have, is a **dangling reference**. On load
the game logs exactly which one and keeps the last content that was valid — the same
way a broken `lighting.ron` keeps the last good sky. A test also opens every shipped
file and fails if any reference dangles, so the shipped game never carries a broken one.

## One thing that will not do anything on a Mac

`display.ron` controls vsync. On macOS it has no visible effect: the system
composites every window and syncs it to the display regardless of what the game
asks for, so the frame rate stays pinned to your screen's refresh rate either way.
On a MacBook with a ProMotion display that means it moves between 60 and 120 on
its own, depending on whether anything is animating.

This was measured rather than assumed. The setting does work on Windows and Linux,
which is why it is still there.

## What is not in these files

Some values live in the code because changing them without also changing an art
asset would silently break the game rather than produce an error.

The main one is hex tile geometry. The size of a hex is a measurement of the 3D
model in `assets/meshes/hex.glb` — changing the number without changing the model
makes tiles overlap or leaves gaps between them. If you need tiles at a different
size, the model has to change too. Ask a programmer.
