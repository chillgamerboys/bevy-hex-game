---
name: update-docs
description: Audit documentation for stale test-count anchors, status claims, and index coverage. Reuses exact-head selector-chosen test evidence, updates an optional live count only when complete evidence already exists, and commits allowlisted fixes. Atomic — historical baselines and human-observed numbers are NEVER overwritten. Runs automatically as step 4 of /audit-pr.
---

When invoked, follow these steps:

1. **Check whether a live test-count anchor exists.** Do this before considering test
   counts:

   ```bash
   grep -nE 'and [0-9]+ tests' CLAUDE.md
   ```

   If this prints nothing, there is no live count task: set no `$UNIT`, run no tests,
   and continue to the status/index audit. This is the normal current state.

   If an anchor exists, reuse only complete exact-head output already produced by the
   selector-chosen `/test-full` gate. Sum its workspace + doctest `test result:` lines
   only when the selector actually ran the complete workspace/residual closure. If the
   current gate was narrow or waived, report `live count not recomputed — exact-head
   selector did not authorize a complete workspace count`, leave the anchor unchanged,
   and continue. **Never launch `cargo test --workspace` from this documentation skill
   and never broaden the test plan merely to refresh a number.** A failed selected test
   remains a test-gate failure; do not propagate its counts.

2. **Read the PR or commit context.**
   - Run `gh pr view --json number,title,body,url 2>/dev/null`
   - If a PR exists, read title and body for context about what changed.
   - If no PR, run `git log --oneline origin/dev...HEAD` instead.

3. **Understand scope.** Run
   `git diff --name-only origin/dev...HEAD` and summarize
   which areas were modified. This tells you which doc sections might
   need changes beyond just count drift.

4. **Propagate counts to Documentation Map anchors.**

   ⚠️ **HISTORICAL AND HUMAN-OBSERVED NUMBERS ARE NEVER UPDATED.**
   A count that names a specific PR, phase, date, or release is a
   point-in-time record — overwriting it destroys the trail the doc
   exists to preserve. And some numbers in this repo are *measured by
   a human at the window* (frame rate, entity counts) — a skill that
   cannot run the game must never touch them.

   ### Atomic-update allowlist

   `update-docs` **WILL touch** only:

   - CLAUDE.md `## Current state`: a live test-count clause
     ("… and NNN tests."), if one exists — update NNN to `$UNIT`.
     The current document intentionally delegates exact counts to dated foundation
     evidence, so an absent clause is valid and requires no edit.

   `update-docs` **WILL NOT touch**:

   - The FPS and entity-count figures in the same CLAUDE.md paragraph
     (human-observed at the window).
   - Any line containing `PR #N`, `as of <past date>`, "shipped
     with", "at release".
   - CHANGELOG entries, commit-message excerpts quoted in docs.
   - The root README, the stable docs under `docs/`.

   If you find a stale count outside the allowlist and want to update
   it, **STOP and ask the operator** — don't unilaterally rewrite
   history.

   Two further anchors, live since the docs restructure:

   - `docs/planning/status.md` — the designated drift doc. Verify its
     "what is built" claims against the diff. Prose corrections beyond
     a count need the operator, but **report** every claim the diff
     falsified: this is the one doc whose whole job is to be current.
   - `docs/README.md` — the index. Every tracked doc under `docs/`
     needs a row, and every row needs a file (see the drift check in
     step 6).

   Note `docs/planning/production-audit.md` is a **dated snapshot and
   must never be edited here** — its counts are historical by design.
   It carries no `and NNN tests` clause, so the anchor grep does not
   reach it; keep it that way.

5. **Audit non-count fields.** Beyond counts: if the diff added or
   removed a skill, a doc, or a workflow step that CLAUDE.md's
   "Skill pipeline" subsection describes, flag the mismatch (report
   it; fixing prose beyond the allowlist needs the operator).

6. **Drift checks.**

   **Index completeness** — every tracked doc under `docs/` has a row
   in `docs/README.md`'s index table, and every path that table names
   exists. Scope the search to the table, not the whole file: several
   docs are also linked from the "Start here" section, and matching
   those would hide a row deleted from the index itself.

   ```bash
   TABLE=$(sed -n '/^## The index/,/^## /p' docs/README.md)

   # Docs present but missing from the index table
   for f in $(git ls-files 'docs/*.md' | grep -v '^docs/README.md$'); do
       printf '%s\n' "$TABLE" | grep -q "(${f#docs/})" || echo "UNINDEXED: $f"
   done

   # Rows naming a file that no longer exists
   printf '%s\n' "$TABLE" | grep -oE '\(([a-z0-9/._-]+\.md)\)' | tr -d '()' | sort -u \
       | while read -r rel; do
           [ -e "docs/$rel" ] || echo "INDEXED BUT MISSING: docs/$rel"
       done
   ```

   A miss is reported, not auto-fixed: a new doc needs an audience and
   a purpose, and only the person who wrote it knows them.

   Add further greps here as mechanical drift surfaces are discovered
   (each check: a command, an expected value, and the doc file + label
   it updates). Candidates: scenario files named in `scenarios.ron` all
   exist; config files listed in the config doc match `assets/config/`.

