---
name: update-linear
description: "Link an existing HEX issue to a GitHub PR, apply an explicitly requested workflow transition, or delete a fully delivered issue under the repository retention policy. Never creates issues; UI bug creation belongs to linear-ui-bug-intake."
---

# Link a PR and update existing work

This skill has three narrow responsibilities: idempotent PR linkage, explicit state
transition, and policy-governed deletion for an existing issue. It never searches for work
to create and never turns a PR merge into evidence that a larger epic is complete.

## Inputs and lookup

Require a PR number/current PR and an explicit `HEX-N`, or one unambiguous `HEX-N`
already present in the branch/PR body. If multiple or conflicting identifiers exist,
stop and ask which relationship is authoritative.

Use the available Linear connector's current schema. Fetch by human identifier,
validate the returned Hex Game team key/name, and discover workflow states dynamically.
Never store or copy team, user, label, project, or workflow UUIDs into this skill.
If the connector is unavailable, leave GitHub and Linear unchanged and report the
exact recommended operations; Linear remains a soft coordination dependency.

## Link mode

1. Fetch the issue and PR, including current relations/attachments/comments and the
   PR body.
2. Add or normalize one stable PR-body section without changing unrelated prose:

   ```text
   ## Linear

   - HEX-N — issue title — canonical issue URL
   ```

   Render the actual entry as a normal Markdown link using the fetched HTTPS URL; the
   placeholder above intentionally avoids looking like a repository-relative link.

3. Add the PR URL to the issue using the connector's supported attachment/link
   relation. If it has no relation operation, add one concise comment instead. Check
   existing relations and comments first so retries never duplicate it.
4. Re-fetch both sides and verify the identifier and URLs round-trip.

Do not replace a valid different link without explicit user direction. Do not create a
ticket as a fallback. Initial UI observations use the repository's canonical
`linear-ui-bug-intake` workflow. Wave lanes are keyed by their manifest lane ids and reuse
existing tickets only when they add coordination value.

## Transition mode

Apply a state transition only when the user or a lifecycle skill explicitly requests
it. Resolve the requested semantic state against the team's live workflow by returned
name/type and use the connector exactly as currently declared; do not assume an
undocumented endpoint or parameter.

- **In Progress:** approved implementation has started.
- **In Review:** the relevant review candidate is open and the issue's acceptance
  scope is actually represented by that candidate.
- **Done:** the issue's full promised outcome is verified on `origin/dev`.

Before `Done`, follow
[delivery-state.md](../../../docs/development/delivery-state.md): verify the exact merge
on `dev`, executable behavior, docs, and residual acceptance scope. A leaf PR entering
a wave, a partial epic, or one fixed symptom of a broader bug stays non-terminal.
Rewrite residual scope only with explicit authority and preserve useful history.

After any transition, re-fetch the issue and verify the returned workflow state. A
failed or unverifiable write is a warning that must be surfaced, never reported as
success and never used to block an otherwise valid code merge.

## Delete mode

Delete only when the user or a lifecycle skill explicitly requests retirement under the
free-workspace policy in
[delivery-state.md](../../../docs/development/delivery-state.md). Re-fetch the issue and
verify every precondition there: complete delivery on `origin/dev`, a durable record of the
identifier/title/outcome/exact SHA/PR, no residual scope or active descendants, and no
other-owner or regression-history reason to retain it. Never delete the current UI bug-bash
parent, a partial epic, an active duplicate target, or an issue owned by the other owner.

Use the connector's declared deletion operation; do not invent an endpoint. Verify the
returned deletion state or confirm an immediate lookup returns an explicit not-found result;
a timeout or connector error is not deletion evidence. If deletion is unavailable, report
the exact manual action. Do not substitute `Done` and claim that the free-plan issue budget
was reclaimed.

## Report

Report the issue and PR URLs, prior and resulting workflow state or verified deletion,
linkage changes, durable record location, verification result, and any recommended operation
left unapplied.
