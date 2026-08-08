# Territory sweep

Last swept 2026-08-08 after `git fetch origin --prune`.

Commands:

```sh
gh pr list --state open --limit 100 \
  --json number,title,headRefName,baseRefName,isDraft,author,updatedAt,headRefOid
git diff --numstat origin/dev...origin/<branch>
```

| PR | Head | Base | Head SHA | Files | Add | Del | Relevant regions |
|---:|---|---|---|---:|---:|---:|---|
| 186 | `wave/visibility` | `dev` | `d3ec9e7b230a8a1704e7d82896dbe31e7689f98e` | 34 | 3597 | 518 | perception, core exports, combat commands/AI, game save/UI |
| 187 | `wave/hex-81-surface-feature-contract` | `dev` | `244b6b7b0fde076b1abddfc5beaefc306c4ea29d` | 4 | 852 | 6 | `hex_core` public surface contract, boundary docs |
| 188 | `wave/hex-87-movement-feedback` | `dev` | `3234f060abc0510c5ab68a7c30d3a8e540dd5862` | 8 | 881 | 85 | unit movement/selection, gameplay walk |
| 189 | `wave/hex-79-heal` | `dev` | `fbf9ee7c23a810cc4af4b8bf79099462ffe6c22e` | 37 | 3018 | 244 | combat authority, save, gameplay/UI/content |
| 190 | `wave/hex-89-first-person` | `wave/visibility` | `15102e231372bb741d916abd827e7e4090be8bc6` | 52 | 5366 | 775 | inherited visibility, world camera/cutaway, game/unit/UI walk |

All five PRs are drafts and remained open at the sweep. PR 190 is deliberately measured
against `origin/dev`, not only its stacked base, so its full inherited collision surface is
visible.

## Lane impact

- **Foundation:** new crate and new core modules/adjacent exports are remappable; root
  Cargo/docs/test selector are coordinator-only. PR 187 is a semantic input to the world
  snapshot review even if text merges cleanly.
- **L1:** no current branch touches `crates/hex_multiplayer/**`.
- **L2:** blocked by 186/188/189/190 unless exact symbols are remapped.
- **L3:** blocked by 186/187/190 and world-owner ratification.
- **L4:** blocked by 186/188/189/190 unless exact symbols are remapped.

Re-sweep before foundation landing, wave creation, each dispatch, each merge, and the
combined gate. A changed footprint is an escalation and manifest update, not an informal
exception.
