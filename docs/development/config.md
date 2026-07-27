# Changing the game without writing code

Most of how the game looks and feels is controlled by a handful of text files in
`assets/config/`. You can edit them in any text editor. You do not need to know
Rust, and you do not need to recompile the game.

| File | Controls |
|---|---|
| `world.ron` | Map size, terrain preset and shape, how tall a voxel is |
| `substances.ron` | What the world is made of — including water and metal — and its colours |
| `camera.ron` | Initial gameplay frame, pan speed, zoom and tilt |
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
| `camera.ron` | Movement values straight away; initial frame on the next rebuild |
| `display.ron` | Straight away |
| `world.ron` | On the next world rebuild |
| `substances.ron` | On the next world rebuild |
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

**Use procedural terrain instead.** The primary recipe separates the broad landform,
its materials, and its tactical structure:

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
to the scenario, as described in **Configuring a scenario** below.

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

**Time of day.** In `lighting.ron`, `sun_rotation` is the sun's angle in radians
(a full circle is about 6.28). Lower `sun_illuminance` towards `1000.0` for overcast;
`100000.0` is direct noon sun. `sun_color` tints it: warm values read as a low sun.

> **Change the second number, not the first.** The three are Euler angles rather than
> "height, compass, roll" — they wrap past 6.28, and whether the sun ends up above or
> below the horizon depends on the first two *together*. The second swings it round the
> compass and is safe to play with; changing the first as well can put the sun
> underneath the map, which lights nothing and renders the terrain as a black mass with
> no error anywhere. This has happened. There is now a test for it.

The sun is worth treating carefully. The terrain has no texture, so **shadows are the
only thing giving it shape** — changes that weaken them, or that light the shadowed
faces back up, tend to read as "flat" rather than "soft".

**Two extras that ship switched off.** Both live in `lighting.ron` and are worth
knowing about before you reach for them:

```ron
sky_light_intensity: 0.0,   // a soft fill from the sky itself
fog_density:         0.0,   // distance haze
```

`sky_light_intensity` adds light that varies with which way a surface faces — blue from
overhead, a warm bounce from the ground — so it colours shadows instead of flooding
everything equally. It is the honest way to soften shadows. Try `100`–`200` first;
several hundred already starts washing out the shading that gives terrain its shape.
`ground_color` sets the bounce colour underneath it.

`fog_density` hazes distant terrain, tinted `fog_color` and glowing `fog_sun_color`
towards the sun. At this camera distance it costs more colour than it buys atmosphere,
which is why it is off — `0.002` is about as much as is worth trying.

**The sky and its clouds.** The sky is drawn procedurally — a vertical colour
gradient with hexagonal clouds — and every part of it lives in `lighting.ron`.
Like everything else in that file it updates **straight away**: edit, save, and the sky
changes while you watch. It is the easiest thing in the game to tune.

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
