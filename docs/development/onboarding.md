# Map contributor onboarding

For someone taking ownership of **the map** — the hex grid, its terrain, and how it
is drawn. It assumes no Rust and no game-engine experience, and it should get you
from nothing to a running game you can change.

If you only want to tweak numbers — how tall the hills are, what colour the tiles
are — you may not need this at all. Try [config.md](config.md) first.

## Before you begin

Follow [setup.md](setup.md) through its first-run smoke test. Come back here once
**The Crossing** launches correctly; everything below is specific to understanding
and changing the map.

## The words

Enough to read the code and the other docs.

**Crate** — a folder of code that gets compiled as a unit, like a module or package.
This project has twelve, under `crates/`. Yours is `hex_map`.

**Entity** — one thing in the world: a tile, the player piece, the camera, the sun.
Just an ID.

**Component** — a piece of data attached to an entity. A rendered tile has a
`HexTile` marker, a `HexCoord` (which hex it is), a surface `TilePos` (which level
is on top), a `HexSpan` (the prism it draws), a `SubstanceId` (what it is made of),
and `Headroom` (how much clear space is above it).

**System** — a function that runs every frame, or when something happens. "Move
everything that is moving" is a system.

**Resource** — data that exists once for the whole game rather than per entity. The
voxel map is one.

**Plugin** — a bundle of systems that get switched on together. Subsystem modules
such as `grid.rs` expose one; support modules such as `generator.rs` do not.

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

### Voxels and columns

The world is made of **hex prisms stacked in columns**. Each one is at a *level* —
how far up it is, counting from the bedrock floor at level 0 — and is made of some
*substance*: stone, dirt, grass.

```
level 4   grass
level 3   dirt
level 2   air        ← a cave
level 1   stone
level 0   bedrock
```

A **position** is a coordinate plus a level. That is how everything in the world is
addressed, and it is what makes a bridge and the ground beneath it different places.

One thing that surprises people: **a tile you see is usually several voxels**. Runs of
the same substance are merged into one prism, so a fifteen-level stone column is drawn
once rather than fifteen times. That keeps the game fast, and it means a voxel buried
inside a column has no object of its own — it is found by position.

Because of that, a tile also carries its **headroom**: how many clear voxels sit above
it. Zero means the tile is buried inside a column, so it is solid rock rather than
somewhere to stand. A small number means a low ceiling — a character is two levels
tall, so a one-voxel gap under a bridge is a wall to it and a corridor to something
smaller. **Whether terrain is walkable depends on who is walking.**

There is a rule about those, and it is a game-design decision rather than a
technical one:

> **Stacked surfaces are not connected.** Someone on a bridge cannot step down to
> the ground underneath. They have to walk a ramp or spiral across adjacent
> coordinates that descends gradually, or use something that explicitly bypasses
> it — a teleport, a tunnel.

The practical version: **never write code that reduces a coordinate to one height.**
If one column exposes several surfaces and you keep only the highest, every lower
one silently becomes unreachable.

## What the map owns

```
crates/
  hex_map/       ← YOURS. terrain, tiles, map settings
  hex_core/      shared vocabulary — HexCoord, HexSpan, TilePos, Headroom
  hex_assets/    loading files from disk
  hex_objects/   static authored-object presentation
  hex_world/     camera and sky
  hex_units/     the player and movement
  hex_combat/    turns, commands, and combat policy
  hex_lattice/   the pure magic rules engine
  hex_anim/      reusable animation vocabulary
  hex_dev/       the inspector
  hex_game/      wiring it all together
  hex_editor/    the standalone Asset Workshop
assets/
  config/world.ron        ← YOURS. the map's shape and terrain settings
  config/substances.ron   ← YOURS. the map's substance catalogue
```

Only the `hex_game` binary depends on `hex_map`; it wires the map into the app.
The other library crates, including `hex_units` and `hex_world`, cannot import
it. Cargo enforces that dependency direction, so the map's implementation stays
isolated at compile time.

That boundary does not make every map change harmless at runtime. Gameplay reads
the tile components that the map publishes, so missing or incorrect values can
still break movement or drawing. The compiler protects the dependency boundary;
the component contract protects behaviour.

That also means: if you find yourself needing to edit `hex_core` or `hex_game`, stop
and talk to whoever owns gameplay first. It is allowed — it is just a conversation
worth having, because those files are shared.

The contract you must keep is that every rendered tile carries **`HexTile`,
`HexCoord`, a surface `TilePos`, `HexSpan`, `SubstanceId`, and `Headroom`**.
`TilePos` names the topmost material voxel in the rendered run, not its base;
`SubstanceId` says whether it is solid, and `Headroom` reports the clear levels above
it. That is how the rest of the game finds out what and where the ground is.
Everything behind that interface is yours to change.

Both `world.ron` and `substances.ron` are map-owned content. Their loading machinery
and the shared `SubstanceTable` live in `hex_assets`, because gameplay also needs
to ask whether a substance is solid, but decisions about the map's settings and
substance catalogue belong here. Rendered substances reference the shared visual
palette by stable swatch id; adding a material therefore requires coordinating its
map-owned behavior with its palette entry rather than embedding another RGB literal.

## Knowing you have not broken anything

```sh
cargo test --workspace     # full workspace suite, a couple of seconds
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Both run automatically on every pull request that changes code or configuration,
and both must pass. Markdown-only changes run the documentation link check instead.

**Then run the game and look at it.** This matters more than it sounds. Every bug
found in this project so far was found by a person looking at the window — including
a crash and a piece sunk into the ground, both of which passed every automated check
at the time. The tests raise the floor; they do not replace looking.

## When it will not build

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

Inside `#[test]` functions, `unwrap()`, `expect()`, `panic!`, `dbg!`, and terminal
printing are relaxed because a panic is how a test reports failure. The other
strict lints still apply there: in particular, slice indexing remains denied, so
use `.get()`, `.first()`, or destructuring in tests too.

**`Resource does not exist`** at runtime — something read data that had not been
created yet. Usually a system in the wrong stage; see the scheduling notes in
`crates/hex_map/CLAUDE.md`.

## Where to go next

| | |
|---|---|
| [systems/map.md](../systems/map.md) | How the map works: voxels, substances, and the rules |
| [`crates/hex_map/CLAUDE.md`](../../crates/hex_map/CLAUDE.md) | The rules for your crate. Your AI agent reads this automatically |
| [config.md](config.md) | Changing settings without code |
| [architecture.md](../architecture.md) | Why the project is shaped this way |
| [CONTRIBUTING.md](../../CONTRIBUTING.md) | House style and the checks |
| [troubleshooting.md](troubleshooting.md) | When the window looks wrong and the log says nothing |
| [the docs index](../README.md) | Everything else, and who each doc is for |

If something in here turns out to be wrong or missing, changing it is a perfectly
good first pull request.
