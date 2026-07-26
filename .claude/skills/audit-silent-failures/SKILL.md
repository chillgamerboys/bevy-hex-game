---
name: audit-silent-failures
description: Grep for the seven Rust/Bevy silent-failure anti-patterns — error-swallowing this repo's clippy config cannot see. Reports candidates with inline known-false-positive context so reviewers don't re-derive "is this real?" Audit-only — does not auto-fix.
---

When invoked, follow these steps:

1. **Run the seven canonical greps** below. Capture per-pattern total
   counts and the first ~5 examples (with line numbers) for each.

2. **Apply the false-positive guards** from the table inline. For every
   match, decide: "investigate" or "likely-FP". The guards are heuristics
   — when in doubt, mark "investigate" and let a human judge.

3. **Print a summary table** (pattern × total × likely-FP × investigate)
   followed by the actual lines under each "investigate" bucket. Do
   NOT auto-fix; each pattern needs human judgment about whether the
   suppression is intentional.

4. **No commit.** This skill is diagnostic only. Fixes go in a separate
   PR, scoped to whichever pattern the reviewer chose to address.

## What clippy already covers (don't duplicate it)

This repo's clippy config **denies** `unwrap_used`, `panic`, `todo`,
slice indexing, `dbg_macro`, `print_stdout`/`print_stderr`,
`let_underscore_must_use`, `map_err_ignore`, and float `==` in
non-test code, and bans `#[allow]` outright. This audit hunts what
those lints *cannot* see: code that handles an error by quietly
making it disappear. Tests may unwrap/expect/panic by design — matches
in `#[cfg(test)]` modules or `crates/*/tests/` are likely-FP for
panic-shaped patterns, but **not** for pattern 5 (`#[ignore]`).

## Patterns and false-positive guards

All greps run over `crates/*/src` with `--include='*.rs'` unless noted.
BSD grep on macOS: always use `-E`, never `\|` alternation without it.

### 1. `unwrap_or_default()` / `unwrap_or(...)` swallowing errors

**Concern:** a failed parse/load/lookup silently becomes a default
value. This repo's convention is the opposite: a bad RON file must
stall loading loudly, and a failed hot reload keeps the last valid
value *and reports the error*.

**Grep:**
```bash
grep -rnE '\.unwrap_or_default\(\)|\.unwrap_or\(' crates/*/src --include='*.rs'
```

**FP guards:**
- On an `Option` where empty genuinely means the default (e.g.
  `.get(k).copied().unwrap_or(0)` for a lookup with a documented
  zero-default) → likely-FP.
- On a `Result` from parsing, IO, or asset loading → **investigate**.
  Defaulting here is exactly the "diverges silently from what someone
  wrote" failure the settings pipeline was built to prevent.
- In test code → likely-FP.

### 2. Bare `.ok()` discarding a `Result`

**Concern:** converts an error into `None` and drops it on the floor.
`map_err_ignore` doesn't catch this shape.

**Grep:**
```bash
grep -rnE '\.ok\(\);' crates/*/src --include='*.rs'
grep -rnE 'let _ *= *[^=]*\.ok\(\)' crates/*/src --include='*.rs'
```

**FP guards:**
- `.ok()?` and `.ok().map(...)` / `.ok().and_then(...)` chains
  propagate the absence — excluded by the `;` anchor, but if one
  slips through → likely-FP.
- A bare `something.ok();` statement → **investigate**: if the
  failure doesn't matter, the code should say why in a comment; if it
  does, it should be handled or logged.

### 3. Empty or bail-out `Err` arms

**Concern:** an error arm that does nothing, or exits the loop/function
without recording that anything went wrong.

**Grep:**
```bash
grep -rnE 'Err\(_\) *=> *(\{ *\}|\(\)|continue|return|None)' crates/*/src --include='*.rs'
grep -rnE 'if let Ok\(' crates/*/src --include='*.rs'
```

The second grep needs a manual pass: for each `if let Ok(...)`, is
there an `else`, and is silently skipping the `Err` case correct on
this path?

**FP guards:**
- The arm sits beside a sibling arm that logs, or a comment says
  "best-effort" with a reason → likely-FP.
- `map_err_ignore` is denied in this repo, so an `Err(_)` binding
  that survived clippy is already unusual → default to
  **investigate**.

### 4. Log-and-continue on startup paths

**Concern:** logging an error and carrying on is the documented
*hot-reload* contract (retain last valid value, report). On an
*initial-load or startup* path the same shape is a bug — the game
must stall loudly, not limp forward with missing state.

**Grep (then inspect context):**
```bash
grep -rnB2 -E '(warn|error)!\(' crates/*/src --include='*.rs' | grep -E 'Err\(|-B2|--'
```

