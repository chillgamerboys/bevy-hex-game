---
name: release
description: "Cut an explicitly approved semantic version: land the version bump on dev, promote the exact played dev head to main, then create and push the matching immutable tag."
---

# Cut a release

Releases are operator-approved aggregated events. The bump is ordinary work on `dev`;
only `/promote` moves it to `main`, and only the promoted commit may be tagged.

## Compute and prepare the bump

1. Fetch `origin/dev`, `origin/main`, and tags. Require a clean workspace already
   based exactly on current `origin/dev`; never checkout, switch, create, or rename a
   branch inside Conductor.
2. Find the latest reachable `vX.Y.Z` tag and inspect non-merge Conventional-Commit
   subjects and `BREAKING CHANGE:` footers through `origin/dev`. Before `1.0.0`, both
   a breaking change and a `feat` bump the minor version; a `fix`/`perf` or otherwise
   releasable range bumps patch. Reserve the `0.x` to `1.0.0` jump for explicit user
   approval. At or after `1.0.0`, use ordinary major/minor/patch semantics. Ask the
   user to approve the exact next version before editing. A dry run stops here after
   showing the proposed changelog.
3. Update only the workspace package version, regenerated `Cargo.lock`, and the
   changelog section derived from actual commits. Preserve historical entries and
   never rewrite unrelated package versions.
4. Validate the version and lockfile, then open the bump PR to `dev` using
   `/create-pr`. Run `/audit-pr` and merge only with explicit user approval through
   `/merge-pr`.

## Promote, tag, and publish

After the bump PR lands, run `/promote`; its exact-`dev` named-human playtest is the
release gate. Fetch `origin/main` after promotion and verify the manifest version,
lockfile, changelog heading, and promoted merge all agree. Require final confirmation,
then create one annotated `vX.Y.Z` tag at that exact `origin/main` commit and push only
that tag. Never tag from `dev`, move a tag, or replace an existing tag.

Monitor the release workflow and verify the GitHub release and expected platform
artifacts. Report the version, source range, version-PR/merge SHA, tag target, workflow
result, and any missing artifact. A failed publish does not justify retagging; diagnose
and retry the workflow against the same immutable tag.
