# Getting started

For someone taking ownership of **the map** — the hex grid, its terrain, and how it
is drawn. It assumes no Rust and no game-engine experience, and it should get you
from nothing to a running game you can change.

If you only want to tweak numbers — how tall the hills are, what colour the tiles
are — you may not need this at all. Try [CONTENT.md](CONTENT.md) first.

## 1. Run it

```sh
cargo dev
```

The first build takes 10–20 minutes; it is compiling the entire game engine. After
that it is seconds. You should get a window with a hex grid, a red piece in the
middle, and a sky.

Press `ENTER` at the title screen. Then:

| | |
|---|---|
| Right-drag | turn the camera |
| `W` `A` `S` `D` | move the camera |
| Scroll | zoom |
| Click a tile | the piece walks there |
| `ESC` | pause |
| `BACKSPACE` | back to the title screen |

`cargo dev` gives you a live inspector and reloads asset files as you save them.
`cargo run --release` is the faster, shipping build with neither.

**Always start it through `cargo`.** Running the built file directly gives you a
plain blue window, because the game looks for its artwork in the wrong place. It is
not a crash and there is no useful error.

## 2. The words

Enough to read the code and the other docs.

**Crate** — a folder of code that gets compiled as a unit, like a module or package.
This project has six, under `crates/`. Yours is `hex_map`.

**Entity** — one thing in the world: a tile, the player piece, the camera, the sun.
Just an ID.

**Component** — a piece of data attached to an entity. A tile has a `HexCoord`
(which hex it is), a `HexSpan` (its column), a `Transform` (where it is in 3D).

**System** — a function that runs every frame, or when something happens. "Move
everything that is moving" is a system.

**Resource** — data that exists once for the whole game rather than per entity. The
height map is one.

**Plugin** — a bundle of systems that get switched on together. Each file in your
crate exposes one.

That is the whole of Bevy's model: entities have components, systems act on them.

### Hex coordinates

Hexes are addressed with **three** numbers, `x`, `y`, `z`, that always add up to
zero. It sounds redundant and it makes the maths much easier — distance is
straightforward, and so are rotations.

```rust
HexCoord::new_cubic(3, -5, 2)   // 3 + (-5) + 2 == 0
HexCoord::ORIGIN                // the centre
```

Only two are stored; the third is worked out. You never see that.

### Columns

**A hex is a column, not a height.** It has a bottom and a top:

```rust
HexSpan::from_ground(3.0)   // ordinary ground, 0.0 up to 3.0
HexSpan::new(8.0, 10.0)     // a platform floating in mid-air
```

Right now every column starts at ground level, because the terrain is a simple
height field. The type is built for more than that: **a bridge over open ground is
two tile entities at the same coordinate**, one on the ground and one in the air.

There is a rule about those, and it is a game-design decision rather than a
technical one:

> **Stacked columns are not connected.** Someone on a bridge cannot step down to the
> ground underneath. They have to walk a ramp or spiral that descends gradually, or
> use something that explicitly bypasses it — a teleport, a tunnel.

The practical version: **never write code that reduces a coordinate to one height.**
If two columns share an address and you keep "the highest", the lower one silently
becomes unreachable.

## 3. What is yours

```
crates/
  hex_map/       ← YOURS. terrain, tiles, map settings
  hex_core/      shared vocabulary — HexCoord, HexSpan
  hex_assets/    loading files from disk
  hex_world/     camera and sky
  hex_gameplay/  the player and movement
  hex_dev/       the inspector
  hex_game/      wiring it all together
assets/
  config/world.ron   ← YOURS. the map's settings
```

**You cannot break the others by accident.** Nothing depends on `hex_map`, and the
compiler refuses any attempt to reach into it from elsewhere. If someone else's code
stops working, it is not because of a change you made inside your crate.

That also means: if you find yourself needing to edit `hex_core` or `hex_game`, stop
and talk to whoever owns gameplay first. It is allowed — it is just a conversation
worth having, because those files are shared.

The one thing you must keep: **tiles carry a `HexCoord` and a `HexSpan`.** That is
how the rest of the game finds out where the ground is. Everything else about how
the map works is yours to change.

## 4. Knowing you have not broken anything

```sh
cargo test --workspace     # ~40 tests, a couple of seconds
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Both run automatically on every pull request, and both must pass.

**Then run the game and look at it.** This matters more than it sounds. Every bug
found in this project so far was found by a person looking at the window — including
a crash and a piece sunk into the ground, both of which passed every automated check
at the time. The tests raise the floor; they do not replace looking.

## 5. When it will not build

The compiler is strict here on purpose, because much of this code is written by AI
agents and the rules catch a specific class of mistake. The messages are usually
precise about the fix.

**`#[allow] attribute found`** — something tried to switch a warning off. That is
banned; the warning has to be fixed, or converted to
`#[expect(the_lint, reason = "why this is fine")]` with a real explanation.

**`missing documentation`** — every public thing needs a `///` comment above it
saying what it is.

**`used unwrap()`** / **`indexing may panic`** — code that would crash the game if
something unexpected happened. Use `.get()` and handle the "nothing there" case.

**`Resource does not exist`** at runtime — something read data that had not been
created yet. Usually a system in the wrong stage; see the scheduling notes in
`crates/hex_map/CLAUDE.md`.

## 6. Where to go next

| | |
|---|---|
| [`crates/hex_map/CLAUDE.md`](../crates/hex_map/CLAUDE.md) | The rules for your crate. Your AI agent reads this automatically |
| [CONTENT.md](CONTENT.md) | Changing settings without code |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Why the project is shaped this way |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | House style and the checks |

If something in here turns out to be wrong or missing, changing it is a perfectly
good first pull request.
