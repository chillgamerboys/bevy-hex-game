# Reconciliation command guide

Load this reference when the source branches have different bases, stacked ancestry,
or stale parent state. Commands are inspection recipes, not a substitute for reading
the diffs.

## Inventory

```sh
git fetch origin --prune
git branch -r --contains <sha>
git merge-base origin/dev origin/<source>
git log --graph --oneline --decorate origin/dev origin/<source>
git diff --stat <merge-base>..origin/<source>
git log --reverse --format='%H %s' <merge-base>..origin/<source>
git cherry -v <intended-base> origin/<source>
```

For a PR, also inspect its declared relationship rather than inferring it from the
commit graph:

```sh
gh pr view <number> \
  --json number,title,state,isDraft,baseRefName,headRefName,headRefOid,body,mergeable,statusCheckRollup
```

## Select the integration operation

- **Merge the branch** when its full ancestry is current and belongs in the wave.
- **Cherry-pick a unique range** when the tip contains an obsolete parent snapshot
  but the intended child commits are cleanly identifiable.
- **Reimplement the small residual diff on the wave** when conflict resolution is
  the feature and transplanting would preserve a duplicate implementation.
- **Stop for an ownership decision** when choosing a version changes world or
  gameplay authority. Mechanical conflicts alone do not require a stop.

After every operation, compare both the file diff and commit provenance:

```sh
git diff --stat origin/dev...HEAD
git diff --check
git log --oneline origin/dev..HEAD
```

Never rebase or force-push a published source branch. Keep source branches until the
wave has landed and every child PR has been retargeted.
