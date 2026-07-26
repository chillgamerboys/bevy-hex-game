# Changing the game without writing code

Most of how the game looks and feels is controlled by six text files in
`assets/config/`. You can edit them in any text editor. You do not need to know
Rust, and you do not need to recompile the game.

| File | Controls |
|---|---|
| `world.ron` | Map size, terrain shape, terrain seed, how tall a voxel is |
| `substances.ron` | What the world is made of — stone, dirt, grass — and their colours |
| `camera.ron` | How fast the camera pans, how far it can zoom and tilt |
| `lighting.ron` | Sun brightness and angle, the sky gradient and its hex clouds, ambient light |
| `player.ron` | Player size, movement speed, colour, how many levels tall |
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
| `camera.ron` | Straight away |
| `display.ron` | Straight away |
| `world.ron` | On the next world rebuild |
| `substances.ron` | On the next world rebuild |
| `lighting.ron` | Sky-dome colours and clouds straight away; sun, ambient light and direction on the next rebuild |
| `player.ron` | Speed on the next movement started; size, colour and `levels_tall` on the next rebuild |

**To rebuild the world**, press `BACKSPACE` to return to the title screen, then
`ENTER` to start again. It takes under a second and picks up your edit.

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

**On initial startup, the game sits on "loading…" forever.** One of the files has a
mistake in it — most likely a missing comma. The terminal will name the file, the
line, and the column.

The game deliberately refuses to start without one valid value for every setting.
If a hot reload fails later, the asset server reports the error and the last valid
settings stay active; fix the file and save it again.

**A change had no effect.** Check you saved the file, and that you are running with
`cargo dev` rather than `cargo run --release`.

**You want to undo everything.** These files are tracked in git:

```sh
git checkout assets/config/
```

## Things worth trying

**A world you can come back to.** By default the terrain is different every launch.
In `world.ron`:

```ron
seed: Some(20260725),
```

Any number works. The same number always produces the same map, so if you find a
map you like, write the number down.

**Bumpier terrain.** In `world.ron`, `magnitude` is how tall the hills are and
`x_freq` / `y_freq` are how close together. Bigger frequencies mean rougher ground:

```ron
steps: [
    (x_freq: 0.035, y_freq: 0.05, magnitude: 3.0),
],
```

Add a second line with a *higher* frequency and *smaller* magnitude to lay fine
detail over the broad shape:

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

**A bigger map.** `grid_radius: 20` gives 1261 columns. Be careful: the tile count
grows quadratically, so `40` is 4921 columns and `100` is over 30,000 — and each
column draws several prisms, one per band of substance. Raise it a little at a time and
watch the frame rate.

**Chunkier terrain.** `level_height` in `world.ron` is how tall one voxel is. The
default `0.4` is quite flat; raising it towards `1.0` gives blockier terrain that reads
better once you are digging into it.

**Time of day.** In `lighting.ron`, `sun_rotation` is the sun's angle in radians
(a full circle is about 6.28). Lower `sun_illuminance` towards `1000.0` for
overcast; `100000.0` is direct noon sun. Rebuild the world to see it.

**The sky and its clouds.** The sky is drawn procedurally — a vertical colour
gradient with hexagonal clouds — and every part of it lives in `lighting.ron`.
Unlike the sun, these update **straight away**: edit, save, and the sky changes while
you watch. It is the easiest thing in the game to tune.

The gradient runs between two colours:

```ron
sky_color:    (0.55, 0.80, 0.95),  // at the horizon
zenith_color: (0.25, 0.50, 0.85),  // straight up
```

`sky_color` doubles as the fallback colour behind everything, so keep it a believable
sky tone. Set the two close together for a flat, even sky; push them apart for a
deeper, more dramatic one.

The clouds are hexagons — a nod to the grid — shaped by four more values:

```ron
cloud_color:     (0.97, 0.98, 1.0),  // usually near-white
cloud_coverage:  0.4,   // 0.0 is a clear sky, 1.0 clouds every cell
hex_cloud_scale: 13.0,  // bigger = smaller and more numerous clouds
cloud_softness:  0.12,  // 0.02 is crisp-edged, 0.3 is soft and fluffy
```

- **`cloud_coverage`** is how much of the sky is clouded, from none to solid.
- **`hex_cloud_scale`** sets cloud *size* — but inverted: turn it **up** for many
  small hexes, **down** for a few big ones.
- **`cloud_softness`** blurs the hex edges. Low keeps them sharp and geometric; high
  makes them read as soft clouds that happen to be hex-shaped.

For a clear blue sky, set `cloud_coverage: 0.0`. For an overcast one, raise coverage
and soften the edges. The colours take `(red, green, blue)` from `0.0` to `1.0`, the
same as everywhere else.

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
