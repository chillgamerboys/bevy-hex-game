# Runtime UI verification

Runtime UI is accepted as a task graph, not a gallery of widgets. Every shipping
route below must have a populated immutable presentation fixture, named primary and
secondary actions, a renderer-free model transition, a structural oracle, and a
bounded native frame when its visual composition is distinct.

`hex_ui` renders route-specific views and emits only typed `MainMenuIntent` and
`SandboxIntent` values. It owns no navigation, persistence, map/seed selection,
roster, character, deployment, content, or launch state. `hex_gameplay_model` owns
pure transitions, while `hex_game` adapts them to Bevy, assets, persistence, and
loading.

## Shipping route inventory

The Main Menu exposes exactly four actions:

```
Main Menu
├── Campaign ───────────────────────────> exactly three slot cards
├── Sandbox ──> Overview ─┬─> Map Browser ─> Map Detail
│                         ├─> Party ────> Character Picker
│                         └─> Enemies ──> Character Picker
├── Tools ────────────────────────────> Character / Spell Creator
│                                         └─> Map Creator (Coming Soon)
└── Settings
```

There is no shipping navigation for internal launch catalogs, standalone showcases,
deterministic fixtures, alternate rule profiles, experiment statistics, report
history, report comparison, or report deletion. `Screen::Title` is only the internal
coarse host for Main Menu, Campaign, and Tools. `Screen::LatticeDemo` remains reachable
only from the Creator's local mechanics test.

## Exhaustive task cases

| Surface | Required state | Persistent actions | Content and behavioral oracle |
|---|---|---|---|
| Main Menu | root | Campaign, Sandbox, Tools, Settings | exactly four enabled named controls in that order; no obsolete route label |
| Campaign | all empty | Back; New Game per card | exactly slots 1–3; selecting an empty card binds that exact `CampaignSlotId` |
| Campaign | mixed | Back; New Game or Continue as applicable | Empty, Available, and Invalid project distinctly; invalid data and reason remain visible |
| Campaign occupied card | representative party | Continue | existing character/lattice presentation, formatted accumulated active-play time, exact slot intent |
| Sandbox Overview | default draft | Back, Start Sandbox | Flat Arena; Hedge Mage in Party 1; Raider in Enemies 1; two six-slot roster summaries |
| Sandbox Overview | blocked cases | Back | first centralized blocker and no contradictory launch affordance |
| Map Browser | full catalog | Back | one row per stable Sandbox map ID; selection creates pending choice only; Create New Map disabled/Coming Soon |
| Generated Map Detail | pending map | Back, Regenerate, Use Map | exact resolved seed is visible; Regenerate changes only pending seed; Use Map commits |
| Authored Map Detail | pending map | Back, Use Map | no Regenerate control; Back leaves committed draft unchanged |
| Party roster | sparse/duplicate six slots | Back | 2×3 slot composition at Standard/Wide; exact slot edit and clear; duplicates preserved |
| Enemies roster | sparse/duplicate six slots | Back | same component and ordering contract as Party with typed Enemy identity |
| Compact roster | six slots | Back | one vertical scroll owner; every slot and action reachable without horizontal clipping |
| Character Picker | template + saved choices | Back, Use Character, Create a New Character | preview does not mutate; commit targets exact side/slot; Back cancels |
| Character Picker after Creator | newly saved choice | Back | exact picker restored, new record highlighted, not automatically applied |
| Character Creator | saved clean Map-ready character | Library, Save, Local Test, Open in Sandbox | Open in Sandbox preserves map/Enemies, replaces Party with slot 1 only, retains typed Creator origin |
| Spell Creator | library/workspace variants | Library, Save | Tools-origin return stays typed as Tools |
| Tools | complete | Back, Character Creator, Spell Creator | Map Creator visible, disabled, and labelled Coming Soon; exactly those three tools |
| Deployment placing | 1v1 and sparse 6v6 | Undo when available, Return to Sandbox | compact current-character card; stable Party-then-Enemies progress; any canonical legal unoccupied exact surface accepted; invalid footing or occupancy visibly refused; ordinary HUD absent from layout, focus, scrolling, and picking |
| Deployment Review | complete 1v1 and sparse 6v6 | Undo, Return to Sandbox, Start Combat | every occupied slot has one unique exact surface; earlier slots can be selected for repositioning; exact launch remains frozen only after Start |
| Sandbox outcome | Victory and Defeat | Retry Exact, Return to Sandbox | no telemetry/report controls; retry retains launch snapshot identity |
| Settings | authored and persisted values | Back | changes save immediately; all controls have labels; chosen presentation survives restart |

