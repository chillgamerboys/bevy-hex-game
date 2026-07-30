# Codex working agreements

Read `CLAUDE.md` before changing this repository. It is the operational summary for
architecture, commands, ownership, and Bevy-specific traps. When working under a
directory with its own `CLAUDE.md`, read that file too. Treat
`docs/architecture.md`, `docs/contracts.md`, and the relevant system or design doc
as contracts rather than background reading.

## Plan the integration shape first

Before splitting one outcome across branches, agents, or PRs, use
`$plan-parallel-work`. Classify the work as independent, stacked, or a wave before
implementation begins. Do not open one PR per subtask by default.

Use `$land-development-wave` when reconciling three or more related branches or PRs,
when several branches touch the same contracts or files, or when only the combined
runtime state is meaningful to review.

The canonical rules and decision table are in
`docs/development/parallel-development.md`.

## Ownership and shared concerns

- The world owner controls `hex_map`, `hex_world`, `hex_perception`, and their
  domain content and schema. The gameplay owner controls `hex_core`, `hex_units`,
  `hex_combat`, `hex_lattice`, `hex_anim`, and generic asset-loader infrastructure.
- `hex_game` is shared integration; `hex_objects` and `hex_editor` are shared
  presentation/tooling without gameplay authority.
- A wave may contain both owners' work, but it does not erase ownership. Keep
  owner-specific changes in identifiable commits and treat cross-owner behavior
  changes as explicit decisions.
- When two lanes solve the same concern, consolidate on one shared contract in the
  foundation or wave. Do not preserve parallel implementations merely to keep
  branches independent.

## Branch and review safety

- Everything lands on `dev`; only a deliberate promotion moves `dev` to `main`.
- One integration owner writes to a shared wave branch. Other contributors own
  their source branches.
- Do not rebase or force-push a published or shared branch. Reconcile with additive
  commits.
- Do not blindly merge a stacked branch tip when it carries obsolete parent state.
  Inspect its merge base and transplant or merge only the intended unique work.
- Keep source branches until the wave lands. Retarget open child PRs before deleting
  any branch they use as a base.
- Run focused checks while a lane is changing. Run the complete CI-equivalent suite
  and automated visual walk on the combined wave candidate; the human runtime walk
  remains the final visual gate.

## Code review rules

- Block violations of the gameplay/world boundary and facts reconstructed from
  another owner's private implementation.
- Review the combined behavior of related branches. A leaf test that passes is not
  evidence that composition, regeneration, state re-entry, or presentation works.
- Treat an exact infrastructure timeout differently from a compiler, test, or lint
  failure, but never silently convert it to a pass. Record retries and any maintainer
  waiver on the wave PR.
