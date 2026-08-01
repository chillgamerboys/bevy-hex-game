# Runtime UI

The runtime UI is a presentation adapter, not a gameplay authority. `hex_game`
observes canonical resources and components, projects immutable view models, and
applies typed `UiIntent` messages through the same command and navigation funnels as
other inputs. `hex_ui` renders those models. It must not infer legality, mutate
combat state, or import gameplay/world implementations.

## Dependency contract

`hex_ui` may depend only on Bevy, `hex_core`, `hex_assets`,
`hex_gameplay_model`, and serialization support. It must never depend on
`hex_game`, `hex_combat`, `hex_units`, `hex_lattice`, `hex_map`, `hex_world`, or
`hex_perception`.

The public seam is deliberately small:

- `UiPlugin` installs the design system, focus navigation, responsive scaling, and
  renderer-owned surfaces.
- Immutable screen view models contain display-ready facts. They do not expose
  mutable screen state or ECS entities.
- `UiIntent` describes player intent. The composition root decides whether and how
  it changes application or gameplay state.
- `ActionAffordance` carries the label, shortcut, priority, and canonical enabled or
  disabled reason for one action.
- The default-off `test-support` feature exposes immutable UI-tree observations.
  `HeadlessUiPlugin` runs the real Bevy UI, text, focus, and layout schedules on a
  synthetic primary window without Winit, a renderer, or gameplay plugins.

Creator and Combat Lab transitions remain in `hex_gameplay_model`. A widget may
select or submit a transition, but it must not duplicate the transition policy.
Combat Lab setup, saved-report controls, and the deployment HUD are rendered from
`CombatLabScreenView` and `DeploymentView`; `hex_game` alone validates map readiness,
persists report annotations, resolves exact surfaces, and admits Start Combat. The
3D deployment highlights and placement tokens remain spatial world presentation,
not runtime UI.

## Information hierarchy

Every interactive screen uses the same hierarchy:

1. A title or breadcrumb says where the player is.
2. Setup flows show the current step and the remaining steps.
3. Main content contains the current choice, with validation adjacent to the
   affected control.
4. A persistent footer contains Back and the next or confirm action.

Gameplay adds a persistent **Now / Choose / Confirm** rail. It always presents the
current actor, phase, remaining movement and action, and the currently authorized
actions. Blocking decisions take `Required` priority and show their progress. The
rail is outside the HUD visibility tree, so hiding inspectors or the HUD cannot hide
the required action.

Party and turn state are primary information. Inspector, event log, and Combat Lab
statistics are secondary drawers. A responsive region presents at most one secondary
drawer; compact layouts collapse secondary information before they reduce action
visibility.

Builds with the default-off `dev` feature add a `DEV · TIME` panel to the gameplay
Inspector region. `hex_ui` renders only the immutable current-hour or unavailable
projection and emits typed half-hour/preset intents; the `hex_game` adapter remains
responsible for changing the existing session clock. Static lighting exposes a reason
instead of controls, and shipping builds contain neither the panel nor its adapter.

## Responsive model

The world camera always renders at native resolution. Bevy's global `UiScale`
remains `1.0`, and operating-system DPI scaling stays authoritative. Semantic
tokens scale UI content without scaling the entire Bevy layout tree.

Auto scale is:

```text
clamp(min(logical_width / 1920, logical_height / 1080), 1.0, 1.5)
```

Manual 75%, 100%, 125%, 150%, 175%, and 200% choices replace Auto; they do not
multiply it. Body and supporting type use that content scale subject to the 18px
essential-text floor. Display, title, and heading type use
`1 + 0.5 × (content_scale - 1)`, capped at `1.5`. Control
geometry uses the same moderated growth above 100% but never shrinks below its
authored baseline, preserving 44×44 logical-pixel targets. Spacing uses
`1 + 0.25 × (content_scale - 1)`, capped at `1.25`. Layout is selected from the
logical canvas divided by the greater of content and spacing scale. This matters at
200%: a 1920×1080 window must reflow to Compact instead of keeping Standard side
rails beside doubled essential copy. Below 100%, spacing remains the density limit so
smaller type does not unexpectedly promote an ordinary canvas to Wide.

