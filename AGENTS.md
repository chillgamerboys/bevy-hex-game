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

Once the answer is a wave, use `$plan-epic` to decompose it into lanes with disjoint
ownership, sequence them with separate `dispatch_blockers` and `merge_blockers`, and commit
the manifest under `docs/planning/waves/<slug>/`. That covers three or more related lanes,
several branches touching the same contracts or files, and a feature set whose review only
means something combined.

The canonical rules and decision table are in
`docs/development/parallel-development.md`; the wave artifact, lane field table, ownership
algebra, merge order, and the recipes for reconciling pre-existing branches are in
`docs/development/wave-protocol.md`.

Executing a wave with parallel agents is Claude-side (`/dispatch`, `/inject`) because it
needs harness worktree isolation. A Codex coordinator plans with `$plan-epic` and lands
lanes by hand, following the same merge order.

## Keep delivery state reconciled

Strongly prefer `$reconcile-delivery-state` before calling a PR or wave complete,
after it lands on `dev`, and whenever roadmap/design work relies on existing Linear
tickets. Code, status/design/roadmap docs, GitHub, and Linear are projections of one
delivery; compare them rather than trusting a branch name or ticket state.

For broad planning and architecture passes, inspect every non-completed Hex Game
ticket when Linear is connected, not only tickets referenced by the current branch.
Before merge, include documentation corrections in the candidate. After merge, verify
the exact `dev` SHA, then recommend or apply ticket updates for delivered, partial,
and obsolete work.

Linear is strongly advised for coordination but is a soft signal. Missing access or
an owner who does not use Linear must never block a valid PR. Emit a visible warning
and put the exact recommended ticket updates in the handoff. The canonical workflow
and optional Codex setup are in
`docs/development/delivery-state.md`.

## Capture UI bugs before fixing them

Use `$linear-ui-bug-intake` for a reproduced UI, interaction, accessibility, focus,
clipping, overlap, or presentation defect. Search Linear before writing, keep one
ticket per independently fixable root cause, attach durable evidence, and verify the
saved issue. During the current HUD review, new Bugs are children of `HEX-67`.

Bug intake records evidence and acceptance criteria; it does not implement the fix.
Use `$plan-parallel-work` after the ticket set is concrete, grouping lanes by component
and hot-file overlap rather than opening one PR per ticket.

## Ownership and shared concerns

- The world owner controls `hex_map`, `hex_world`, `hex_perception`, and their
  domain content and schema. The gameplay owner controls `hex_core`, `hex_units`,
  `hex_combat`, `hex_lattice`, `hex_anim`, and generic asset-loader infrastructure.
- `hex_game` is shared integration; `hex_objects` and `hex_editor` are shared
  presentation/tooling without gameplay authority. `hex_ui` is shared runtime
  presentation behind a strict dependency ceiling: Bevy, `hex_core`, `hex_assets`,
  `hex_gameplay_model`, and serialization support only.
- A wave may contain both owners' work, but it does not erase ownership. Keep
  owner-specific changes in identifiable commits and treat cross-owner behavior
  changes as explicit decisions.
- A wave *lane* sits inside exactly one crate authority. A lane whose declared ownership
  crosses the world/gameplay boundary is a stop condition, not a review comment: re-cut the
  seam, or land a behavior-neutral foundation on `dev` first.
- When two lanes solve the same concern, consolidate on one shared contract in the
  foundation or wave. Do not preserve parallel implementations merely to keep
  branches independent.

## Branch and review safety

- Everything lands on `dev`; only a deliberate promotion moves `dev` to `main`.
- Lane PRs target `wave/*`; the wave PR targets `dev`; nothing inside a wave loop ever
  targets `main`.
- One integration owner writes to a shared wave branch. Other contributors own
  their source branches.
- Do not rebase or force-push a published or shared branch. Reconcile with additive
  commits.
- Do not blindly merge a stacked branch tip when it carries obsolete parent state.
  Inspect its merge base and transplant or merge only the intended unique work.
- Keep source branches until the wave lands. Retarget open child PRs before deleting
  any branch they use as a base.
- Run focused checks while a lane is changing. Run the complete selector-chosen
  CI-equivalent gate on the combined wave candidate. Run the automated visual walk
  and human runtime route only when the candidate affects presentation or experience;
  a logic-only candidate records the exact-head hook-backed classification instead.

## Code review rules

- Block violations of the gameplay/world boundary and facts reconstructed from
  another owner's private implementation.
- Review the combined behavior of related branches. A leaf test that passes is not
  evidence that composition, regeneration, state re-entry, or presentation works.
- Treat an exact infrastructure timeout differently from a compiler, test, or lint
  failure, but never silently convert it to a pass. Record retries and any maintainer
  waiver on the wave PR.

## Evidence boundary

- Screenshots and rendered frames are valid evidence for static presentation: camera
  framing and occlusion, UI hierarchy/layout/legibility/focus/contrast/reflow, and a
  rendered map's visible geometry, materials, lighting, cutaways, seams, and
  composition. They may show how a state already established by hooks is rendered;
  they do not establish the underlying state.
- Video and human checks are valid for motion and experience, including camera motion,
  native input, animation, control feel, and taste. A static screenshot cannot prove
  motion or control feel.
- Screenshots, rendered frames, video, and human observation must never be used to
  prove or corroborate gameplay or exact world logic when that claim can be observed
  through typed hooks, components, resources, messages, logs, canonical snapshots, or
  deterministic contracts.
- If an authoritative logic hook is missing, add the narrow hook or contract. Do not
  infer legality, occupancy, payment, damage, settlement, turn release, persistence,
  determinism, or any other state transition from pixels or frame timing.
- Presentation and experiential evidence does not duplicate, strengthen, or
  substitute for logical evidence.
