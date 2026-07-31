# Contributing

## Setup

Follow the [setup guide](docs/development/setup.md) for prerequisites, platform
packages, first-run verification, and troubleshooting. The quick start is:

```sh
cargo dev            # run with the world inspector and live asset reload
cargo editor         # run the standalone Asset Workshop
cargo run --release  # run as it ships: no inspector, much faster
```

Contributors also need the dependency auditor:

```sh
cargo install cargo-deny --locked
```

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
python3 tools/gameplay_scope.py plan --base origin/dev --head HEAD
python3 tools/gameplay_scope.py check-graph rules
python3 tools/gameplay_scope.py run clippy
python3 tools/gameplay_scope.py run rules
python3 tools/gameplay_scope.py run contracts
python3 tools/gameplay_scope.py run simulation
python3 tools/gameplay_scope.py run app
python3 tools/gameplay_scope.py run residual
cargo nextest run --workspace --all-features --cargo-profile ci --profile ci -E 'package(hex_map)'
cargo test --workspace --all-features --profile ci --doc
python3 tools/gameplay_scope.py run docs
cargo build --workspace --profile ci
```

The explicit map command mirrors its separately budgeted, world-owned CI shard; it
is part of the full local gate, not a gameplay concern command. CI runs the final
build command on Linux, Windows, and macOS. Run it on your local platform; the CI
matrix covers the other two. Markdown-only changes skip the Rust commands, but
still need valid relative links.

The gameplay commands are separated because their evidence has different authority:
pure rules, focused ECS contracts, deterministic multi-turn snapshots, and headless
game/UI behavior. Their exact Cargo package, target, and feature selections live in
`.config/gameplay-test-scopes.json`; do not duplicate or broaden them in a new
workflow. See the
[gameplay testing contract](docs/development/gameplay-testing.md) before adding a
helper, integration binary, screenshot, soak, or balance claim.

**Then run the affected application.** This is not optional, and it is not covered by
the above. Several failure modes here produce a clean log and a wrong window: missing
assets render as a plain blue screen, a sky shader that fails to load renders a black
sky, and a speed-unit mistake just looks slightly off. Every one of those passes CI.

If your change touches rendering, movement, persistence, or state transitions, walk
it: splash → title → New Game → Party Trial, orbit, move the party, **ESC** to pause,
save with **F5**, return to the title, and Continue. Open Settings, persist one change,
restart, and confirm it survived. Launch an affected Map or focused Demo separately
when the change touches one.

If it touches the Asset Workshop, run `cargo editor` and complete the relevant
[authoring workflow](docs/systems/asset-workshop.md#authoring-workflow). Persistence
changes require a save/reload round trip, recovery changes require an interrupted
dirty draft, and review changes require inspection of every source frame, the contact
sheet, and `report.ron`.

## Where code goes

See [docs/architecture.md](docs/architecture.md) for the full reasoning. The short
version:

| Adding | Goes in | Owner |
|---|---|---|
| Hex math, voxel positions, substances, shared types, states, ordering sets | `hex_core` | gameplay |
| The lattice rules engine: gems, fusions, spells, mana, disables, enchantments | `hex_lattice` | gameplay |
| Voxels, terrain, tile spawning, map settings | `hex_map` | world |
| Generic asset loading; domain schema/settings modules | `hex_assets` | loader infra gameplay; each schema and its content follow the domain owner |
| Sky, camera, presentation cutaways | `hex_world` | world |
| Rules: input, movement, interaction | `hex_units` | gameplay |
| Authoritative illumination, sight, and faction map knowledge | `hex_perception` | world |
| Runtime rendering of authored static voxel objects | `hex_objects` | shared presentation |
| A debug tool | `hex_dev` | gameplay |
| Palette, voxel-style, and object authoring workflow | `hex_editor` | shared tooling |
| A screen or menu | `hex_game` | shared |

The two roles are defined in
[docs/architecture.md](docs/architecture.md#ownership-cuts-both-ways). What crosses
between them is [docs/contracts.md](docs/contracts.md); what each is still asking of
the other is [docs/planning/boundary.md](docs/planning/boundary.md).

Ownership inside `hex_assets` follows the concern, not the directory. The gameplay
owner maintains generic loader traits, load tracking, and cross-domain reference
machinery. A world, perception, or gameplay owner may edit its own schema module,
validation, settings resource, content, and routine loader registration. Such a PR
does not need another owner merely because the schema lives in `hex_assets`; it does
need that review if it changes generic loader behavior or a cross-domain contract.

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

### Waves: how grouped work reaches `dev`

Related work is delivered in **waves** when its branches share contracts or hot files,
form a deep dependency stack, or only make sense as one runtime candidate. A
short-lived `wave/<name>` branch off `dev` collects source lanes in semantic order,
the integrated result gets a combined audit and manual walk, and **one** merge takes
the whole wave to `dev`. This keeps provisional composition off `dev` without forcing
every work lane to become a release unit.

```
feat/ticket-a ─PR─► wave/2-command-flow ─one walked PR─► dev ─promotion─► main
feat/ticket-b ─PR─►        │
```

Choose independent, stacked, or wave topology **before** opening PRs. Source branches
inside a wave are work lanes, not automatically separate PRs. Open a leaf PR only
when focused review or ownership approval is useful; otherwise integrate its
identifiable commits directly. Full CI, the automated visual walk, and the human walk
gate the combined wave. Retarget any stacked child PR *before* deleting its parent
branch; wave branches die after their one merge — `dev` never does.

The complete decision table, manifest, review budget, stale-parent reconciliation,
and cleanup rules are in
[parallel development and integration waves](docs/development/parallel-development.md).
Before declaring a PR or wave complete, also reconcile implementation,
status/design/roadmap docs, GitHub, and—when available—Linear using
[the delivery-state contract](docs/development/delivery-state.md). Linear is strongly
recommended for cross-owner visibility but never blocks a valid contribution.

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

### Long-running cross-owner work starts with contracts

A program that will span map and gameplay ownership starts with a small PR containing
documentation, shared `hex_core` vocabulary, ordering sets, and headless contract
tests. It must not change runtime behavior. Merge that PR before branching the
implementation lanes so both owners compile against one boundary.

After the contract lands:

- world PRs may change world-owned crates plus world/perception schema modules,
  settings, and content in `hex_assets`, but do not edit `hex_units` or
  `hex_combat`;
- gameplay and perception PRs consume published shared facts and do not import
  `hex_map` internals;
- changes to movement, AI, targeting, engagement, or command validation land as
  isolated adapter PRs reviewed by that crate's owner;
- a necessary shared-type change lands in its own commit or PR before code on both
  sides depends on it.

Merge updated `dev` or the wave base into a published implementation branch after a
contract change; do not rebase or force-push shared history. A wave may integrate both
owners' lanes, but it does not erase their ownership: preserve identifiable commits
and resolve cross-owner behavior through the published contract rather than one
side's private implementation.
