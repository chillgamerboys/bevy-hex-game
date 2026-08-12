# Host-owned Campaign territory sweep

Sweep time: 2026-08-12. Base:
`origin/dev@a0f95e62d02c663902b864cc08a89e831d9ba437`.

## Open pull requests

| PR | Base / head | Measured footprint | Relationship |
|---|---|---:|---|
| #196 `feat(lattice): let a shared gem fund more than one fusion` | `dev` / `feat/lattice-fusion-gem-sharing@25d0be5d9a492d5c3ef679c087c126b51db722a9` | 4 files, +528/-41 | `hex_lattice/src/cast.rs`, `tables.rs`, and its engine tests are disjoint. Its one-line `crates/hex_game/src/lib.rs` logging composition edit is an annotated L2 hotspot; preserve it with an additive current-`dev` merge. |

The merge base for #196 is
`9267d9f899a6caaec870980e10668c82cdcf1d06`. Its exact numstat is:

```text
1       0       crates/hex_game/src/lib.rs
200     32      crates/hex_lattice/src/cast.rs
16      9       crates/hex_lattice/src/tables.rs
311     0       crates/hex_lattice/tests/engine.rs
```

No other open PR exists. The Direct Sandbox source/wave branches are retained historical
branches but their PRs are merged and their wave manifest is closed; they are not active
territory.

## Delivery and ticket reconciliation

- PR #192 is delivered on `dev` through merge `d0a3a334ebb719456c5a07a483212cacb068060a`.
- PR #201 closed its durable wave record through
  `a0f95e62d02c663902b864cc08a89e831d9ba437`; every post-merge check passed.
- The live Linear sweep found no non-terminal issue whose outcome is Campaign multiplayer,
  EOS, or Steam integration. The wave uses `ticket: null` rather than creating one issue
  per lane.
- `HEX-95` remains an independent Main Menu heading-inset UI bug and is not wave scope.

## Refresh points

Fetch/prune and remeasure all open PRs immediately before:

1. merging the EOS feasibility/shared-contract foundation to `dev`;
2. cutting `wave/host-owned-campaign`;
3. dispatching each lane;
4. merging each lane to the wave; and
5. opening/merging the combined wave PR.

Any disagreement with this banked table is an escalation, not an in-lane judgment call.
