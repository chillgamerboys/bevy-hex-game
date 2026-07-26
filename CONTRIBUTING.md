# Contributing

## Setup

```sh
cargo dev            # run with the world inspector and live asset reload
cargo run --release  # run as it ships: no inspector, much faster
```

The toolchain is pinned in `rust-toolchain.toml`; rustup fetches the right
compiler on first build. A cold build takes 10–20 minutes. Nothing else is
required on macOS or Windows — see the README appendix for Linux and WSL2.

Only changing values in `assets/config/`? You want [docs/CONTENT.md](docs/CONTENT.md)
instead; none of this applies.

## Before opening a PR

Everything CI checks, runnable locally:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check
```

**Then run the game.** This is not optional, and it is not covered by the above.
Several failure modes here produce a clean log and a wrong window: missing assets
render as a plain blue screen, a missed skybox event renders a black sky, and a
speed-unit mistake just looks slightly off. Every one of those passes CI.

If your change touches rendering, movement, or state transitions, walk it: splash
→ title → **ENTER** → gameplay, orbit, click a tile, **ESC** to pause, **BACKSPACE**
to return to the title, **ENTER** again to rebuild the world.

## Where code goes

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full reasoning. The short
version:

| Adding | Goes in |
|---|---|
| Hex math, voxel positions, substances, shared types, states, ordering sets | `hex_core` |
| Voxels, terrain, tile spawning, map settings | `hex_map` |
| Asset loading, shared settings | `hex_assets` |
| Sky and camera | `hex_world` |
| Rules: input, movement, interaction | `hex_gameplay` |
| A debug tool | `hex_dev` |
| A screen or menu | `hex_game` |

**`hex_map`, `hex_world` and `hex_gameplay` may not depend on each other.** If you
need something in more than one, it belongs in `hex_core`. Cargo will stop you either
way; this is just the reason.

`hex_map` is a leaf — nothing depends on it but the binary. That is what makes it
safe to hand to one owner.

## Lints

CI runs clippy with `-D warnings` and a deliberately strict configuration, because
much of this code is written by AI agents and lints catch a specific class of mistake
more cheaply than review does.

**`#[allow(...)]` is banned.** If you cannot satisfy a lint, write
`#[expect(the_lint, reason = "why this is genuinely fine")]`. `#[expect]` also warns
when the lint stops firing, so stale suppressions surface instead of accumulating.
Without this rule, an agent that cannot satisfy a lint simply switches it off and
every other rule becomes advisory.

Also denied: `unwrap()`, `panic!`, `todo!`, slice indexing, `dbg!`, `println!`,
`==` on floats, `unsafe`, and undocumented public items. Restriction lints are
relaxed inside `#[test]` functions — a panic there is the failure signal.

## House style

- Modules expose `pub fn plugin(app: &mut App)`, not a `Plugin` struct.
- Register reflected types beside the type, not in a central list.
- Put every `Update` system in an `AppSystems` set. Ordering that crosses a crate
  boundary needs a shared set in `hex_core` — `.chain()` cannot express it, and a
  local chain that looks right will race.
- Observers are global and fire in every state. If one touches a resource that
  only exists during gameplay, take it as `Option<Res<T>>` or it will panic in a
  menu.
- Speeds are world units per **second**, driven by `Res<Time>`. Never `SystemTime`.

## Commits and PRs

### Branch off `dev`, and open your PR against `dev`

```
feat/whatever  ──PR──►  dev  ──PR──►  main
```

```sh
git checkout dev && git pull
git checkout -b feat/your-thing
# ...work...
gh pr create --base dev
```

**`dev` is permanent.** It is the integration branch, not a release branch that gets
tidied up afterwards — never delete it. Feature branches are deleted once merged;
`dev` is not.

`main` moves only by merging `dev` into it, as a deliberate promotion after someone
has actually played the game. That gap exists for a specific reason: **CI cannot see
anything.** It will happily pass a black sky, a hairline gap between every tile, or a
piece standing inside the terrain — all three have happened here, green across the
compiler, clippy, the full test suite and five CI jobs. `dev` is where work is allowed
to be wrong until a human has looked at the window.

If you open a PR against `main` by mistake, retarget it rather than merging:

```sh
gh pr edit <N> --base dev
```

Branch prefixes: `chore/`, `fix/`, `perf/`, `feat/`, `docs/`.

**`refactor/*` branch names cannot exist** while a branch named `refactor` does — a
git ref cannot be both a file and a directory. Use another prefix.

Explain *why* in the commit message; the diff already says what. If you hit
something surprising, write it down — several comments in this codebase exist
because someone lost an hour to the thing they describe.

Merge with merge commits (`gh pr merge N --merge`), not squash, so per-PR history
is preserved.
