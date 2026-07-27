# Contributing

## Setup

```sh
cargo dev            # run with the world inspector and live asset reload
cargo run --release  # run as it ships: no inspector, much faster
```

The toolchain is pinned in `rust-toolchain.toml`; rustup fetches the right
compiler on first build. Install the dependency auditor once with
`cargo install cargo-deny --locked`. A cold build takes 10–20 minutes. Linux also
needs the system packages in the README appendix; macOS and Windows do not.

Only changing values in `assets/config/`? You want [docs/development/config.md](docs/development/config.md)
instead; none of this applies.

## Before opening a PR

CI always checks every relative link in tracked Markdown:

```bash
set -u
broken=$(mktemp)
while IFS= read -r file; do
  dir=$(dirname "$file")
  grep -oE '\]\([^)#][^)]*\)' "$file" 2>/dev/null \
    | sed -E 's/^\]\(//; s/\)$//; s/#.*$//' \
    | grep -vE '^[a-z]+:' \
    | while IFS= read -r link; do
        [ -z "$link" ] && continue
        [ -e "$dir/$link" ] || echo "BROKEN: $file -> $link"
      done
done < <(git ls-files '*.md') > "$broken"
if [ -s "$broken" ]; then cat "$broken"; exit 1; fi
echo "all relative links resolve"
```

Run the same Rust checks CI runs:

```sh
cargo fmt --all --check
cargo deny check
cargo clippy --workspace --all-targets --all-features --profile ci -- -D warnings
cargo test --workspace --all-features --profile ci
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
cargo build --workspace --profile ci
```

CI runs the final build command on Linux, Windows, and macOS. Run it on your local
platform; the CI matrix covers the other two. Markdown-only changes skip the Rust
commands, but still need valid relative links.

**Then run the game.** This is not optional, and it is not covered by the above.
Several failure modes here produce a clean log and a wrong window: missing assets
render as a plain blue screen, a sky shader that fails to load renders a black sky,
and a speed-unit mistake just looks slightly off. Every one of those passes CI.

If your change touches rendering, movement, or state transitions, walk it: splash
→ title → click a scenario → gameplay, orbit, click a tile, **ESC** to pause,
**BACKSPACE** to return to the title, then click a scenario again to rebuild the world.

## Where code goes

See [docs/architecture.md](docs/architecture.md) for the full reasoning. The short
version:

| Adding | Goes in |
|---|---|
| Hex math, voxel positions, substances, shared types, states, ordering sets | `hex_core` |
| The lattice rules engine: gems, fusions, spells, mana, disables, enchantments | `hex_lattice` |
| Voxels, terrain, tile spawning, map settings | `hex_map` |
| Asset loading, shared settings | `hex_assets` |
| Sky and camera | `hex_world` |
| Rules: input, movement, interaction | `hex_units` |
| A debug tool | `hex_dev` |
| A screen or menu | `hex_game` |

**`hex_map`, `hex_world` and `hex_units` may not depend on each other.** If you
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
`==` on floats, `unsafe`, and undocumented public items. Tests may unwrap, expect,
panic, debug and print because those are useful failure signals; slice indexing
and the other restriction lints remain denied there.

## House style

- Subsystem modules expose `pub fn plugin(app: &mut App)`, not a `Plugin` struct;
  support modules such as generators do not need one.
- Register reflected types in their crate's composing plugin, not in a central
  list. Shared `hex_core` types are registered by the runtime plugin that introduces
  them to the app.
- Put an `Update` system in an `AppSystems` phase when it participates in shared
  cross-crate ordering. Self-contained state and UI systems can run outside those
  phases. Ordering that crosses a crate boundary needs a shared set in `hex_core`;
  `.chain()` cannot express it, and a local chain that looks right will race.
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
compiler, clippy, the full test suite and every CI check. `dev` is where work is allowed
to be wrong until a human has looked at the window.

If you open a PR against `main` by mistake, retarget it rather than merging:

```sh
gh pr edit <N> --base dev
```

Branch prefixes: `chore/`, `fix/`, `perf/`, `feat/`, `docs/`, `refactor/`.
The retired top-level `refactor` branch is gone, so the `refactor/*` namespace is
available again.

Explain *why* in the commit message; the diff already says what. If you hit
something surprising, write it down — several comments in this codebase exist
because someone lost an hour to the thing they describe.

Merge with merge commits (`gh pr merge N --merge`), not squash, so per-PR history
is preserved.