| Class | Semantic-density-adjusted logical canvas | Behavior |
|---|---|---|
| Compact | below 1440×810 | one content column; drawers overlay/collapse; action rail remains full width |
| Standard | at least 1440×810 and below 2400 px wide | primary content plus one secondary region |
| Wide | at least 2400 px wide | bounded primary content with one persistent secondary region |

Representative structure:

```text
Compact                         Standard / Wide
┌ title + step ┐                ┌ title + step ────────────────┐
│ main content │                │ main content │ one drawer    │
│ validation   │                │ validation   │ or party info │
├──────────────┤                ├──────────────────────────────┤
│ Now / Choose / Confirm        │ Now / Choose / Confirm       │
└──────────────┘                └──────────────────────────────┘
```

Structural tests cover 960×540, 1280×720, 1512×949, 1920×1080, 2560×1440, and
3840×2160 at 1× and 2× device scale in every semantic UI scale mode. Device pixels
remain separate from logical layout, so 1280×720 @2× and the observed 3024×1898
physical fullscreen client (1512×949 logical @2×) exercise the same contract.
Primary controls must be fully visible immediately; secondary catalog/report content
may instead prove complete scroll reachability. Every required control remains
unobscured, accessible, and at least 44×44. `UiTreeSnapshot` intersects each node
and named text node's actual glyph rectangles with the canvas and Bevy's inherited
`CalculatedClip`; a nonzero `ComputedNode` whose glyphs or box cross a clipped edge
is not treated as fully visible. The oracle also checks focus order and interactive
overlap without interpreting the text or pixels as gameplay truth.
The matrix uses the full production title routes, the independently filtered Map
Scenarios and Demos catalogs, populated
Settings, Creator and Combat Lab setup projections, a 6v6 deployment, and the maximum
ordinary gameplay action rail plus required, aiming, statistics, and report states. A half logical
pixel is the only target-size tolerance, accounting for physical-pixel rounding at
fractional Auto scales.

## Typography, spacing, and contrast

At 100% scale the semantic type tokens are:

| Token | Size | Use |
|---|---:|---|
| Display | 48 | game/title display |
| Screen title | 32 | top-level screen heading |
| Heading | 24 | sections and drawer titles |
| Body/control | 20 | required information and controls |
| Supporting | 18 | guidance and validation detail |
| Metadata | 16 | optional, nonessential annotations only |

Essential text must be at least 18 logical pixels. Pointer targets are at
least 44×44 logical pixels. Layout uses semantic gaps and panel padding rather than
screen-specific offsets where a shared token applies.

