# Territory sweep

Last swept 2026-08-10 after fetching `origin` and completing the temporary world-owner
landing train. `origin/dev` is
`1dca1065c7681737ce424fa187879ea31974e356`; the wave's additive refresh is
`e610e26c50398e43ff23bc4db0890ba7463f11ae`.

Commands:

```sh
gh pr list --state open --limit 100 \
  --json number,title,headRefName,baseRefName,isDraft,author,updatedAt,headRefOid
git diff --numstat origin/dev...origin/<branch>
```

| PR | Original head | Measured footprint | Relevant regions | Current disposition |
|---:|---|---:|---|---|
| 186 | `wave/visibility@d3ec9e7b` | 34 files, +3597/−518 | perception, core exports, combat commands/AI, game save/UI | merged to `dev` as `3f2f6dc4` |
| 187 | `wave/hex-81-surface-feature-contract@244b6b7b` | 4 files, +852/−6 | `hex_core` public surface contract, boundary docs | merged as `0e14e89d`; reserved vocabulary has no live snapshot producer |
| 188 | `wave/hex-87-movement-feedback@3234f060` | 8 files, +881/−85 | unit movement/selection, gameplay walk | merged as `9267d9f8` |
| 189 | `wave/hex-79-heal@fbf9ee7c` | 37 files, +3018/−244 | combat authority, save, gameplay/UI/content | merged as `b6ac0455` after additive targeting repair |
| 190 | `wave/hex-89-first-person@15102e23` | 52 files, +5366/−775 | inherited visibility, world camera/cutaway, game/unit/UI walk | unique work represented through `32577c26`; delivery reconciled at `1dca1065` |
| 196 | `feat/lattice-fusion-gem-sharing@25d0be5d` | 4 files, +528/−41 | lattice rules/tests plus one-line `hex_game/src/lib.rs` composition | open; no L3/protocol overlap, re-sweep coordinator composition before L4/final gate |

The measured footprints remain as the durable pre-landing collision record. Trova's source
branches are retained; none is automatically deleted during the temporary delegation.

## Lane impact

- **Foundation/L1/L2:** merged to the wave; the final `dev` refresh preserved both
  multiplayer authority and landed world/gameplay contracts.
- **Coordinator protocol amendment:** owns only `crates/hex_multiplayer/**`, architecture,
  contracts, and this wave record; no open PR overlaps those regions.
- **L3:** all dispatch blockers are clear under the 2026-08-10 temporary world-authority
  ratification. Refresh exact `hex_map`/`hex_perception` anchors before implementation.
- **L4:** PR #195 remains merge-blocked on L3; refresh it from the post-L3 wave and resolve
  only its manifest-owned regions.

Re-sweep before L3 dispatch, L3 merge, #195 refresh/merge, and the combined gate. A changed
footprint is an escalation and manifest update, not an informal exception.
