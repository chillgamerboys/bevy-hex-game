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

## Responsive model

The world camera always renders at native resolution. Bevy's global `UiScale`
changes UI only and leaves operating-system DPI scaling authoritative.

Auto scale is:

```text
clamp(min(logical_width / 1920, logical_height / 1080), 1.0, 2.0)
```

Manual 75%, 100%, 125%, 150%, 175%, and 200% choices replace Auto; they do not
multiply it. Layout is selected from the effective post-scale canvas:

| Class | Effective canvas | Behavior |
|---|---|---|
| Compact | below 1600×900 | one content column; drawers overlay/collapse; action rail remains full width |
| Standard | 1600×900 through 2399 px wide | primary content plus one secondary region |
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

Structural tests cover 960×540, 1280×720, 1920×1080, 2560×1440, and
3840×2160 in Auto and 200% modes. Required controls must remain visible,
unobscured, and reachable.

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

Essential text must be at least 18 physical pixels at 1080p. Pointer targets are at
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

The runtime derives sequential focusability from the visible hierarchy. Controls in
a hidden or `Display::None` subtree temporarily leave the tab order and return at
their original logical index when the surface reopens; focus is cleared if its
control becomes unreachable.

A true modal uses a Bevy `TabGroup` so focus cannot escape until its blocking choice
is resolved. Informational drawers are not modals and do not trap focus. Controller
navigation and remapping are intentionally deferred.

This foundation uses Bevy's stable
[`UiScale`](https://bevy.org/examples/ui-user-interface/ui-scaling/), tab navigation,
focus, and accessibility components. Experimental widgets and BSN are outside this
stabilization change.

## Preferences

Preferences schema v2 persists `UiScaleMode`. Reading v1 preserves its display and
audio settings and defaults UI scale to Auto. A change previews immediately through
the renderer resource and is persisted through the existing preferences writer.

## Testing oracle boundary

Use the cheapest authoritative oracle:

- Pure view-model, scale, breakpoint, intent, and priority behavior stays inline in
  `hex_ui`.
- Game and UI wiring uses the existing `gameplay_app` integration target with
  `test-support`. `GameplayStateSnapshot` reads canonical resources/components;
  `UiTreeSnapshot` reads presentation structure only.
- Deterministic combat and balance evidence belongs to the rules, contracts, and
  simulation partitions.
- `walks/gameplay_ui.ron` is the sole gameplay presentation walk and reviews at most
  ten frames. It may judge layout, hierarchy, legibility, focus, contrast, and
  responsive reflow only.

A screenshot must never prove legality, budgets, decisions, damage, Channel,
outcomes, persistence, deployment, or report identity. The visual runner therefore
has no combat-solving verbs; presentation fixtures open authored states while typed
tests prove their canonical facts.

Forest, Waterfall, map-review, V3, and world-owned captures remain outside this
contract and are unchanged.

## Screen audit

| Surface | Primary task | Persistent action | Secondary content |
|---|---|---|---|
| Splash/loading | understand progress | none | none |
| Title | choose route | selected route | development scenario lists |
| Settings | change one preference | Back | persistence notice |
| Creators | finish the current authoring step | Back / Next / Confirm | optional details and history |
| Combat Lab setup | choose fixture/profile and deploy | Back / Launch | fixture explanation and tuning |
| Gameplay | act for the current unit | Now / Choose / Confirm rail | inspector, log, statistics |
| Pause | resume, save, or leave | Resume | save notice |
| Report / Compare | inspect one axis at a time | Back / Retry / Copy / Tune | alternate report views |

When a screen changes, review it against the hierarchy, focus, compact reflow, and
oracle boundary above before adding a screen-specific workaround.
