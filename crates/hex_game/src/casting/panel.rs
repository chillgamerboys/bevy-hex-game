//! The spell panel: what this unit can cast, and why not when it cannot.
//!
//! Built on the lattice demo's shape, because that screen already solved this problem:
//! a row per spell, a fixed-width action slot holding either a cast button or the live
//! blocked reason from `castable`, and the spell's **name** on the button, since entity
//! order is not stable across the wholesale rebuilds this kind of readout does.
//!
//! # Everything here is deaf to picking
//!
//! The panel floats over the map, and the map is clicked. Every layer of it therefore
//! carries `Pickable::IGNORE` except the buttons themselves — without that the panel
//! swallows tile clicks and click-to-move silently stops working underneath it, which
//! is a bug this codebase has already shipped once with the HUD.
//!
//! The text helpers in [`widgets`](crate::menus::widgets) already carry that marker, so
//! only the raw nodes here add it. Adding it twice is not harmless: a bundle with two
//! of one component panics on spawn.

use bevy::picking::Pickable;
use bevy::prelude::*;
use hex_core::Screen;

use crate::menus::widgets::{
    blurb, divider, fine, heading, label, row_button, small_button, UiAssets, EDGE, PANEL_BG,
    SMALL_BUTTON_WIDTH,
};
use crate::screens::DespawnOnExit;

use super::preview::AimVolume;
use super::{AimControl, Aiming, AimsSpell, CastReadout, SpellRow};

/// Width of the panel. Wide enough for the demo's 132px action slot beside a spell's
/// name and a line of detail, and no wider — it is sitting on top of the game.
const PANEL_WIDTH: f32 = 396.0;

/// Width of the panel's content, inside its padding.
const CONTENT_WIDTH: f32 = PANEL_WIDTH - 28.0;

/// Width of the three aim controls, which have to fit side by side inside the content.
const CONTROL_WIDTH: f32 = 112.0;

/// Width of the colour bar carrying a spell's element.
const SWATCH_WIDTH: f32 = 5.0;

/// The stable container the rows are rebuilt under.
#[derive(Component)]
pub(super) struct PanelBody;

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
) {
    *readout = CastReadout::default();
    aiming.0 = None;

    commands
        .spawn((
            Name::new("Casting Panel"),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(12.0),
                right: Val::Px(12.0),
                width: Val::Px(PANEL_WIDTH),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(14.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            },
            BorderColor::all(EDGE),
            BackgroundColor(PANEL_BG),
            // Without this the panel swallows clicks on any tile behind it, and
            // click-to-move silently stops working across a quarter of the screen.
            Pickable::IGNORE,
            DespawnOnExit(Screen::Gameplay),
        ))
        .with_children(|panel| {
            panel.spawn((heading(&assets, "casting"), Pickable::IGNORE));
            panel.spawn((
                Name::new("Casting Body"),
                PanelBody,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        });
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
    bodies: Query<Entity, With<PanelBody>>,
    assets: Res<UiAssets>,
) {
    if !readout.is_changed() && !aiming.is_changed() && !volume.is_changed() {
        return;
    }
    let Ok(body) = bodies.single() else { return };

    commands.entity(body).despawn_related::<Children>();
    commands.entity(body).with_children(|rows| {
        if readout.caster.is_none() {
            rows.spawn(blurb(&assets, "no unit to cast from"));
            return;
        }
        if readout.spells.is_empty() {
            rows.spawn(blurb(&assets, "this unit inscribes no spells"));
            return;
        }
        if let Some(reason) = readout.unavailable {
            rows.spawn(fine(&assets, format!("· {reason}")));
        }
        for row in &readout.spells {
            spawn_row(rows, row, readout.unavailable.is_some(), &assets);
        }
        rows.spawn((divider(CONTENT_WIDTH), Pickable::IGNORE));
        spawn_footer(rows, &readout, &aiming, &volume, &assets);
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
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(10.0),
            align_items: AlignItems::Center,
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|entry| {
        entry.spawn((
            Node {
                width: Val::Px(SWATCH_WIDTH),
                height: Val::Px(44.0),
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
                        small_button(format!("Cast {}", row.name)),
                        AimsSpell(row.name.clone()),
                    ))
                    .with_children(|button| {
                        button.spawn(blurb(assets, "aim"));
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
                        width: Val::Px(SMALL_BUTTON_WIDTH),
                        ..default()
                    },
                    Pickable::IGNORE,
                    children![fine(
                        assets,
                        // A spell the lattice cannot pay for says so; one held back
                        // only by whose turn it is keeps showing its price, because
                        // that reason is already on the line above the list.
                        blocked.map_or_else(
                            || row.cost.clone(),
                            |reason| format!("blocked · {reason}")
                        )
                    )],
                ));
            }
        }

        entry.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                ..default()
            },
            Pickable::IGNORE,
            children![
                label(assets, row.name.clone()),
                fine(assets, row.detail.clone())
            ],
        ));
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

    rows.spawn(blurb(assets, format!("aiming {}", aim.spell)));
    // The anchor is written as a whole `TilePos`, level included. A bare coordinate
    // would read identically for a bridge deck and the ground beneath it, which is the
    // one distinction a player aiming down a shaft most needs.
    rows.spawn(fine(
        assets,
        format!(
            "anchor ({}, {}) level {}   ·   {} voxels, {} on surfaces",
            aim.anchor.coord.x(),
            aim.anchor.coord.y(),
            aim.anchor.level,
            volume.voxels,
            volume.painted
        ),
    ));
    if volume.painted == 0 {
        // Otherwise a spell aimed into open air looks like a preview that failed.
        rows.spawn(fine(assets, "· nothing on that volume to paint"));
    }
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
    rows.spawn(fine(assets, "click a lit surface to aim somewhere else"));
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

    /// Every layer of the panel except its buttons must let world picks through.
    ///
    /// Pickability is per entity, so ignoring only the backing node still leaves a row,
    /// a swatch or a line of text able to swallow a tile click — and the panel covers a
    /// quarter of the screen. The HUD shipped exactly this bug once.
    #[test]
    fn the_casting_panel_does_not_block_tile_clicks() {
        let mut app = drawn_panel(true);
        let mut nodes = app
            .world_mut()
            .query_filtered::<(Entity, Option<&Pickable>, Has<Button>, Option<&Name>), With<Node>>(
            );
        let mut checked = 0;
        for (entity, pickable, is_button, name) in nodes.iter(app.world()) {
            if is_button {
                continue;
            }
            checked += 1;
            assert_eq!(
                pickable,
                Some(&Pickable::IGNORE),
                "{:?} ({:?}) blocks world picks",
                name.map(Name::as_str),
                entity
            );
        }
        // Otherwise a panel that stopped drawing anything would pass this by checking
        // nothing at all, which is the failure it exists to prevent one level up.
        assert!(checked >= 8, "only {checked} non-button nodes were checked");
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
}