Practical form: find `Err(e) => {` arms containing `warn!`/`error!`
and check whether the error propagates after logging.

**FP guards:**
- Hot-reload / `AssetEvent` handlers → likely-FP (documented
  contract).
- `OnEnter(Screen::Loading)` / initial-parse paths → **investigate**:
  the loading screen exists to block until every settings file parsed.

### 5. `#[ignore]` without a reason; `#[expect]` with a vacuous one

**Concern:** disabled tests and suppressed lints with no recorded
justification. Applies to test code too — this is the one pattern
where tests get no pass.

**Grep (all of `crates/`, tests included):**
```bash
grep -rnE '#\[ignore\][[:space:]]*$' crates/ --include='*.rs'
grep -rnE '#\[expect\([^)]*reason *= *"(|TODO|temp|fixme|for now)"' crates/ --include='*.rs'
```

**FP guards:**
- `#[ignore = "specific reason"]` with a real condition → likely-FP.
- Bare `#[ignore]` → **investigate**, always.
- `#[expect]` whose reason states a genuine invariant → likely-FP
  (that is the repo's sanctioned mechanism); a reason that restates
  the lint name or says "for now" → investigate.

### 6. Weak `expect` messages

**Concern:** `expect_used` is allowed where `unwrap_used` is not, on
the theory that the message documents the invariant. An empty or
operation-restating message defeats that theory.

**Grep:**
```bash
grep -rnE '\.expect\(""\)|\.expect\("[^"]{1,12}"\)' crates/*/src --include='*.rs'
```

**FP guards:**
- Message states the *invariant* ("mesh guaranteed by build step",
  "registered in plugin") → likely-FP even if short.
- Message restates the operation ("failed", "parse error") →
  **investigate**: it will panic without telling anyone why the
  invariant was supposed to hold.

### 7. `|| true` in CI and scripts

**Concern:** suppresses arbitrary command failures in automation.

**Grep:**
```bash
grep -nrE '\|\| true\b' .github/workflows/ 2>/dev/null
```

(Extend to `scripts/` if that directory ever appears.)

**FP guards:**
- `git fetch … || true` in ci.yaml's `changes` job → likely-FP,
  **known and deliberate**: the job fails safe to `code=true`, running
  the full pipeline when the diff can't be computed.
- The preceding command is idempotent cleanup (`rm`, `mkdir -p`) →
  likely-FP.
- Otherwise → investigate.

## Output format

Print a summary table, then per-pattern detail under "investigate":

```
=== Silent-Failure Audit Report ===

| Pattern | Total | Likely-FP | Investigate |
|---|---|---|---|
| 1. unwrap_or on Result | N | N | N |
| 2. bare .ok() discard | N | N | N |
| 3. empty Err arms | N | N | N |
| 4. log-and-continue (startup) | N | N | N |
| 5. bare #[ignore] / vacuous #[expect] | N | N | N |
| 6. weak expect messages | N | N | N |
| 7. || true (CI) | N | N | N |

=== Investigate (Pattern 1) ===
  crates/hex_assets/src/settings.rs:42 — .unwrap_or_default() on a RON
                       parse result (initial load path)
  ...
```

If a pattern has **0 in the Investigate column**, that's the goal — say
so explicitly so reviewers know the audit cleared it. Don't bury good
news.

## Findings shape (for audit-pr receipt v3)

When invoked from `/audit-pr`, return findings as a list shaped:

```json
{
  "pattern": "1_unwrap_or_result",
  "file": "crates/hex_assets/src/settings.rs",
  "line": 42,
  "snippet": "ron::from_str(&raw).unwrap_or_default()",
  "classification": "investigate"
}
```

`/audit-pr` step 3 includes only `classification: investigate` rows
in the receipt's `findings` array. Likely-FP candidates stay in the
human-readable report but don't propagate to the gate.

## Troubleshooting

**Grep matches inside the SKILL.md files themselves:** this skill
documents the patterns by quoting them. `grep` against `.claude/skills/`
will hit the documentation. Filter with `--exclude-dir=.claude`.

**False-positive guards drift:** the FP heuristics codify what was OK
at the time of this port. New idioms (e.g., a new sanctioned
best-effort helper) should be added to the relevant pattern's FP-guard
list. Append rather than replace — the original guards still help
future reviewers.

**Skill produces a flood of matches:** that means the codebase has new
silent-failure patterns. The skill is doing its job; the noise *is* the
finding.

---

**Self-updating:** if you encounter a new false-positive class
(canonical idiom that the skill keeps flagging), append it to the
relevant pattern's FP-guard list before reporting. If you encounter a
genuinely new anti-pattern not in the seven, propose it as a new section
to the user.
