---
name: linear-ui-bug-intake
description: Capture reproducible player-facing UI defects in the Hex Game Linear workspace, deduplicate them against active and recently terminal issues, create verified Bug children under HEX-67, append variant or regression evidence, and upload screenshots. Use whenever an agent notices or is asked to log a UI layout, clipping, overlap, focus, accessibility, interaction, or inaccessible-control defect during runtime or visual review; do not use to implement fixes or to label a purely new aesthetic request as a bug.
---

# Linear UI Bug Intake

Create one durable Linear record per independently fixable UI defect. Consolidate
route, viewport, scale, and operating-system variants when they exhibit the same
defect. Treat Linear writes as unverified until a follow-up read proves them.

## Bound the work and authorization

- Perform intake only. Do not edit product code, create a branch or PR, assign an
  implementation agent, or move an issue to In Progress.
- Use `HEX-67` (HUD/UI bug bash) as the default parent unless the user explicitly
  names another parent.
- Treat an explicit session instruction such as “log every UI bug you find” as
  standing authorization for issue creation, evidence comments, and screenshot
  attachments. Do not reconfirm each ticket while that authorization remains in
  force. Without standing authorization, present the proposed write and obtain
  approval once before mutating Linear.
- Never let standing authorization expand beyond intake. Do not delete issues or
  comments, alter unrelated fields, or reopen a terminal issue without separate
  authorization.
- If Linear is unavailable, stop before mutation. Return the complete ticket draft
  and a visible warning; never claim it was logged.

Classify clipping, overlap, broken reflow, illegible contrast, missing labels,
incorrect focus order, inaccessible controls, and broken UI interaction as Bugs.
Classify a request for a new look or behavior with no violated current contract as an
Improvement and keep it out of this Bug workflow unless the user explicitly expands
scope.

## 1. Capture the observation before searching

Establish a compact defect signature from the component, trigger, route or phase,
and symptom. Describe what is visible or directly experienced; do not guess at a
code-level root cause.

Record every field below. Resolve missing environment facts instead of substituting
“latest,” “default,” or an assumed value.

- **Exact build SHA:** the full 40-character commit that produced the running
  binary. Use `git rev-parse --verify HEAD` only when that HEAD actually produced
  the observed build. Record whether the worktree was clean; if it was dirty, say so
  and include a concise changed-path summary because the SHA alone is incomplete.
- **Route/state:** the screen and nested route used to reach the defect.
- **Phase:** for example Deployment, active Gameplay, paused Gameplay, or terminal
  outcome. Keep this separate from route.
- **Viewport:** logical width by height. Also record output pixel dimensions when
  device scaling makes them different.
- **Scale:** in-game UI scale mode/value and operating-system device scale.
- **OS:** exact platform and version, obtained from the platform rather than memory.
- **Reproduction:** minimal numbered steps from a stable entry point.
- **Observed:** one precise symptom, including frequency if intermittent.
- **Expected:** the current contract or ordinary usable behavior being violated.
- **Impact:** affected task and whether a workaround exists. Mark **Blocker** only
  when progress is impossible or a required flow/control is inaccessible.
- **Evidence:** local screenshot/video path, existing durable link, log excerpt, or
  human interaction observation. A static screenshot proves presentation only; it
  does not prove gameplay state, timing, motion, or input behavior.
- **Acceptance:** observable, testable outcomes across the reproducing route and
  relevant viewport/scale variants. State the result, not an implementation.

For intermittent defects, include attempts and hits, such as “3 of 10 entries.” For
an interaction or state claim, use a typed hook/log when one exists and use visual
evidence only for how that established state is presented.

## 2. Resolve live Linear identities

Resolve identities during every intake run. Do not copy UUIDs from older skills,
prior tickets, or this repository.

1. List teams using the query `Hex Game`; select exactly one active exact-name match
   and fetch it. Verify its name is `Hex Game` and its key is `HEX` when the key is
   returned. Stop on zero or ambiguous matches.
2. Fetch the parent by identifier (`HEX-67` by default) with relations. Verify that
   it belongs to the resolved team. Stop rather than parenting across teams.
3. List issue statuses for the resolved team. Resolve the single status named
   `Backlog` whose type is `backlog`. Do not pass a cached status ID.
4. Paginate team issue labels and resolve the single exact label named `Bug`. Do not
   create or guess a missing label.
5. Resolve `me`; verify the user is active and belongs to the resolved team. Use the
   returned user identity for the assignee and later verification.

Also retain the live status list for duplicate searching. Treat `completed`,
`canceled`, and `duplicate` status types as terminal; all other live types are open.

## 3. Search for duplicates before every write

Search beyond exact titles. Run the following searches with two or three concise
query variants derived from the defect signature (component + symptom, route +
symptom, and distinctive trigger when useful):

1. Paginate all children of the resolved parent, including terminal children.
2. For every live open status, search the resolved Hex Game team without a date
   cutoff.
3. For every live terminal status, search the team with `updatedAt: "-P90D"`.