Campaign adds no delete, overwrite, difficulty, or character-selection flow. A New
Game action exists only on an Empty card and cannot replace an occupied or invalid
record.

Creator library, workspace, recovery, deletion-confirmation, local mechanics test,
ordinary gameplay, pause, damage decision, aiming, and party-formation cases remain
in the registry when their behavior is unchanged. They must use typed origins where
their Back action crosses into Tools, a Sandbox picker, or a Creator-owned Sandbox
flow.

## Presentation contract

All routes use the established dark, warm, arcane tokens. A full-screen page has one
title/breadcrumb region, one content region, persistent actions, visible keyboard
focus, and one unambiguous scroll owner. Every interactive target is at least
44×44 logical pixels. Controls need text or an accessible label; color alone cannot
carry selection, readiness, changed state, or error meaning.

Standard and Wide use the reference horizontal compositions. Party and Enemies are
2×3 rosters with a stable reading and focus order. Compact stacks page content and
uses one scrollable roster column. A nested catalog or lattice surface may scroll
inside a bounded panel only when the surrounding page itself does not compete for the
same gesture.

The structural matrix is mandatory at these logical canvases:

| Canvas | Presentation settings |
|---|---|
| 1280×720 | Auto and 200% |
| 1920×1080 | Auto and 200% |
| 3840×2160 | Auto and 200% |

Every case fails on clipping, overlap, inaccessible controls, ambiguous scroll
ownership, bad focus order, missing labels, an undersized target, or content outside
the logical viewport. Scale tests use the UI's semantic presentation modes; they do
not infer correctness from physical-window size or a desktop screenshot.

## Pure and headless evidence

Renderer-free model tests cover every route and Back transition, draft preservation,
pending versus committed map identity, generated-seed regeneration, exact seed
launch, Party/Enemy side and slot identity, Creator return destinations, blocker
priority, stable roster flattening with duplicates, guided Party-then-Enemies
deployment order, exact occupancy, reselection, Undo, Review, and Retry Exact identity.

Headless application tests cover exactly four Main Menu actions, exactly three
Campaign cards, mixed slot projection, absence of obsolete shipping navigation,
canonical catalog/content/rules/deployment resources, a real exact-terrain placement
outside the staging regions, complete ordinary-HUD suppression, cold launch, Sandbox
re-entry, outcome return, Creator-origin return, and actual focus-tree/control names.
Those tests may build UI trees through default-off `test-support`; production plugins
do not gain test fixture routes.

Screenshots prove only presentation structure: hierarchy, layout, legibility,
contrast, responsive reflow, focus visibility, and the pictured state. They do not
prove map selection, save semantics, readiness, placement, exact occupancy, combat
rules, outcome, or Retry identity. Those claims require typed state and canonical
snapshots.

## Bounded native review

Review at most ten native frames from this set:

1. Main Menu
2. Campaign
3. Sandbox Overview
4. Map Browser
5. generated Map Detail
6. Party
7. Enemies
8. Character Picker
9. Tools
10. one targeted Compact or 4K duplicate chosen from the structural findings

An authored Map Detail may replace the generated detail when the no-Regenerate state
is the risk under review. The automation receipt records the exact-head commit SHA and
visual-walk status. Capture failures and review findings include their walk step and
PNG path; successful frame paths remain in the capture output and runtime log rather
than being duplicated in the receipt. The reviewer opens every selected image; a
successful capture process is not itself approval.

The exact-head human playthrough remains the final gate for motion, control feel,
native text rendering, and taste. It follows Main Menu → Campaign save/Continue,
Main Menu → Sandbox map/rosters/deployment/outcome/retry/return, Tools → Creator
return, and persistence after restart.
