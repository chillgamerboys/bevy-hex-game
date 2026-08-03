---
name: test-quick
description: "Run the focused iteration gate: format plus the fail-closed selector's affected Clippy and test concerns for the current diff. Use while implementing; /test-full owns merge-candidate validation."
---

# Focused validation

Use the exact PR base when one exists; otherwise use `origin/dev`. For a source lane
targeting a wave, never substitute `dev` for its actual base.

1. Fetch the base and run the scope planner using the exact argument construction in
   [`CONTRIBUTING.md`](../../../CONTRIBUTING.md#before-opening-a-pr).
2. Record the complete plan. Unknown paths, invalid configuration, and empty diffs
   fail closed; do not hand-narrow or broaden the result.
3. For a Rust-affecting diff, run `cargo fmt --all --check`.
4. Run `python3 tools/test_scope.py run clippy` only when selected by the plan.
5. Run every value returned by `selected-tests` in order. If `residual` is selected,
   also run the workspace doctest command from `CONTRIBUTING.md`.
6. Stop on the first failure and preserve its output.

This tier deliberately omits dependency policy, documentation, link, shipping, and
visual/human gates. Markdown-only work reports that no Rust concerns were selected;
it does not claim the candidate gate is green.

Report the base, changed-file classification, selected concerns, commands actually
run, elapsed time, and the first failure or all-green result. Do not describe an
unselected concern as executed or passed.
