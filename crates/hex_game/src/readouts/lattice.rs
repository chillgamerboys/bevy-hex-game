//! The player's live lattice and the retained, knowledge-gated hostile target.

use bevy::picking::Pickable;
use bevy::prelude::*;
use hex_assets::{ElementCatalog, SpellBook};
use hex_combat::{CombatSystems, FactionKnowledge};
use hex_core::{AppSystems, Mode, PendingDecision, Screen, UnitId};
use hex_lattice::{LatticeSpec, LatticeState, LatticeStats};
use hex_units::{Downed, Faction, Player, Selected, StandsOn, UnitRegistry};

use crate::casting::{Aiming, CastReadout};
use crate::menus::lattice_view::{
    known_cell_view, live_cell_view, spawn_lattice_cells, CellInteraction, LatticeCellView,
    LatticeScale,
};
use crate::menus::widgets::{blurb, fine, heading, panel, UiAssets, PANEL_BG};
use crate::readouts::HudElement;
use crate::screens::DespawnOnExit;

const PANEL_WIDTH: f32 = 286.0;
const TARGET_TOP: f32 = 326.0;
const PULSE_COLOR: Color = Color::srgba(0.25, 0.10, 0.06, 0.9);
const FRAME: Pickable = Pickable {
    should_block_lower: true,
    is_hoverable: false,
};

#[derive(Resource, Default, Debug, PartialEq)]
struct LatticeReadouts {
    own: Option<OwnLattice>,
    target: Option<TargetLattice>,
}

#[derive(Debug, PartialEq)]
struct OwnLattice {
    unit: UnitId,
    name: String,
    cells: Vec<LatticeCellView>,
}

#[derive(Debug, PartialEq)]
struct TargetLattice {
    unit: UnitId,
    name: String,
    state: TargetState,
}

#[derive(Debug, PartialEq)]
enum TargetState {
    Opaque,
    Known {
        cells: Vec<LatticeCellView>,
        unknown: Option<usize>,
    },
}

/// The last hostile a real aim named. Empty anchors never erase it.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedTarget(pub(crate) Option<UnitId>);

#[derive(Component)]
struct OwnPanel;

#[derive(Component)]
pub(super) struct TargetPanel;

#[derive(Component)]
struct OwnBody;

#[derive(Component)]
struct TargetBody;

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<LatticeReadouts>()
        .init_resource::<RetainedTarget>()
        .add_systems(OnEnter(Screen::Gameplay), spawn_panels)
        .add_systems(
            Update,
            (retain_target, refresh_readouts)
                .chain()
                .in_set(AppSystems::Update)
                .after(CombatSystems::Advance)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            rebuild_panels
                .after(refresh_readouts)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(OnExit(Screen::Gameplay), clear_focus);
}

fn spawn_panels(
    mut commands: Commands,
    mut readouts: ResMut<LatticeReadouts>,
    mut focus: ResMut<RetainedTarget>,
    assets: Res<UiAssets>,
) {
    *readouts = LatticeReadouts::default();
    *focus = RetainedTarget::default();

    commands
        .spawn((
            Name::new("Own Lattice Panel"),
            OwnPanel,
            HudElement,
            panel(),
            FRAME,
            DespawnOnExit(Screen::Gameplay),
        ))
        .insert(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(12.0),
            width: Val::Px(PANEL_WIDTH),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(12.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            row_gap: Val::Px(7.0),
            ..default()
        })
        .with_children(|panel| {
            panel.spawn(heading(&assets, "your lattice"));
            panel.spawn((Name::new("Own Lattice Body"), OwnBody, Pickable::IGNORE));
        });

    commands
        .spawn((
            Name::new("Target Lattice Panel"),
            TargetPanel,
            HudElement,
            panel(),
            FRAME,
            DespawnOnExit(Screen::Gameplay),
        ))
        .insert(Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: Val::Px(TARGET_TOP),
            left: Val::Px(12.0),
            width: Val::Px(PANEL_WIDTH),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(12.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            row_gap: Val::Px(7.0),
            ..default()
        })
        .with_children(|panel| {
            panel.spawn(heading(&assets, "target lattice"));
            panel.spawn((
                Name::new("Target Lattice Body"),
                TargetBody,
                Pickable::IGNORE,
            ));
        });
}