7. **Commit.** If anything changed, commit with a descriptive message
   listing what was updated (Conventional subject, e.g.
   `docs: refresh test count to NNN`). If nothing was stale, no
   commit — report "All documentation is current — no changes needed."

8. **Refresh the audit-pr receipt's `head_sha`.** When step 7 commits,
   `HEAD` moves to a new sha — but the in-flight `/audit-pr` receipt at
   `/tmp/audit-pr-receipt-{PR}.json` (written by audit-pr Step 5 AFTER
   this skill returns) was generated against the pre-commit HEAD. If
   the receipt file already exists (e.g., when re-running update-docs
   standalone after a partial audit), refresh its `head_sha` field to
   `git rev-parse HEAD`:

   ```bash
   if [[ -f "/tmp/audit-pr-receipt-${PR_NUM}.json" ]]; then
       NEW_HEAD=$(git rev-parse HEAD)
       if command -v jq >/dev/null; then
           jq --arg sha "$NEW_HEAD" '.head_sha = $sha' \
               "/tmp/audit-pr-receipt-${PR_NUM}.json" > /tmp/.receipt.tmp
       else
           python3 -c '
import json, sys
p = f"/tmp/audit-pr-receipt-{sys.argv[1]}.json"
d = json.load(open(p))
d["head_sha"] = sys.argv[2]
json.dump(d, open("/tmp/.receipt.tmp", "w"), indent=2)
' "$PR_NUM" "$NEW_HEAD"
       fi
       mv /tmp/.receipt.tmp "/tmp/audit-pr-receipt-${PR_NUM}.json"
   fi
   ```

   When invoked from `/audit-pr` Step 4, this happens automatically as
   part of the orchestrator's Step 5 receipt-write (which reads the
   current HEAD). When invoked standalone (operator re-runs
   `/update-docs` to refresh a count), the receipt may already exist —
   refresh it so the operator's subsequent `/merge-pr` doesn't fail
   the SHA-match check.

9. **Report.** Summary table of what was changed (file, field, old → new)
   plus the commit SHA. If invoked from /audit-pr step 4, the gate
   continues whether or not a commit was made.

## Documentation Map

The doc-map below is the per-repo source of truth for what
`update-docs` audits. Edit this section directly when adding /
removing tracked docs.

**Active anchors:**

- `CLAUDE.md` — the optional "… and NNN tests." clause in `## Current state`.
  When present, it is maintained mechanically from `$UNIT`; its intentional absence
  is valid because exact counts currently live only in dated evidence.

- `docs/planning/status.md` — status claims (report-only beyond
  counts; it is the designated drift doc).
- `docs/README.md` — index completeness vs the `docs/` tree.

**Never touched:**

- `docs/planning/production-audit.md` — a dated snapshot; its numbers
  are a record, not a claim about now.
- `docs/planning/audit-log.md` — `/audit-diff` owns it.

## Findings shape (for audit-pr receipt v3)

When invoked from `/audit-pr`, return findings as a list shaped:

```json
{
  "file": "CLAUDE.md",
  "before": "and 226 tests",
  "after": "and 241 tests"
}
```

If nothing drifted, `findings: []` and step status `pass`.

## What this skill does NOT do

- **Update human-observed numbers.** FPS and entity counts are
  measured at the window; the skill cannot see the window.
- **Rewrite prose.** Beyond the allowlisted anchors, mismatches are
  reported, not fixed.
- **Architectural reviews.** Out of scope; reviewer's job.

## Troubleshooting

**Selected test evidence failed:** do not update counts. Surface the existing
test-gate failure; this skill must not retry or broaden it.

**Live counts diverge:** only complete exact-head selector-authorized output may update
the anchor. Narrow or waived evidence cannot manufacture a workspace total; leave the
anchor untouched and report why. Dated or historical counts remain untouched.

**Doctests:** complete pre-existing output includes their separate `test result:`
lines. Any live count therefore means "tests the selected complete suite ran", not
"#[test] fns".

**Grep false positives:** historical text ("more than 180 tests" in
an old commit message) is fine — only the CLAUDE.md anchor line is
current usage.

---

**Self-updating:** if you discover a new doc file to audit, a new
anti-pattern, or a new false-positive class, append it to the
Documentation Map before reporting.