Normal text targets at least 4.5:1 contrast. Large text, focus indicators, and other
meaningful non-text boundaries target at least 3:1. State is never communicated by
color alone: labels such as “Required”, “Unavailable”, or the refusal reason remain
visible. These thresholds follow
[Microsoft's game text guidance](https://learn.microsoft.com/en-us/xbox/accessibility/xbox-accessibility-guidelines/101),
[Microsoft's UI-context guidance](https://learn.microsoft.com/en-us/xbox/accessibility/xbox-accessibility-guidelines/114),
and [WCAG 2.2](https://www.w3.org/TR/WCAG22/).

## Focus and keyboard behavior

Interactive controls participate in a logical Tab/Shift-Tab order and carry an
`AccessibleLabel`. Keyboard focus has a visible high-contrast outline. Enter and
Space activate the focused control through its ordinary interaction handler. Escape
uses the screen's typed Back/resume intent consistently.

When a focusable control owns keyboard focus, Enter, Space, and Tab do not also
dispatch gameplay confirm, end-turn, or next-target shortcuts. The focused control's
typed intent is the single input for that keypress; explicit cancel/back keys remain
available.

The runtime derives sequential focusability from the visible hierarchy. Controls in
a hidden or `Display::None` subtree temporarily leave the tab order and return at
their original logical index when the surface reopens; focus is cleared if its
control becomes unreachable.

A true modal uses a Bevy `TabGroup` so focus cannot escape until its blocking choice
is resolved. Informational drawers are not modals and do not trap focus. Controller
navigation and remapping are intentionally deferred.

This foundation uses Bevy's stable tab navigation, focus, accessibility, image
render targets, and screenshot components. The global `UiScale` remains 1.0;
semantic typography, control, and spacing tokens provide accessibility scaling
without doubling whole panels. Experimental widgets and BSN are outside this
stabilization change.

## Preferences

Preferences schema v2 persists `UiScaleMode`. Reading v1 preserves its display and
audio settings and defaults UI scale to Auto. A change previews immediately through
the renderer resource and is persisted through the existing preferences writer.

## Testing oracle boundary

The exhaustive player-task inventory, coverage tiers, and fail-closed control
classification live in [Runtime UI verification](../development/ui-verification.md).
That inventory is the acceptance source for route and fixture completeness; this
section defines which oracle may prove each kind of fact.

Use the cheapest authoritative oracle:

- Pure view-model, scale, breakpoint, intent, and priority behavior stays inline in
  `hex_ui`.
- Game and UI wiring uses the existing `gameplay_app` integration target with
  `test-support`. `GameplayStateSnapshot` reads canonical resources/components;
  `UiTreeSnapshot` reads presentation structure only.
- Deterministic combat and balance evidence belongs to the rules, contracts, and
  simulation partitions.
- The scoped presentation route reviews exactly ten deterministic Bevy image-target
  frames from `walks/gameplay_ui.ron`. Every `Capture` first passes the live
  structural oracle. A typed review viewport supplies logical size and device scale;
  Bevy renders into an `ImageRenderTarget` and captures it with `Screenshot::image`.
  No operating-system capture API or primary-window screenshot participates.

A screenshot must never prove legality, budgets, decisions, damage, Channel,
outcomes, persistence, deployment, or report identity. The visual runner therefore
has no combat-solving verbs; presentation fixtures open authored states while typed
tests prove their canonical facts.

Forest, Waterfall, map-review, V3, and world-owned captures remain outside this
contract and are unchanged.

Local commands:

```sh
mkdir -p .context/ui-review
ui_review_data="$(mktemp -d .context/ui-review/data.XXXXXX)"
HEX_GAME_DATA_DIR="$ui_review_data" \
HEX_WALK_SCRIPT=walks/gameplay_ui.ron \
HEX_WALK_OUT=.context/ui-review/bevy \
cargo run -p hex_game --features visual-walk
```

## Screen audit

| Surface | Primary task | Persistent action | Secondary content |
|---|---|---|---|
| Splash/loading | understand progress | none | none |
| Title | choose route | all primary routes initially visible | none |
| Map Scenarios | choose a map/world presentation fixture | Back | map-only catalog |
| Demos | choose a focused gameplay demonstration | Back | demo-only catalog |
| Settings | change one preference | Back | persistence notice |
| Character / Spell Creator | finish the current authoring step | Library / Save / Test where applicable | palettes, catalogs, validation, and history |
| Combat Lab setup | choose fixture/profile and deploy | Back / Launch | fixture explanation and tuning |
| Gameplay | act for the current unit | Now / Choose / Confirm rail | inspector, log, statistics |
| Pause | resume, save, or leave | Resume | save notice |
| Report / Compare | inspect one axis at a time | Back / Retry / Copy / Tune | alternate report views |

When a screen changes, review it against the hierarchy, focus, compact reflow, and
oracle boundary above before adding a screen-specific workaround.
