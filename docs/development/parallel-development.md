# Parallel development and integration waves

**Audience:** contributors and coding agents.

**Owner:** both owners, jointly.

Parallel work is fastest when the integration shape is chosen before branches start
to multiply. A branch is a work lane, a PR is a review unit, and a wave is a release
candidate. They do not need to be one-to-one.

The goal is not to hide individual work. It is to spend detailed review and expensive
validation at the level where the behavior is actually meaningful.

## Keep agent workflows thin

This document is the tool-neutral source of truth. Agent-specific instruction layers
should route to it instead of copying the whole process:

- Codex loads root `AGENTS.md` automatically and discovers
  `.agents/skills/plan-parallel-work` and
  `.agents/skills/land-development-wave`;
- Claude continues to use the detailed audit, test, PR, promotion, and release skills
  under `.claude/skills/`; and
- both use the same commands, contracts, ownership rules, and wave gate documented
  here and in `CONTRIBUTING.md`.

Do not port every Claude skill merely to achieve symmetry. Add a Codex skill when
Codex lacks a repeatable user-goal workflow; keep stable repository policy in shared
docs so the two tool-specific layers cannot silently diverge.

## Choose the topology before coding

| Shape | Use it when | Branch and PR shape | Validation |
|---|---|---|---|
| Independent | Each change is shippable alone, shares no unsettled contract, and has little risky file overlap | Each branch starts from `dev` and gets its own PR to `dev` | Full review and selector-chosen CI per PR |
| Stacked | One small change strictly depends on one other change and reviewing the child alone is still useful | Parent targets `dev`; child targets parent until the parent lands, then is retargeted | Focused checks per level; selector-chosen CI on each mergeable PR |
| Wave | Three or more related lanes, shared contracts or hot files, a common runtime checkpoint, or a branch stack deeper than two | One `wave/<name>` branch starts from `dev`; source lanes feed it; one wave PR targets `dev` | Focused lane checks; combined audit and selector-chosen CI, plus visual/human review only for affected presentation or experiential surfaces |

These are defaults, not arithmetic. Two branches that both change world composition
belong in a wave even though there are only two. Ten genuinely unrelated fixes should
remain independent.

Do not open a PR merely because a branch exists. Open a leaf PR when isolated review,
ownership approval, or a durable discussion is valuable. Otherwise the branch can
remain a source lane and its commits can be integrated directly into the wave. If a
leaf PR is opened, target the wave and label its purpose clearly:
`wave input — do not merge independently to dev`.

## Establish shared contracts first

Cross-owner programs begin with a small foundation change when they need new shared
vocabulary, scheduling, content readiness, or published world facts. The foundation:

- documents the contract and ownership;
- adds shared types, ordering sets, or headless contract tests;
- avoids unrelated feature behavior; and
- lands on `dev` before implementation lanes depend on it.

If both owners can already work against a live contract, no ceremonial foundation PR
is needed. Record the contract in the wave manifest and proceed.

A wave does not make ownership collective. The world owner retains world generation,
perception, and presentation authority. The gameplay owner retains movement, combat,
casting, AI, and lattice authority. Shared `hex_game` changes are integration
adapters. When two lanes implement the same concern, the integration owner pauses
composition long enough to select one shared contract instead of carrying two
implementations forward.

## Start a wave with a manifest

One integration owner creates `wave/<name>` from current `origin/dev` and records:

- the user-visible outcome and explicit exclusions;
- the source branch, owner, base, and dependency of every lane;
- contracts and hot files each lane expects to touch;
- the order in which lanes will enter the wave;
- focused checks for each lane and combined acceptance scenarios;
- unresolved cross-owner decisions; and
- the cleanup plan for source PRs and branches.

The `$plan-parallel-work` skill provides the manifest template. Store a live working
copy under `.context/waves/` or in the wave PR body. `.context/` is appropriate while
the plan is operational; the PR body becomes the durable record once the wave is
published.

Only the integration owner writes directly to the wave. Lane owners push additive
commits to their own branches. Published and shared branches are never rebased or
force-pushed.

## Integrate by intent, not by branch tip

Before taking a source branch, inspect its declared base, merge base, unique commits,
and diff. This matters most for branches created from other feature branches: merging
their tips can resurrect an obsolete snapshot of the parent after `dev` has moved.

