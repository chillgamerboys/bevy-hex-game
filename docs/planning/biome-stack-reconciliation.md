# Biome stack delivery reconciliation

Status: integrating. Cutoff: 2026-09-02. Base `origin/dev`:
`fc55bd5a1c3c0181b6506d5ac59e1189d287838a`.

This is a delivery-state record for a user-authorized, pre-existing sequential PR stack.
It is intentionally not a wave manifest: there is no aggregate `wave/*` branch, and changing
the published topology would make the history and review state less truthful. The source
waves retain their own manifests on their branches.

## Outcome and exclusions

Deliver Crystal Mountain, Arid/Desert Oasis Rings, Coastal/Ocean Archipelagoes, and the
corrective biome-feedback pass to `dev` in strict dependency order with exact-head automated
and human evidence.

Grand V3, Garden, Outpost, route reauthoring, time-cycle work, subtle-geometry experiments,
and teammate PR #196 are excluded.

## Locked delivery decisions

1. Land #210, then #211, then #212, then #213; never merge a child before its parent is
   verified on `dev`.
2. Use merge commits, never squash, rebase, or force-push, so exact source heads remain
   reachable from `dev`.
3. Retarget each child to `dev` only after its parent merge is verified, then merge current
   `dev` into the child additively.
4. A draft-green manual-sign-off check is a deferral, not named-human evidence.
5. Replace fragile partition counts before refreshing the stack. Add required ignored-test
   identities in the same candidate that introduces them.
6. Keep parent branches until every child and downstream Grand branch is retargeted and the
   exact ancestry is verified.

## Live stack

| Order | PR and source | Declared base | Original head | Unique footprint | Live blocker |
|---:|---|---|---|---:|---|
| 1 | [#210](https://github.com/chillgamerboys/bevy-hex-game/pull/210), `wave/crystal-mountain` | `dev` | `74deb7f84d92e2088c63eafc1d5988c63171896d` | 25 commits; 52 files | cancelled macOS build; exact-head human Crystal review |
| 2 | [#211](https://github.com/chillgamerboys/bevy-hex-game/pull/211), `wave/desert-biomes` | `wave/crystal-mountain` | `441c22cc6968478993e920a1a575fa086edc05ee` | 3 commits; 61 files | Map partitions count failure; cancelled macOS build; exact-head Desert/Oasis review |
| 3 | [#212](https://github.com/chillgamerboys/bevy-hex-game/pull/212), `wave/island-biomes` | `wave/desert-biomes` | `09bc0ebc28be9bb800dadb1f2e6d9a31f01cb3c8` | 6 commits; 62 files | current-base CI; exact-head island presentation/motion review |
| 4 | [#213](https://github.com/chillgamerboys/bevy-hex-game/pull/213), `fix/biome-feedback` | `wave/island-biomes` | `63aed363e5ba394c4404e9b168967548960e851e` | 2 commits; 27 files | Map partitions count failure; exact-head cross-biome flicker review |

The footprints are measured against each immediate parent. Shared hotspots include
`.config/test-scopes.json`, map and scenario definitions, save fixtures, procedural generators,
camera routes, status/roadmap documents, and the source-wave manifests. Later branches extend
their parents and must not replace parent semantics with obsolete snapshots.

## Integration procedure

1. Merge `docs/biome-stack-reconciliation` to establish this truthful delivery record.
2. Merge `fix/ci-test-identity-selection` to `dev`.
3. Merge current `dev` into #210. Resolve `.config/test-scopes.json` by retaining identity
   coverage and adding the three Crystal Mountain ignored-test identities introduced by #210.
   Rerun all platforms and obtain exact-head human review before a merge commit.
4. Retarget #211 to `dev`, merge current `dev` into its source, clear Map partitions without
   numeric pins, rerun macOS, obtain exact-head review, and merge.
5. Repeat for #212 after #211, rerunning all checks even though its old head was green.
6. Repeat for #213 after #212 and complete the native Crystal/Desert/Island flicker route.
7. After every merge, verify both the original and refreshed source heads are ancestors of
   `origin/dev`. Keep all source branches until Grand V3 ancestry is reconstructed.

## Acceptance and stop conditions

- Every refreshed head passes the complete selector-chosen CI-equivalent gate.
- Typed contracts prove generation, publication, regeneration, re-entry, routes, geometry,
  and fingerprints.
- Fresh static review covers each affected biome and a named human records exact-head native
  camera motion, flicker, input/control feel, and play findings.
- Status, roadmap, and source manifests describe only residual work after each merge.
- Stop if a child does not contain its declared parent, GitHub cannot use a merge commit, an
  operation would require force-pushing, a transplant would resurrect obsolete ancestry, or
  a conflict changes world/gameplay authority instead of mechanically composing the stack.

## Close-out

After #213 lands, record all four merge commits and the resulting `origin/dev` SHA here,
retarget downstream consumers, reconcile the source manifests/status/roadmap, and only then
classify the stack as delivered.
