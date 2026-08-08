---
name: plan-ticket
description: "Reconcile an existing HEX issue with the repository, produce an implementation plan, and start it only after approval. Uses dynamic Linear lookup and never creates or switches branches inside a Conductor workspace."
---

# Plan and start existing work

Use this when the user names an existing `HEX-N`. New UI bug reports belong through
the repository's canonical `linear-ui-bug-intake` workflow, not this skill.

## Reconcile the issue

1. Fetch `origin/dev` and read root `CLAUDE.md`, relevant nested instructions,
   architecture/contracts, and the affected system/design/testing documents.
2. Fetch the issue by its human identifier with the available Linear connector. Read
   its description, comments, parent/children, relations, labels, assignee, and current
   workflow state. Validate the team by returned key/name; do not use stored UUIDs.
3. If Linear is unavailable, ask for the issue contents or continue from already
   supplied text, clearly marking ticket reads and later writes unavailable. Missing
   Linear access is coordination loss, not permission to invent issue facts.
4. Follow
   [delivery-state.md](../../../docs/development/delivery-state.md): compare the ticket
   with implementation, executable tests, status, roadmap, design/contracts, GitHub,
   and all relevant non-completed Hex Game work when accessible. Identify delivered,
   obsolete, duplicate, conflicting, and genuinely residual acceptance scope.
5. Choose independent, stacked, or wave topology before proposing lanes, using
   [parallel-development.md](../../../docs/development/parallel-development.md).
6. If the issue already belongs to a wave that is dispatching, stop and use `/inject`
   instead. Re-planning over a live wave rewrites the manifest and orders that running
   builders are reading, and it can hand two of them the same files.

## Plan

Survey affected files and prior art, then produce an approval-ready plan containing:

- outcome and user-visible acceptance criteria;
- reconciled decisions and remaining ambiguity;
- ownership/contracts and integration topology;
- file/module changes in dependency order;
- migration, compatibility, failure, and rollback behavior;
- focused and candidate validation, including presentation/human evidence only where
  applicable; and
- explicit exclusions and ticket updates required at delivery.

Ask only questions whose answer materially changes implementation. Do not edit code,
post to Linear, or change issue state before the user approves the plan.

## Start after approval

When Linear is connected, post the approved plan as a comment and move the issue to
the workflow's current `In Progress` equivalent by resolving the state dynamically
through the connector. Assign only when requested or already unambiguous.

Inside Conductor, never run `git checkout`, `git switch`, create a branch, rename the
current branch, or move the workspace to another ref. Verify the workspace branch and
base instead; if the issue belongs elsewhere, tell the user to create or select the
appropriate Conductor workspace. Outside Conductor, branch creation still requires
explicit user authorization.

Report the reconciled issue scope, approved plan link/comment, resulting state, exact
workspace branch/base, and any Linear operation that could not be performed.
