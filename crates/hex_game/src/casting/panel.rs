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
            FRAME,
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
}