Use a normal merge when the source branch has a clean, current base and its complete
history belongs in the candidate. When the source carries stale parent history,
transplant its intended unique commits from the correct fork point, then audit the
resulting diff. Preserve authorship and record the source PR or commit range in the
wave manifest.

Integrate in semantic order:

1. shared foundations and contract corrections;
2. owner-local foundations;
3. feature lanes;
4. composition and adapters;
5. combined fixes discovered by the wave.

Resolve a shared concern once on the wave. Push a correction back to a source branch
only when that branch remains an independently useful review unit; otherwise record
the integration fix on the wave and avoid duplicating it across every descendant.

## Review and validation budget

Review effort follows risk rather than branch count.

### Source lane

While a lane is moving, inspect its diff and run the narrowest checks that catch local
errors: formatting, the affected package's clippy/check, and focused unit or
integration tests. A leaf PR that exists still receives the repository's configured
GitHub checks, but agents do not need to create leaf PRs solely to obtain a green
badge.

### Combined checkpoint

After each semantic group enters the wave:

- inspect the aggregate diff and changed contracts;
- run affected workspace tests;
- run relevant deterministic scenario captures or a visual walk only for affected
  presentation claims; and
- test composition, regeneration, state exit/re-entry, and failure paths that no
  source lane owns alone.

### Final wave candidate

The wave PR is the merge gate. Run the exact PR-diff selector loop from
`CONTRIBUTING.md`, including only the selected test concerns and selected non-test
format/dependency/Clippy/docs/shipping gates. Unknown paths, selector changes, pushes
to protected branches, invalid configuration, and empty diffs fail closed to the
complete gate. Do not hand-edit the plan or call an omitted concern passed.

GitHub CI additionally runs that shipping-package build on the other supported
platforms and runs domain coverage. Screenshots/frames may judge static camera, UI,
and rendered-map presentation; video/human checks may judge motion, input response,
control feel, and taste. They may show how hook-established state is rendered, but
never prove gameplay or exact world logic that hooks, state, messages, logs,
snapshots, or deterministic contracts can express; add a missing hook rather than
infer logic from pixels.

For affected presentation, native-input, motion, feel, seams, composition, or taste,
run the automated visual walk, inspect every frame, and have a human play the combined
build. Record that playtest against the full final wave head SHA. A wave with no such
changed claim uses the verified-maintainer N/A classification and names its
authoritative hook closure. Any subsequent commit invalidates either classification.

Retry an apparent infrastructure failure once after confirming that no compiler,
test, lint, or application error preceded it. If the same job reaches the same hard
timeout again while every substantive gate is green, record the evidence and require
an explicit maintainer waiver. A timeout is never silently called a pass.

## Land and clean up

The wave lands on `dev` with a merge commit. Promotion from `dev` to `main` remains a
separate deliberate action.

Before the final gate, reconcile implementation, status/design/roadmap documents,
contracts, and—when available—ticket descriptions using
[delivery-state reconciliation](delivery-state.md). Documentation corrections belong
in the candidate. Linear is strongly advised for visibility but is not a merge gate.

After the wave merge:

1. confirm the merge commit and post-merge `dev` checks;
2. close source PRs as superseded, linking the wave PR;
3. retarget any still-open child PR before deleting its former base;
4. delete the wave and merged source branches only after no open PR depends on them;
5. recommend or apply ticket updates based on delivered outcomes, not on incidental
   leaf-PR state, and leave a visible warning if Linear was unavailable; and
6. leave a short reconciliation note for protected or ongoing branches that must now
   merge updated `dev`.

## Lessons from the V3/Ring7 wave

PR #138 exposed three failure modes that the wave process is designed to prevent:

- normal merges of deep branches carried stale parent snapshots, while integrating
  the commits unique to each fork preserved the intended work;
- an apparently stricter seam rule passed isolated reasoning but broke eight combined
  Ring7 tests because the composed width-two aperture had broader valid endpoints;
- repeating full platform validation across more than ten provisional PRs consumed
  time without increasing confidence in the final combined runtime state.

The lesson is not to lower the release standard. It is to apply that standard once to
the candidate that can actually ship, while keeping lane review proportional and
ownership explicit.
