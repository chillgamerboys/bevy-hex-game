# Runtime UI verification

Runtime UI is accepted as a task graph, not a gallery of widgets. Every shipping
route below must have a populated immutable presentation fixture, named primary and
secondary actions, a renderer-free model transition, a structural oracle, and a
bounded native frame when its visual composition is distinct.

The structural registry gives materially distinct HUD and Settings compositions their
own cases. `FormationMainView`, `SettingsKeybindings`, `SettingsCapture`, and
`SettingsConflict` therefore cannot be hidden inside a generic Gameplay or Settings
fixture merely because they share the same coarse `Screen`.

`hex_ui` renders route-specific views and emits only typed `MainMenuIntent`,
`SandboxIntent`, and `MultiplayerIntent` values. It owns no navigation, persistence,
map/seed selection, roster, character, deployment, content, launch, or session
authority. `hex_gameplay_model` owns pure transitions, while `hex_game` adapts them to
Bevy, assets, persistence, loading, and the transport-neutral multiplayer runtime.

## Shipping route inventory

The Main Menu exposes exactly five actions:

```
Main Menu
├── Campaign ───────────────────────────> exactly three slot cards
├── Sandbox ──> Overview ─┬─> Map Browser ─> Map Detail
│                         ├─> Party ────> Character Picker
│                         └─> Enemies ──> Character Picker
├── Multiplayer ─┬─> Host Direct ─> shipped Sandbox / Deployment ─> Lobby
│                └─> Join Direct ────────────────────────────────> Lobby
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
| Main Menu | root | Campaign, Sandbox, Multiplayer, Tools, Settings | exactly five enabled named controls in that order; no obsolete route label |
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
| Multiplayer | home | Host Direct, Join Direct, Back | exactly one explicit entry for each direct role; Steam is described as later traversal, not an available route |
| Host Direct | endpoint draft | Configure Shipped Sandbox, Back | editable advertised host and UDP port; forwarding/CGNAT/no-relay limits remain visible; no socket opens before explicit configuration completes |
| Join Direct | code draft | Join Session when non-empty, optional Reconnect Reserved Seat, Back | bounded credential-bearing input; ordinary diagnostics redact the complete code |
| Multiplayer lobby | host and guest projections | host assignment/kick/launch/close or guest ready/leave | six stable seats; connection/reservation/delegation, assignments, readiness, host, local seat, and exact launch blocker are visible without granting UI authority |
| Multiplayer verification/reconnect/end | representative typed states | Leave Session or Multiplayer Home | loading cannot enter gameplay before exact local-world readiness; reconnect and terminal reasons remain distinct |
| Character Creator | saved clean Map-ready character | Library, Save, Local Test, Open in Sandbox | Open in Sandbox preserves map/Enemies, replaces Party with slot 1 only, retains typed Creator origin |
| Spell Creator | library/workspace variants | Library, Save | Tools-origin return stays typed as Tools |
| Tools | complete | Back, Character Creator, Spell Creator | Map Creator visible, disabled, and labelled Coming Soon; exactly those three tools |
| Deployment placing | 1v1 and sparse 6v6 | Undo when available, Return to Sandbox | compact current-character card; stable Party-then-Enemies progress; any canonical legal unoccupied exact surface accepted; invalid footing or occupancy visibly refused; ordinary HUD absent from layout, focus, scrolling, and picking |
| Deployment Review | complete 1v1 and sparse 6v6 | Undo, Return to Sandbox, Start Combat | every occupied slot has one unique exact surface; earlier slots can be selected for repositioning; exact launch remains frozen only after Start |
| Sandbox outcome | Victory and Defeat | Retry Exact, Return to Sandbox | no telemetry/report controls; retry retains launch snapshot identity |
| Settings General | authored and persisted values | Back | changes save immediately; all controls have labels; chosen presentation survives restart |
| Settings Keybindings | all categories | Back, category tabs, Restore per row, Restore All | Gameplay, Interface, Main View, Camera, and System are complete and stably ordered; shipping omits development-only actions; fixed UI navigation is identified |
| Settings key capture | one rebindable row | Cancel capture | next non-modifier key is captured at highest priority and does not reach gameplay; Escape cancels |
| Settings key conflict | overlapping-context collision or row restore after Swap | Swap, Cancel | no silent stealing; Swap updates both rows atomically; Cancel preserves both; a refused restore leaves persisted overrides valid and unchanged |
| Gameplay exploration | default preferences | Party and eligible Action Bar | Party visible, Initiative ineligible, Activity closed, Main View closed; no redundant screen residue |
| Gameplay player turn | maximum eligible actions | Party, Initiative, Action Bar | stable disclosed order and every authorized control; world movement feedback remains unobscured; no duplicate actor/round/budget summary |
| Gameplay hostile turn | mixed disclosed order | Initiative | no player action affordance; disclosed hostile is inspectable and an unobserved hostile is not activatable or locatable |
| Character Main View | disclosed Party member | close / replace destination | readable lattice and character detail; inspection never changes gameplay authority |
| Formation Main View | six-member exploration party | movement mode, member, preset, and formation-slot controls | one scroll owner; every control is reachable; explicit member selection is the only HUD path that changes formation movement authority |
| Activity | mixed history | All, Combat, Activity | bounded disclosure-frozen lines filter by selected tab; danger has a non-color cue |
| Custom HUD visibility | Party + Activity only | component shortcuts | saved combination is exact; Initiative, Action Bar, and Main View leave no layout or focus residue |
| Compact temporary surface | Party | Escape / same shortcut | blank map becomes exactly one full-screen task; no handle, drawer, or second scroll owner |
| Gameplay casting / aiming | populated and blocked states | cast, target cycle, confirm, cancel | only canonical actions are enabled; retained target and refusal copy remain disclosure-safe |
| Required decisions | disable and restore | Clear, Confirm | Required Decision forcibly owns Main View and cannot close or be replaced until resolved |
| Master-hidden required decision | `H` hidden | Clear, Confirm | all ordinary components are absent while the required decision remains fully reachable |
| Pause | active gameplay | Resume, Return to Main Menu | overlay presentation leaves component preferences and stored Main View state intact |

Campaign adds no delete, overwrite, difficulty, or character-selection flow. A New
Game action exists only on an Empty card and cannot replace an occupied or invalid
record.

Creator library, workspace, recovery, deletion-confirmation, local mechanics test,
and party-formation cases remain in the registry when their behavior is unchanged.
They must use typed origins where their Back action crosses into Tools, a Sandbox
picker, or a Creator-owned Sandbox flow.

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

Gameplay uses the same tokens but a different map-first composition. On Standard/Wide,
Party, Initiative, Activity, and Action Bar may be shown independently around at most
one typed Main View. On Compact the default is a completely unobstructed map: no
collapsed handles, drawers, shortcut hints, or invisible hit regions remain. One
explicit shortcut may temporarily replace it with one full-screen task, closed by the
same key or Escape. A required decision instead owns that surface until answered.

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
They also exhaust the HUD visibility truth table across saved preference, contextual
eligibility, master suppression, phase suppression, Standard/Compact presentation,
temporary summons, and forced decisions. Main View replacement/close rules, repeated
inspection activation, component preference preservation, and restart-only state are
renderer-free contracts.

Headless application tests cover exactly five Main Menu actions, exactly three
Campaign cards, mixed slot projection, absence of obsolete shipping navigation,
canonical catalog/content/rules/deployment resources, a real exact-terrain placement
outside the staging regions, complete ordinary-HUD suppression, cold launch, Sandbox
re-entry, outcome return, Creator-origin return, and actual focus-tree/control names.
Multiplayer tests separately cover role-gated intents, six-seat structure, local-client
menus, exact-world activation gating, and the fail-closed L3 handoff. Those tests may
build UI trees through default-off `test-support`; production plugins do not gain test
fixture routes.

Application tests additionally cover input capture priority, fixed UI navigation,
Swap/Cancel conflict handling, row and confirmed-all restoration, schema-v3 migration,
and preference survival across restart. Inspection tests prove first activation
publishes one disclosed camera subject and one Map-center request, repeated activation
opens Character Main View, Character mode follows, and none of those paths mutates
selection, turn, caster, command ownership, or formation. Hostile cases must prove
that missing observation publishes no identity or location.

Screenshots prove static presentation structure: hierarchy, layout, legibility,
contrast, responsive reflow, focus visibility, camera framing/occlusion, and how the
hook-established pictured state is rendered. Video and human checks prove camera
motion, native-input response, animation, control feel, and taste. A still frame does
not prove motion. No visual artifact proves map selection, save semantics, readiness,
placement, exact occupancy, combat rules, outcome, or Retry identity; those claims
require typed state and canonical snapshots.

## Bounded native review

Review at most ten native HUD frames:

1. minimal Exploration defaults;
2. player turn with maximum eligible actions;
3. hostile turn with mixed disclosed Initiative;
4. forced Required Decision;
5. aiming with Action Bar states;
6. Activity with all three tabs visible;
7. custom Party + Activity visibility;
8. Character Main View;
9. master-hidden HUD with Required Decision still open; and
10. one targeted Compact or 4K/200% duplicate chosen from structural findings.

The shipping route inventory above remains exhaustive in the headless structural
matrix; it does not consume duplicate native-review frames when this change's visual
risk is gameplay HUD composition. The automation receipt records the exact-head
commit SHA and visual-walk status. Capture failures and review findings include their
walk step and PNG path; successful frame paths remain in the capture output and
runtime log rather than being duplicated in the receipt. The reviewer opens every
selected image; a successful capture process is not itself approval.

The exact-head human playthrough remains the final gate for motion, control feel,
native text rendering, and taste. It follows Main Menu → Campaign save/Continue,
Main Menu → Sandbox map/rosters/deployment/outcome/retry/return, Tools → Creator
return, and post-restart presentation. During gameplay it exercises `H`, every
component shortcut, both Main View shortcuts, first/repeated Party and disclosed
Initiative activation in Map and Character camera modes, required-decision ownership,
deployment/outcome suppression, Compact map-only presentation, key capture and one
Swap conflict, then restarts to inspect how the saved preference/keybinding state is
presented. Typed restart hooks separately prove exactly what persisted.