/// Updates retention from an actual hostile occupying the aimed surface.
pub(super) fn retain_target(
    mode: Res<State<Mode>>,
    aiming: Res<Aiming>,
    mut focus: ResMut<RetainedTarget>,
    units: Query<(&UnitId, &Faction, &StandsOn), Without<Downed>>,
) {
    if *mode.get() != Mode::Combat {
        focus.0 = None;
        return;
    }
    let Some(aim) = aiming.0.as_ref() else {
        return;
    };
    if let Some((unit, _, _)) = units.iter().find(|(_, faction, standing)| {
        Faction::Player.is_hostile_to(**faction) && standing.0.pos == aim.anchor
    }) {
        focus.0 = Some(*unit);
    }
}

type OwnData<'w, 's> = Query<
    'w,
    's,
    (
        &'static UnitId,
        &'static Name,
        &'static LatticeSpec,
        &'static LatticeState,
        &'static LatticeStats,
    ),
    With<Player>,
>;

fn refresh_readouts(
    mut readouts: ResMut<LatticeReadouts>,
    focus: Res<RetainedTarget>,
    pending: Res<PendingDecision>,
    casting: Res<CastReadout>,
    registry: Res<UnitRegistry>,
    knowledge: Res<FactionKnowledge>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
    own: OwnData,
    selected: Query<&UnitId, (With<Player>, With<Selected>)>,
    identities: Query<(&Name, &Faction)>,
) {
    let (Some(elements), Some(spells)) = (elements, spells) else {
        return;
    };

    let own_unit = own_focus(
        player_decider(&pending, &registry, &own),
        casting.caster.map(|caster| caster.unit),
        selected.iter().copied().next(),
    );
    let own_view = own_unit
        .and_then(|unit| registry.entity_of(unit))
        .and_then(|entity| own.get(entity).ok())
        .map(|(unit, name, spec, state, stats)| OwnLattice {
            unit: *unit,
            name: name.as_str().to_owned(),
            cells: spec
                .cells()
                .map(|(coord, kind)| {
                    live_cell_view(
                        coord,
                        kind,
                        stats,
                        state,
                        &elements,
                        &spells,
                        CellInteraction::ReadOnly,
                        false,
                    )
                })
                .collect(),
        });

    // No hostile `LatticeSpec` or `LatticeState` appears in this function. The
    // target projection can only be assembled from `FactionKnowledge::view`.
    let target_view = focus.0.and_then(|unit| {
        let entity = registry.entity_of(unit)?;
        let (name, faction) = identities.get(entity).ok()?;
        if !Faction::Player.is_hostile_to(*faction) {
            return None;
        }
        let known = knowledge.view(Faction::Player, unit)?;
        let state = if known.is_opaque() {
            TargetState::Opaque
        } else {
            TargetState::Known {
                cells: known
                    .cells()
                    .map(|(coord, cell)| {
                        known_cell_view(
                            coord,
                            cell.kind,
                            cell.mana,
                            None,
                            cell.disabled,
                            &elements,
                            &spells,
                        )
                    })
                    .collect(),
                unknown: known.unknown_count(),
            }
        };
        Some(TargetLattice {
            unit,
            name: name.as_str().to_owned(),
            state,
        })
    });

    let next = LatticeReadouts {
        own: own_view,
        target: target_view,
    };
    if *readouts != next {
        *readouts = next;
    }
}

fn player_decider(
    pending: &PendingDecision,
    registry: &UnitRegistry,
    own: &OwnData,
) -> Option<UnitId> {
    let unit = pending.decider()?;
    let entity = registry.entity_of(unit)?;
    own.contains(entity).then_some(unit)
}

fn own_focus(
    player_decider: Option<UnitId>,
    casting_unit: Option<UnitId>,
    selected_player: Option<UnitId>,
) -> Option<UnitId> {
    player_decider.or(casting_unit).or(selected_player)
}