Request enough fields to compare title, description, status/type, labels, parent,
updated time, and assignee. Paginate rather than assuming the first page is complete.
Re-fetch each plausible candidate with relations, then read its comments and
attachments. If a candidate has terminal type `duplicate`, follow `duplicateOf` and
evaluate the canonical issue.

Call it a duplicate only when it represents the same independently fixable defect:
the same control/component and trigger produce the same violated behavior. Different
viewport, scale, OS, frequency, or route evidence is normally a variant to append.
Sharing a panel or visual area alone is not enough. If the match is materially
ambiguous, create a separate issue rather than silently merging distinct failures.

## 4. Append evidence or create one child Bug

### Duplicate found

Re-fetch the canonical issue immediately before writing. Append a new comment; do
not rewrite historical evidence. Use this shape:

```markdown
## Additional evidence — <UTC timestamp>

- Build SHA: `<40-character SHA>`
- Worktree: Clean | Dirty — <changed-path summary>
- Route/state: <route>
- Phase: <phase>
- Viewport: <logical WxH> [; output <pixel WxH>]
- UI/device scale: <values>
- OS: <platform and version>
- Reproduction: <numbered steps or concise link to them>
- Observed: <symptom and frequency>
- Expected: <contract or usable behavior>
- Impact: <task/workaround/blocker status>
- Evidence: <attachment filename or durable link>

### Acceptance additions

- [ ] <testable outcome introduced by this variant>
```

Do not change a terminal issue's state under intake-only authorization. Report that
terminal state explicitly so the user can choose whether a regression should reopen.

### No duplicate found

Create one issue titled `[UI] <component>: <concise violated behavior>` with:

- team: the resolved Hex Game team;
- parent: the fetched parent ID exactly as accepted by the connector;
- state: the resolved Backlog status ID;
- label: only the resolved Bug label unless the user requested more;
- assignee: the resolved current user; and
- priority: omit it, except use Urgent (`1`) for a confirmed Blocker.

Do not set priority merely because a bug is noticeable. Omit the priority field for
ordinary defects and verify Linear reports No priority.

Use this description:

```markdown
## Build and environment

- Build SHA: `<40-character SHA>`
- Worktree: Clean | Dirty — <changed-path summary>
- Route/state: <route>
- Phase: <phase>
- Viewport: <logical WxH> [; output <pixel WxH>]
- UI/device scale: <values>
- OS: <platform and version>

## Reproduction

1. <start at a stable route>
2. <minimal action>
3. <trigger>

## Observed

<precise symptom and frequency>

## Expected

<current contract or ordinary usable behavior>

## Impact

<affected task, workaround, and Blocker/Non-blocker>

## Evidence

- <attachment filename, durable link, hook/log, or human observation>

## Acceptance criteria

- [ ] <observable result on the reproducing route>
- [ ] <relevant viewport/scale coverage>
- [ ] <interaction, focus, or labeling result when applicable>
```

Keep literal newlines in Linear Markdown; do not send escaped `\n` sequences.

## 5. Upload every local screenshot with prepare → PUT → finalize

Attach local screenshots to the canonical issue, whether newly created or deduped.
Process one file completely before preparing another:

1. Resolve the real path, safe filename, MIME type, and exact byte size. Confirm the
   file is the intended evidence; never upload source, secrets, or unrelated desktop
   content.
2. Call `prepare_attachment_upload` with issue, filename, content type, size, and a
   descriptive title/subtitle.
3. Immediately PUT the raw file bytes to `uploadRequest.url`. Send every returned
   header verbatim, preserving header names, values, and casing. Do not base64-encode
   or transform the bytes. Do not print the signed URL or headers. Require a 2xx
   response; if the URL expires or the PUT fails, prepare a new upload rather than
   finalizing the old one.
4. Call `create_attachment_from_upload` using the returned `assetUrl` and the same
   issue. Finalize only after the PUT succeeds.

Do not batch prepared URLs; they expire quickly. Do not use the deprecated base64
attachment path as a silent fallback.

## 6. Re-fetch and verify every mutation

Never infer success from a mutation response. Perform the corresponding read
immediately after each write and before the next mutation.

- After issue creation or field update, fetch the issue and assert exact title,
  team, parent, Backlog state, Bug label, assignee, description, and priority.
- After a comment write, list comments and assert the returned comment ID and exact
  body are present.
- After attachment finalization, fetch the issue and assert the attachment ID or
  asset URL, filename/title, and issue association. Fetch the attachment directly
  when that operation is available.
- After any corrective retry, re-fetch again. On a persistent mismatch, stop and
  report the actual returned state; do not describe the write as successful.

Verification applies equally to new issues and duplicate evidence. Never delete a
partially created record to hide a failure; report it so it can be reconciled.

## Report

Return the issue identifier, title, URL, whether it was created or deduplicated, its
verified parent/state/label/assignee/priority, uploaded evidence names, and the exact
build SHA. Name any missing evidence, terminal duplicate, ambiguous classification,
or failed verification. End after intake; implementation belongs to a separately
planned bug-bash lane or wave.
