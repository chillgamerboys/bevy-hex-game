//! The spell panel: what this unit can cast, and why not when it cannot.
//!
//! Built on the lattice demo's shape, because that screen already solved this problem:
//! a row per spell, a fixed-width action slot holding either a cast button or the live
//! blocked reason from `castable`, and the spell's **name** on the button, since entity
//! order is not stable across the wholesale rebuilds this kind of readout does.
//!
//! # The frame absorbs clicks; everything inside it is deaf
//!
//! The panel floats over the map, and the map is clicked, so the instinct is to make the
//! whole thing `Pickable::IGNORE` the way the HUD is. That is wrong **here**, and the
//! difference is worth stating because the two look alike.
//!
//! The HUD is a thin transparent strip with nothing to click. This is a 396px column of
//! opaque chrome with buttons in it, and roughly two thirds of its area is padding, gaps,
//! swatches and labels. A click that lands there — a few pixels off a Cast button — is a
//! click the player aimed *at the panel*. Falling through to the tile behind it issues a
//! move: the unit walks somewhere the player never saw, `Busy` lands, and the aim they
//! were setting up is dropped. So [`FRAME`] blocks, and the click stops at the panel.
//!
//! Everything inside keeps `Pickable::IGNORE`, which now means "fall through to the
//! frame" rather than "fall through to the map" — the buttons are the only children that
//! handle anything, and the frame catches the rest.
//!
//! The text helpers in [`hex_ui`] already carry that marker, so
//! only the raw nodes here add it. Adding it twice is not harmless: a bundle with two
//! of one component panics on spawn.

use bevy::picking::Pickable;
use bevy::prelude::*;
use hex_combat::TurnOrder;
use hex_core::{CommandQueue, ControlOwner, GameCommand, IssuedCommand, Mode, PendingDecision};
use hex_units::{Faction, UnitRegistry};

use crate::readouts::{
    region, DecisionHud, DisableSelection, GameplayUiContext, HudElement, HudRegion,
};
use hex_ui::{
    blurb, fine, heading, row_button, spawn_decision_controls, stacked_row_button, UiAssets,
    BLURB_SIZE, EDGE, LABEL, PANEL_BG,
};

use super::preview::AimVolume;
use super::{AimControl, Aiming, AimsSpell, CastReadout, SpellRow};

/// Width of the three aim controls, which have to fit side by side inside the content.
const CONTROL_WIDTH: f32 = 104.0;

/// Width of the colour bar carrying a spell's element.
const SWATCH_WIDTH: f32 = 5.0;

/// Picking for the panel frame: swallow the click, but do not light up under the cursor.
///
/// `should_block_lower` is the whole point — see the module docs. `is_hoverable` stays
/// false because the frame is not a control and nothing should respond to it; only the
/// buttons inside do, and they carry their own default `Pickable`.
const FRAME: Pickable = Pickable {
    should_block_lower: true,
    is_hoverable: false,
};

/// The stable container the rows are rebuilt under.
#[derive(Component)]
pub(super) struct PanelBody;

#[derive(Component)]
pub(super) struct CastingPanel;

#[derive(Component)]
pub(super) struct EndTurnControl;

#[derive(Component)]
pub(super) struct ChannelControl;