fn rebuild_panels(
    mut commands: Commands,
    readouts: Res<LatticeReadouts>,
    own_bodies: Query<Entity, With<OwnBody>>,
    target_bodies: Query<Entity, With<TargetBody>>,
    mut target_panels: Query<&mut Node, With<TargetPanel>>,
    assets: Res<UiAssets>,
) {
    if !readouts.is_changed() {
        return;
    }

    if let Ok(body) = own_bodies.single() {
        commands.entity(body).despawn_related::<Children>();
        commands.entity(body).with_children(|body| {
            let Some(own) = readouts.own.as_ref() else {
                body.spawn(blurb(&assets, "no player lattice"));
                return;
            };
            body.spawn(fine(&assets, own.name.clone()));
            spawn_lattice_cells(
                body,
                &own.cells,
                &assets,
                LatticeScale::PANEL,
                "Own",
                |_| (),
            );
        });
    }

    let has_target = readouts.target.is_some();
    if let Ok(mut node) = target_panels.single_mut() {
        node.display = if has_target {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(body) = target_bodies.single() {
        commands.entity(body).despawn_related::<Children>();
        commands.entity(body).with_children(|body| {
            let Some(target) = readouts.target.as_ref() else {
                return;
            };
            body.spawn(fine(&assets, target.name.clone()));
            match &target.state {
                TargetState::Opaque => {
                    body.spawn(blurb(&assets, "lattice unknown"));
                }
                TargetState::Known { cells, unknown } => {
                    spawn_lattice_cells(
                        body,
                        cells,
                        &assets,
                        LatticeScale::PANEL,
                        "Target",
                        |_| (),
                    );
                    if let Some(unknown) = unknown.filter(|unknown| *unknown > 0) {
                        body.spawn(fine(&assets, format!("{unknown} cells unknown")));
                    }
                }
            }
        });
    }
}

fn clear_focus(mut focus: ResMut<RetainedTarget>) {
    focus.0 = None;
}

pub(super) fn set_pulse_color(
    active: bool,
    panels: &mut Query<&mut BackgroundColor, With<TargetPanel>>,
) {
    let Ok(mut background) = panels.single_mut() else {
        return;
    };
    background.0 = if active { PULSE_COLOR } else { PANEL_BG };
}

#[cfg(test)]
mod tests {
    use hex_core::{HexCoord, TilePos};

    use super::*;

    fn pos(x: i32, y: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(x, y), 0)
    }

    #[test]
    fn an_empty_anchor_does_not_replace_the_retained_target() {
        let mut focus = RetainedTarget(Some(UnitId(7)));
        let aimed = Some(pos(9, 9));
        let units = [(UnitId(3), Faction::Hostile, pos(1, 0))];
        update_retained(&mut focus, aimed, &units);
        assert_eq!(focus.0, Some(UnitId(7)));
    }

    #[test]
    fn a_hostile_on_the_anchor_replaces_the_retained_target() {
        let mut focus = RetainedTarget(Some(UnitId(7)));
        let units = [(UnitId(3), Faction::Hostile, pos(1, 0))];
        update_retained(&mut focus, Some(pos(1, 0)), &units);
        assert_eq!(focus.0, Some(UnitId(3)));
    }

    #[test]
    fn own_focus_prefers_the_decider_then_caster_then_selection() {
        let decider = UnitId(1);
        let caster = UnitId(2);
        let selected = UnitId(3);
        assert_eq!(
            own_focus(Some(decider), Some(caster), Some(selected)),
            Some(decider)
        );
        assert_eq!(own_focus(None, Some(caster), Some(selected)), Some(caster));
        assert_eq!(own_focus(None, None, Some(selected)), Some(selected));
        assert_eq!(own_focus(None, None, None), None);
    }

    fn update_retained(
        focus: &mut RetainedTarget,
        aimed: Option<TilePos>,
        units: &[(UnitId, Faction, TilePos)],
    ) {
        let Some(anchor) = aimed else { return };
        if let Some((unit, _, _)) = units.iter().find(|(_, faction, standing)| {
            Faction::Player.is_hostile_to(*faction) && *standing == anchor
        }) {
            focus.0 = Some(*unit);
        }
    }
}
