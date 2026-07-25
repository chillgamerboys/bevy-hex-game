# hex_map — context for AI agents

You are working in `crates/hex_map/`. This file is read automatically on every turn
in this directory. Read it before changing anything.

## What this crate owns

Everything about the map:

| File | Holds |
|---|---|
| `src/generator.rs` | Terrain generation. `HeightMap`, the `HeightGenerator` trait, Perlin and friends |
| `src/grid.rs` | Turning generated terrain into tile entities |
| `src/settings.rs` | Designer-facing settings, loaded from `assets/config/world.ron` |

Plus `assets/config/world.ron` itself, which is edited by a non-programmer.

## Your blast radius is bounded, deliberately

**Nothing depends on `hex_map` except the binary.** `hex_core`, `hex_assets`,
`hex_world` and `hex_gameplay` cannot see it. Cargo enforces this — a `use hex_map::`
in any of them fails to compile.

The consequence: **you cannot break gameplay, the camera, the sky, the screens or
the menus from here.** Work confidently inside this crate.

## The one contract you must keep

The rest of the game learns about terrain **through components on tile entities**,
never by reading anything defined here:

```rust
commands.spawn((
    HexTile,                    // marker
    hex_coord,                  // hex_core::HexCoord — which hex
    span,                       // hex_core::HexSpan  — which column
    Mesh3d(...), MeshMaterial3d(...), Transform { ... },
));
```

`hex_gameplay` queries `(&HexCoord, &HexSpan)` with `With<HexTile>`. That is the
entire interface.

**So: however you generate, store, or stream the map, spawn tiles carrying a
`HexCoord` and a `HexSpan`, and everything keeps working.** Replace `HeightMap`
wholesale if you want to — nothing outside this crate references it.

### The transform must agree with the span

A tile's `Transform` has to match the column it claims to occupy:

- `translation.y == span.centre()`
- `scale.y == span.height()`

Gameplay reads `span.top` to place a piece on a surface. If the transform disagrees,
pieces float or sink and **nothing errors** — the tiles still render. There is a test
for this (`tests/spawning.rs::every_tile_transform_matches_its_span`); keep it
passing.

## Columns, and the rule about them

A hex is a **column**, not a height. `HexSpan { bottom, top }`, in world units.

- Ground: `HexSpan::from_ground(3.0)` → `{ 0.0, 3.0 }`
- A floating platform: `HexSpan::new(8.0, 10.0)`
- A bridge over ground: **two tile entities** at one `HexCoord`, with disjoint spans

**Columns stacked at the same coordinate are not connected.** A unit on a bridge
cannot step down to the ground beneath it; reaching it means a ramp or spiral of
adjacent columns descending gradually, or an ability that explicitly bypasses the
rule. That is a game-design decision, and it means:

> **Never key a map by `HexCoord` alone in a way that collapses a stack.**
> `HashMap<HexCoord, f32>` keeping "the highest column" silently makes every lower
> column unreachable. `HeightMap` does this today only because today's terrain has
> exactly one column per coordinate.

`HexSpan::overlaps` tells you whether two columns collide. `HexSpan::step_to` gives
the height difference between two surfaces.

## Rules that will block your commit

CI runs `cargo clippy --workspace --all-targets --all-features -- -D warnings`.

**`#[allow(...)]` is banned.** If you cannot satisfy a lint, you must write:

```rust
#[expect(clippy::some_lint, reason = "why this is genuinely fine here")]
```

Silencing a warning without a reason is not available to you. This is deliberate.

Also denied: `unwrap()`, `panic!`, `todo!`, `unimplemented!`, slice indexing
(`v[0]` — use `.get()`, `.first()`, or destructure), `dbg!`, `println!` (use Bevy's
`info!`/`warn!`), comparing floats with `==`, and any undocumented public item.

Restriction lints are relaxed inside `#[test]` functions — a panic there is the
point.

## Scheduling: where your systems go

Systems that build the world run on `OnEnter(Screen::Gameplay)`, in one of:

```rust
GameplaySetup::Resources   // insert resources — the height map goes here
GameplaySetup::Terrain     // spawn tiles — needs Resources to have run
GameplaySetup::Actors      // hex_gameplay's, not yours; needs tiles to exist
```

**Do not put tile spawning outside `Terrain`.** Systems in one `OnEnter` schedule run
in *unspecified order* unless a set says otherwise, and the set boundary is also what
supplies the sync point — entities created via `Commands` are not queryable until the
queue is applied. Both halves matter; ordering alone is not enough. This has caused a
real bug (the player spawned before the tiles existed and sank into the ground).

Clean up on `OnExit(Screen::Gameplay)`. There is a test that nothing leaks.

## Things that fail silently here

A clean log is not evidence a change worked. **Look at the window.**

| Symptom | Cause |
|---|---|
| Plain blue window | Assets not found — run through `cargo`, never the binary directly |
| Tiles in the wrong place, no error | Transform disagrees with the span |
| Stuck on "loading…" | `world.ron` failed to parse. The terminal names the line |
| Terrain differs every run | `seed: None` in `world.ron`. Set a number to reproduce a map |
| Tile scaled to nothing | A zero-height span. `HexSpan::new` refuses these; check you used it |

## Working here

```sh
cargo dev                      # run with inspector and live asset reload
cargo test -p hex_map          # fast; no GPU needed
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Editing `assets/config/world.ron` while `cargo dev` is running reloads it, but the
world is only rebuilt on entering gameplay — press `BACKSPACE` then `ENTER` to see
terrain changes.

`HeightGenerator` implementations **must be pure**: the same coordinate must always
give the same height. Results are cached, so an impure generator produces terrain
that changes depending on what has been looked at.

## Before you finish

1. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
2. `cargo test --workspace`
3. **Run the game and look at it.** Every bug found in this codebase so far was found
   by a human looking at the window, not by CI.

## Further reading

- [`docs/ONBOARDING.md`](../../docs/ONBOARDING.md) — start here if the vocabulary is new
- [`docs/CONTENT.md`](../../docs/CONTENT.md) — editing settings without code
- [`docs/ARCHITECTURE.md`](../../docs/ARCHITECTURE.md) — the whole crate graph and why
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md) — house style
