---
name: create-pr
description: "Open or update a PR from the current Conductor branch using the repository template, correct dev/wave base, and optional existing Linear issue linkage. Never creates or switches branches."
---

# Create a pull request

## Preflight

1. Read [parallel-development.md](../../../docs/development/parallel-development.md)
   and resolve the agreed independent, stacked, or wave base. Default independent work
   to `dev`; source lanes use their declared `wave/*` base.
2. Fetch the base and inspect `git status`, current branch, commits, and
   `git diff --stat origin/<base>...HEAD`. Refuse `dev`, `main`, an empty range,
   unresolved conflicts, or uncommitted candidate changes.
3. In Conductor, use the current branch exactly as provided. Never run checkout,
   switch, branch creation, or branch rename. If the workspace is on the wrong branch,
   stop and ask the user to select/create the correct workspace.
4. If a PR already exists for the branch, update/report it instead of creating a
   duplicate.

## Open the PR

Push the current branch normally, without force. Derive a Conventional-Commit title
from the delivered outcome unless the user supplied one. Populate the canonical
[pull request template](../../../.github/pull_request_template.md) from the actual diff:

- explain why, not a file-list paraphrase;
- record checks already run and leave unrun boxes unchecked;
- preserve all evidence-boundary and manual-runtime fields;
- name ownership/shared-boundary changes;
- preserve the selector's distinction between selected and unselected concerns; and
- leave exact-head runtime fields pending on a draft instead of inventing evidence.

Create with `gh pr create --base <resolved-base> --head <current-branch>`. Use `--draft`
when implementation or required evidence remains; otherwise open ready only when the
user requested review and the candidate is actually ready.

## Existing Linear issue

If the branch, user, or plan supplies one unambiguous existing `HEX-N`, invoke
`/update-linear` in link mode and optionally move it to the live `In Review`
equivalent. A failed or unavailable Linear operation is a visible warning, never a PR
creation failure. Do not create an issue here; new UI observations use
`linear-ui-bug-intake`.

Report the PR URL, head/base, draft state, linked issue or warning, and remaining gates.
