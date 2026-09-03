---
name: recover-cargo-capacity
description: Diagnose and safely recover disk or memory capacity in this Rust/Cargo workspace while preserving dirty worktrees, source, captures, branch history, and reproducible evidence. Use when builds fail for lack of space, several worktrees have large target directories, duplicate Cargo jobs are competing, or a heavy Bevy build must be made sustainable without weakening tests.
---

# Recover Cargo Capacity

Recover only reconstructible build state, and prove the original gate afterward. This
skill does not authorize deleting source worktrees or broad filesystem cleanup.

Read the
[storage cases](../../../docs/development/problem-solving-casebook.md) before acting.

## 1. Audit without changing state

Run the bundled [read-only audit helper](scripts/audit_cargo_storage.py) from the repository
root:

```sh
python3 .agents/skills/recover-cargo-capacity/scripts/audit_cargo_storage.py
```

The script is read-only. Review free space, running Cargo/rustc processes, every Git
worktree's branch/HEAD/dirty state/upstream divergence, and each reported target/profile
size and cache tag. Also inspect `git status` yourself in every cleanup candidate. Its
`manual-validation-required` classification is evidence for review, never cleanup
authorization or proof that a path is exclusively Cargo-owned.

The audit reports ignored paths inside a discovered Cargo target separately from ignored
paths elsewhere. A target-contained ignore does not protect an otherwise clean owner by
itself; any ignored path outside those normalized target boundaries remains possible review
evidence and protects the worktree.

Classify the failure before cleaning:

- **Disk capacity:** target profiles or duplicate worktrees consume the filesystem.
- **Memory pressure:** concurrent rustc/linker/game processes exceed RAM or swap.
- **Environment:** wrong target directory, toolchain, or launch path.
- **Code/test:** the original compiler or test failure persists with adequate capacity.

Cleaning cannot turn the last two classes into a pass.

## 2. Stop duplicate pressure first

Identify running build, linker, test, and game processes. Do not kill unrelated work.
Coordinate with their owning task; stop only duplicate or explicitly abandoned jobs. Then
serialize expensive Bevy builds, reuse one intended target profile, and disable incremental
compilation for constrained one-off release gates when appropriate.

Never launch several cold full-workspace builds merely to see which finishes first.

## 3. Select exact reconstructible targets

Use this priority order:

1. Obsolete debug or release profiles in an explicitly identified Cargo target directory.
2. Target directories belonging to finished, clean worktrees whose branch/PR state has
   been verified.
3. Package-scoped Cargo artifacts when a narrower `cargo clean -p ...` is sufficient.
4. A whole exact target directory only when it is cache-tagged, no process uses it, and
   rebuilding it is acceptable.

Before any destructive command, state the absolute target, measured size, why it is
reconstructible, the owning worktree, its dirty/untracked count, upstream/PR disposition,
and the expected next build. Resolve variables and globs to explicit paths first. Obtain
explicit user authorization for those exact candidates, then rerun the audit and refuse
the action if the path, device, inode, cache tag, size/mtime snapshot, or active-process
state drifted.

Prefer Cargo's cleanup command with an explicit manifest or target directory. Do not use a
broad recursive removal command, a home/workspace root, or an unvalidated environment
variable as the target.

Never use `git clean`, wildcard deletion, or whole-worktree removal as a Cargo-capacity
operation. Prefer stopping duplicate builds or relocating the next explicit
`CARGO_TARGET_DIR` to a volume with adequate bytes and inodes before deleting a costly
shared cache.

## 4. Protect durable state

Never delete or overwrite:

- tracked or untracked source;
- dirty worktree changes;
- task history, Git metadata, branches, commits, or unpushed work;
- `.context` captures or review artifacts unless the user separately authorizes their
  exact removal;
- authored fixtures, downloaded source assets, or other non-reproducible inputs.

Do not remove a Git worktree to recover build capacity until its status, upstream
divergence, open-PR role, and unique untracked files have all been checked. Even then,
worktree removal is a separate repository-lifecycle decision, not the default cleanup.

## 5. Rebuild deliberately

After recovery:

1. Report what was removed, how much space was recovered, and whether it is reconstructible.
2. Recheck free space and confirm no unexpected worktree changed.
3. Warm only the profile needed by the next gate.
4. Run the exact command that previously failed.
5. Verify the intended test count or benchmark sample count; a zero-test filtered command
   is not success.
6. If the same gate fails again with adequate capacity, reclassify it as code, test,
   environment, or harness failure and stop cleaning.

## Stop Conditions

- The target path is unresolved, shared by an active task, or lacks clear ownership.
- The candidate worktree is dirty, unpushed, or tied to an unresolved PR.
- A build process may still be using the target.
- The only way forward appears to be deleting captures, source, or Git history.
- Cleanup has already restored adequate capacity but the original gate still fails.