/// Spawns the panel frame, and clears whatever the last session left in the interface.
///
/// Resetting the readout here is load-bearing rather than tidy: the rebuild is driven
/// by change detection, and re-entering gameplay with an identical readout would leave
/// a freshly spawned, permanently empty body behind a resource that never changes again.
pub(super) fn spawn_panel(
    mut commands: Commands,
    mut readout: ResMut<CastReadout>,
    mut aiming: ResMut<Aiming>,
    assets: Res<UiAssets>,
    regions: Query<(Entity, &HudRegion)>,
) {
    *readout = CastReadout::default();
    aiming.0 = None;

    let panel = commands
        .spawn((
            Name::new("Casting Panel"),
            CastingPanel,
            DecisionHud,
            HudElement,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                height: Val::Px(126.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::axes(Val::Px(9.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            },
            BorderColor::all(EDGE),
            BackgroundColor(PANEL_BG),
            FRAME,
        ))
        .with_children(|panel| {
            panel.spawn(heading(&assets, "actions"));
            panel.spawn((
                Name::new("Casting Body"),
                PanelBody,
                Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    align_items: AlignItems::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        })
        .id();
    if let Some(actions) = region(HudRegion::Actions, &regions) {
        commands.entity(actions).add_child(panel);
    }
}

/// Redraws the panel whenever what it says has changed, and not otherwise.
///
/// A full despawn-and-respawn, like the lattice demo's readout: one dumb redraw path
/// cannot drift out of step with the state the way per-widget patching can, and all
/// three resources it reads are written only on a real change.
pub(super) fn rebuild_panel(
    mut commands: Commands,
    readout: Res<CastReadout>,
    aiming: Res<Aiming>,
    volume: Res<AimVolume>,
    mode: Res<State<Mode>>,
    context: Res<GameplayUiContext>,
    decision: Res<DisableSelection>,
    pending: Res<PendingDecision>,
    mut panels: Query<&mut Node, With<CastingPanel>>,
    bodies: Query<Entity, With<PanelBody>>,
    assets: Res<UiAssets>,
) {
    if !readout.is_changed()
        && !aiming.is_changed()
        && !volume.is_changed()
        && !mode.is_changed()
        && !context.is_changed()
        && !decision.is_changed()
        && !pending.is_changed()
    {
        return;
    }
    let Ok(mut panel) = panels.single_mut() else {
        return;
    };
    panel.display = if *mode.get() == Mode::Combat {
        Display::Flex
    } else {
        Display::None
    };
    if panel.display == Display::None {
        return;
    }
    let Ok(body) = bodies.single() else { return };

    commands.entity(body).despawn_related::<Children>();
    commands.entity(body).with_children(|rows| {
        if let Some(summary) = decision.summary() {
            let owner = context
                .decision_owner
                .as_ref()
                .map_or_else(|| "UNKNOWN ALLY".to_owned(), |unit| unit.label());
            let target = context
                .decision_target
                .as_ref()
                .map_or_else(|| "UNKNOWN TARGET".to_owned(), |unit| unit.label());
            rows.spawn(blurb(
                &assets,
                if summary.restoring {
                    format!("RESTORE TARGET · {owner} → {target}")
                } else {
                    format!("DAMAGE CHOICE · {owner}")
                },
            ));
            spawn_decision_controls(rows, summary, &assets);
            return;
        }
        if pending.is_open() {
            let owner = context
                .decision_owner
                .as_ref()
                .map_or_else(|| "UNKNOWN UNIT".to_owned(), |unit| unit.label());
            let task = match *pending {
                PendingDecision::ChooseDisables { .. } => "DAMAGE CHOICE",
                PendingDecision::ChooseRestores { .. } => "RESTORATION CHOICE",
                PendingDecision::None => unreachable!("the decision was checked as open"),
            };
            rows.spawn(blurb(
                &assets,
                format!("RESOLVING {task} · {owner} · PLAYER COMMANDS LOCKED"),
            ));
            return;
        }
        if context
            .acting
            .as_ref()
            .is_some_and(|unit| unit.faction == Faction::Hostile)
        {
            rows.spawn(blurb(&assets, "ENEMY TURN · PLAYER COMMANDS LOCKED"));
            return;
        }
        if readout.caster.is_none() {
            rows.spawn(blurb(&assets, "no unit to cast from"));
            spawn_end_turn(rows, &assets);
            return;
        }
        if readout.spells.is_empty() {
            rows.spawn(blurb(&assets, "this unit inscribes no spells"));
            spawn_turn_controls(rows, &assets);
            return;
        }
        if let Some(reason) = readout.unavailable {
            rows.spawn(blurb(&assets, reason.to_uppercase()));
        }
        if aiming.0.is_some() {
            spawn_footer(rows, &readout, &aiming, &volume, &assets);
            spawn_turn_controls(rows, &assets);
        } else {
            for row in &readout.spells {
                spawn_row(rows, row, readout.unavailable.is_some(), &assets);
            }
            spawn_turn_controls(rows, &assets);
        }
    });
}

fn spawn_turn_controls(rows: &mut ChildSpawnerCommands, assets: &UiAssets) {
    rows.spawn((stacked_row_button("Channel", 94.0), ChannelControl))
        .with_children(|button| {
            button.spawn(blurb(assets, "channel"));
            button.spawn(fine(assets, "restore mana"));
        });
    spawn_end_turn(rows, assets);
}

fn spawn_end_turn(rows: &mut ChildSpawnerCommands, assets: &UiAssets) {
    rows.spawn((stacked_row_button("End Turn", 94.0), EndTurnControl))
        .with_children(|button| {
            button.spawn(blurb(assets, "end turn"));
            button.spawn(fine(assets, "SPACE"));
        });
}

/// One spell: its element bar, its action slot, and what it is.
fn spawn_row(
    rows: &mut ChildSpawnerCommands,
    row: &SpellRow,
    unavailable: bool,
    assets: &UiAssets,
) {
    rows.spawn((
        Name::new("Spell Row"),
        Node {
            flex_basis: Val::Px(0.0),
            flex_grow: 1.0,
            min_width: Val::Px(0.0),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(4.0),
            align_items: AlignItems::Center,
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|entry| {
        entry.spawn((
            Node {
                width: Val::Px(SWATCH_WIDTH),
                height: Val::Px(74.0),
                border_radius: BorderRadius::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(row.color),
            Pickable::IGNORE,
        ));

        // The action slot is a fixed width whether it holds a button or a reason, so
        // every row aligns and nothing moves as a fight goes on.
        match (unavailable, row.blocked) {
            (false, None) => {
                // The spell's name in the button `Name` gives walk scripts a stable
                // handle — entity order is not stable across UI rebuilds.
                entry
                    .spawn((
                        Name::new(format!("Cast {}", row.name)),
                        Button,
                        AimsSpell(row.name.clone()),
                        Node {
                            width: Val::Px(148.0),
                            max_width: Val::Px(148.0),
                            flex_grow: 1.0,
                            height: Val::Px(74.0),
                            padding: UiRect::all(Val::Px(7.0)),
                            flex_direction: FlexDirection::Column,
                            justify_content: JustifyContent::Center,
                            row_gap: Val::Px(2.0),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(6.0)),
                            ..default()
                        },
                        BorderColor::all(EDGE),
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
                    ))
                    .with_children(|button| {
                        button.spawn((
                            Text::new(row.name.clone()),
                            TextFont {
                                font: assets.body.clone().into(),
                                ..TextFont::from_font_size(BLURB_SIZE)
                            },
                            TextColor(LABEL),
                            Pickable::IGNORE,
                        ));
                        button.spawn(fine(assets, row.cost.clone()));
                    });
            }
            (_, blocked) => {
                // The reason takes the button's exact width, so every row lines up
                // whether or not its spell can be cast — a slot that collapsed when a
                // spell went down would make the whole list jump exactly when a fight
                // starts going badly.
                entry.spawn((
                    Name::new("Blocked Reason"),
                    Node {
                        width: Val::Px(148.0),
                        max_width: Val::Px(148.0),
                        flex_grow: 1.0,
                        height: Val::Px(74.0),
                        padding: UiRect::all(Val::Px(7.0)),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        ..default()
                    },
                    BorderColor::all(EDGE),
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.04)),
                    Pickable::IGNORE,
                    children![
                        blurb(assets, row.name.clone()),
                        fine(
                            assets,
                            blocked.map_or_else(
                                || row.cost.clone(),
                                |reason| format!("BLOCKED · {reason}")
                            )
                        )
                    ],
                ));
            }
        }
    });
}

pub(super) fn channel_from_button(
    clicks: Query<&Interaction, (Changed<Interaction>, With<ChannelControl>)>,
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    registry: Res<UnitRegistry>,
    owners: Query<(Option<&ControlOwner>, &Faction)>,
    mut queue: ResMut<CommandQueue>,
) {
    queue_current_player_command(
        clicks
            .iter()
            .any(|interaction| *interaction == Interaction::Pressed),
        &order,
        &pending,
        &registry,
        &owners,
        &mut queue,
        |unit| GameCommand::Channel { unit },
    );
}

pub(super) fn end_turn_from_button(
    clicks: Query<&Interaction, (Changed<Interaction>, With<EndTurnControl>)>,
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    registry: Res<UnitRegistry>,
    owners: Query<(Option<&ControlOwner>, &Faction)>,
    mut queue: ResMut<CommandQueue>,
) {
    queue_current_player_command(
        clicks
            .iter()
            .any(|interaction| *interaction == Interaction::Pressed),
        &order,
        &pending,
        &registry,
        &owners,
        &mut queue,
        |unit| GameCommand::EndTurn { unit },
    );
}

pub(crate) fn queue_current_player_command(
    requested: bool,
    order: &TurnOrder,
    pending: &PendingDecision,
    registry: &UnitRegistry,
    owners: &Query<(Option<&ControlOwner>, &Faction)>,
    queue: &mut CommandQueue,
    command: impl FnOnce(hex_core::UnitId) -> GameCommand,
) {
    if pending.is_open() || !requested {
        return;
    }
    let Some(unit) = order.current() else {
        return;
    };
    let Some(entity) = registry.entity_of(unit) else {
        return;
    };
    let Ok((owner, faction)) = owners.get(entity) else {
        return;
    };
    if *faction != Faction::Player {
        return;
    }
    queue.push(IssuedCommand {
        seat: owner.copied().unwrap_or_default().0,
        command: command(unit),
    });
}

/// The aim in flight, and what can be done with it.
fn spawn_footer(
    rows: &mut ChildSpawnerCommands,
    readout: &CastReadout,
    aiming: &Aiming,
    volume: &AimVolume,
    assets: &UiAssets,
) {
    let Some(aim) = aiming.0.as_ref() else {
        rows.spawn(blurb(assets, "pick a spell to aim it"));
        return;
    };

    rows.spawn(blurb(
        assets,
        format!(
            "AIMING {} · {} VOXELS / {} SURFACES",
            aim.spell.to_uppercase(),
            volume.voxels,
            volume.painted
        ),
    ));
    if readout.unavailable.is_some() {
        return;
    }

    rows.spawn((
        Name::new("Aim Controls"),
        Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|controls| {
        controls
            .spawn((
                row_button("Confirm Cast", CONTROL_WIDTH),
                AimControl::Confirm,
            ))
            .with_children(|button| {
                button.spawn(blurb(assets, "cast"));
                button.spawn(fine(assets, "ENTER"));
            });
        controls
            .spawn((row_button("Next Target", CONTROL_WIDTH), AimControl::Next))
            .with_children(|button| {
                button.spawn(blurb(assets, "next"));
                button.spawn(fine(assets, "TAB"));
            });
        controls
            .spawn((row_button("Cancel Aim", CONTROL_WIDTH), AimControl::Cancel))
            .with_children(|button| {
                button.spawn(blurb(assets, "cancel"));
                button.spawn(fine(assets, "Q"));
            });
    });
}

#[cfg(test)]
mod tests {
    use bevy::MinimalPlugins;
    use hex_assets::TargetShape;
    use hex_core::{HexCoord, PlayerSeat, TilePos, UnitId};

    use crate::casting::{Aim, Caster, SpellRow};

    use super::*;

    fn row(name: &str, blocked: Option<&'static str>) -> SpellRow {
        SpellRow {
            name: name.to_owned(),
            detail: "tier 1 · evocation · Fire".to_owned(),
            cost: "1 mana · range 3 · one voxel".to_owned(),
            blocked,
            color: Color::WHITE,
            range: 3,
            shape: TargetShape::Single,
        }
    }

    /// The panel, drawn once for a caster with one castable and one blocked spell.
    fn drawn_panel(aiming: bool) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        });
        app.init_resource::<CastReadout>();
        app.init_resource::<Aiming>();
        app.init_resource::<AimVolume>();
        app.init_resource::<GameplayUiContext>();
        app.init_resource::<DisableSelection>();
        app.init_resource::<PendingDecision>();
        app.insert_resource(State::new(Mode::Combat));
        app.add_systems(Startup, spawn_panel);
        app.add_systems(Update, rebuild_panel);
        app.update();

        app.insert_resource(CastReadout {
            caster: Some(Caster {
                unit: UnitId(1),
                seat: PlayerSeat::default(),
                standing: TilePos::new(HexCoord::ORIGIN, 4),
            }),
            unavailable: None,
            spells: vec![
                row("Ember", None),
                row("Renewal", Some("spell hex disabled")),
            ],
            levels_per_bonus: 5,
        });
        if aiming {
            app.insert_resource(Aiming(Some(Aim {
                spell: "Ember".to_owned(),
                anchor: TilePos::new(HexCoord::from_axial(1, -1), 4),
            })));
        }
        app.update();
        app
    }

    /// The frame swallows a click; everything inside it defers to the frame.
    ///
    /// Pickability is per entity, so this has to hold for every node, not just the
    /// backing one. Two different failures are in scope. A frame that stopped blocking
    /// would send a near-miss on a Cast button to the tile behind it, walking the unit
    /// somewhere the player never picked and dropping the aim they were mid-way through
    /// setting up. An *inner* node that blocked would be worse in the other direction:
    /// it would shadow whatever sits under it, so a button's own label could eat the
    /// press meant for the button.
    #[test]
    fn the_frame_absorbs_clicks_and_nothing_inside_it_does() {
        let mut app = drawn_panel(true);
        let mut nodes = app
            .world_mut()
            .query_filtered::<(Entity, Option<&Pickable>, Has<Button>, Option<&Name>), With<Node>>(
            );
        let mut frames = 0;
        let mut inner = 0;
        for (entity, pickable, is_button, name) in nodes.iter(app.world()) {
            if is_button {
                continue;
            }
            let named = name.map(Name::as_str);
            if named == Some("Casting Panel") {
                frames += 1;
                assert_eq!(
                    pickable,
                    Some(&FRAME),
                    "the frame must absorb the click rather than pass it to the map"
                );
                continue;
            }
            inner += 1;
            assert_eq!(
                pickable,
                Some(&Pickable::IGNORE),
                "{named:?} ({entity:?}) shadows what is under it"
            );
        }
        assert_eq!(frames, 1, "exactly one frame, and it was found");
        // Otherwise a panel that stopped drawing anything would pass this by checking
        // nothing at all, which is the failure it exists to prevent one level up.
        assert!(inner >= 8, "only {inner} inner nodes were checked");
    }

    /// A castable spell gets a button whose `Name` a walk script can find, and a
    /// blocked one gets its reason in the same slot instead.
    #[test]
    fn a_row_is_a_button_or_a_reason() {
        let mut app = drawn_panel(false);
        let names: Vec<String> = app
            .world_mut()
            .query_filtered::<&Name, With<Button>>()
            .iter(app.world())
            .map(|name| name.as_str().to_owned())
            .collect();
        assert!(
            names.contains(&"Cast Ember".to_owned()),
            "a castable spell should offer a handle a walk can click: {names:?}"
        );
        assert!(
            !names.contains(&"Cast Renewal".to_owned()),
            "a blocked spell must not offer a button: {names:?}"
        );

        let texts: Vec<String> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|text| text.0.clone())
            .collect();
        assert!(
            texts.iter().any(|line| line.contains("spell hex disabled")),
            "a blocked spell should say why: {texts:?}"
        );
    }

    /// Aiming puts the three controls up, and stopping takes them down again.
    #[test]
    fn the_aim_controls_appear_only_while_a_spell_is_aimed() {
        for (aiming, expected) in [(false, false), (true, true)] {
            let mut app = drawn_panel(aiming);
            let present = app
                .world_mut()
                .query_filtered::<&Name, With<Button>>()
                .iter(app.world())
                .any(|name| name.as_str() == "Confirm Cast");
            assert_eq!(present, expected, "aiming was {aiming}");
        }
    }

    #[test]
    fn a_hostile_turn_replaces_player_commands_with_a_lock_message() {
        let mut app = drawn_panel(false);
        app.world_mut().resource_mut::<GameplayUiContext>().acting =
            Some(crate::readouts::UiUnitIdentity {
                unit: UnitId(9),
                name: "raider #9".to_owned(),
                faction: Faction::Hostile,
                party_slot: None,
            });
        app.update();

        let names: Vec<_> = app
            .world_mut()
            .query_filtered::<&Name, With<Button>>()
            .iter(app.world())
            .map(|name| name.as_str().to_owned())
            .collect();
        assert!(!names.iter().any(|name| name.starts_with("Cast ")));
        assert!(!names.iter().any(|name| name == "End Turn"));
        let texts: Vec<_> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|text| text.0.clone())
            .collect();
        assert!(texts.iter().any(|text| text.contains("COMMANDS LOCKED")));
    }

    #[test]
    fn a_hostile_ai_decision_never_exposes_player_commands() {
        let mut app = drawn_panel(false);
        let hostile = crate::readouts::UiUnitIdentity {
            unit: UnitId(9),
            name: "raider #9".to_owned(),
            faction: Faction::Hostile,
            party_slot: None,
        };
        app.world_mut()
            .resource_mut::<GameplayUiContext>()
            .decision_owner = Some(hostile.clone());
        app.world_mut()
            .resource_mut::<GameplayUiContext>()
            .decision_target = Some(hostile);
        *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
            decider: UnitId(9),
            count: 1,
            source: UnitId(1),
        };
        app.update();

        let names: Vec<_> = app
            .world_mut()
            .query_filtered::<&Name, With<Button>>()
            .iter(app.world())
            .map(|name| name.as_str().to_owned())
            .collect();
        assert!(!names.iter().any(|name| name.starts_with("Cast ")));
        assert!(!names.iter().any(|name| name == "End Turn"));
        let texts: Vec<_> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|text| text.0.clone())
            .collect();
        assert!(texts.iter().any(|text| {
            text.contains("RESOLVING DAMAGE CHOICE")
                && text.contains("HOSTILE · RAIDER #9")
                && text.contains("COMMANDS LOCKED")
        }));
    }

    #[test]
    fn a_player_without_spells_still_has_an_end_turn_control() {
        let mut app = drawn_panel(false);
        app.world_mut().resource_mut::<CastReadout>().spells.clear();
        app.update();

        let names: Vec<_> = app
            .world_mut()
            .query_filtered::<&Name, With<Button>>()
            .iter(app.world())
            .map(|name| name.as_str().to_owned())
            .collect();
        assert!(names.iter().any(|name| name == "End Turn"));
        assert!(!names.iter().any(|name| name.starts_with("Cast ")));
    }

    #[test]
    fn a_decision_replaces_ordinary_actions_with_clear_and_confirm() {
        let mut app = drawn_panel(false);
        app.world_mut()
            .resource_mut::<DisableSelection>()
            .begin_test_decision(UnitId(1), UnitId(2), 1, true);
        app.update();

        let names: Vec<_> = app
            .world_mut()
            .query_filtered::<&Name, With<Button>>()
            .iter(app.world())
            .map(|name| name.as_str().to_owned())
            .collect();
        assert!(!names.iter().any(|name| name.starts_with("Cast ")));
        assert!(!names.iter().any(|name| name == "End Turn"));
        assert!(names.iter().any(|name| name == "Clear Disable Selection"));
        assert!(
            !names.iter().any(|name| name == "Confirm Disable Selection"),
            "confirmation stays disabled until the quota is selected"
        );

        let disabled = app
            .world_mut()
            .query::<(Entity, &Name)>()
            .iter(app.world())
            .find_map(|(entity, name)| {
                (name.as_str() == "Confirm Disable Selection Disabled").then_some(entity)
            })
            .expect("the disabled confirmation should be drawn");
        let node = app
            .world()
            .entity(disabled)
            .get::<Node>()
            .expect("the disabled confirmation is a UI node");
        assert_eq!(
            node.flex_direction,
            FlexDirection::Column,
            "the hint belongs below confirm rather than overflowing beside it"
        );
    }
}
