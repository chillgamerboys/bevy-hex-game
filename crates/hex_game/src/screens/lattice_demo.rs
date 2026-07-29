//! Interactive sandbox for the lattice ruleset, reachable from the title menu.
//!
//! Builds a small demonstration lattice out of the *real* content in
//! `elements.ron` / `spells.ron` and lets a human exercise every rule the
//! content can currently reach: binary casting with live blocked-reasons, mana
//! capacity versus channelling throughput, damage disables, and enchantment
//! mana locking. Fusion resolution now has content behind it — the hedge-mage's
//! Lightning Bolt is the one shipped spell requiring a higher-order element —
//! though this demo's own fixture does not use it. **Burn is not here at all**: it
//! stopped being something a lattice carries and now lives in `hex_combat`'s effect
//! ledger, which this screen deliberately cannot see — a burn needs a turn order to
//! tick against, and a sandbox has none. Nothing
//! persists — the lattice is rebuilt from content on every entry, and `Reset`
//! rebuilds the battle state. Reset is no longer the *only* way back from a
//! strike (`hex_lattice::restore` exists now), but it stays the demo's, because
//! a restoring spell is a cast and the demo has no target but itself.
//!
//! # The tables adapter is no longer demo-local
//!
//! This screen used to carry its own copy of the `SpellTable`/`FusionTable`
//! adapter, because the permanent seat belonged to a ticket that had not
//! landed. It has: [`ContentTables`] lives in
//! `hex_assets` beside the content it reads, and the demo uses the same one
//! the game does. Two copies of the unknown-spell fallback was one fix away
//! from drifting apart.

use std::collections::BTreeMap;

use bevy::prelude::*;
use hex_assets::{ContentIndex, ContentTables, ElementCatalog, SpellBook};
use hex_core::{ElementId, LatticeCoord, Screen};
use hex_lattice::{
    apply_cast, apply_disables, castable, channel, Casting, CellKind, LatticeSpec, LatticeState,
    LatticeStats, SpellTable,
};

use crate::casting::blocked_reason;
use crate::menus::lattice_view::{
    live_cell_view, spawn_lattice_cells, CellInteraction, LatticeScale,
};
use crate::menus::widgets::{
    blurb, display, divider, fine, heading, label, panel, small_button, UiAssets,
    SMALL_BUTTON_WIDTH,
};

use super::{despawn_screen, screen_root};

/// Spells the demo tries to place, chosen to cover the casting axes: a cheap
/// evocation, a defensive enchantment, a ritual, and a tier-6 evocation whose
/// cost makes `Unsatisfiable` easy to reach. Missing names are skipped and
/// reported in the demo log rather than failing the screen.
const DEMO_SPELLS: [&str; 4] = ["Ember", "Metal Shield", "Flamethrower", "Fireball"];

/// Where each placed spell sits. Spacing of four keeps one spell's ring of
/// gems from ever touching another's. A fusion's recipe gems reach two steps
/// out and *can* meet a neighbouring ring — `free_neighbor` relocates them
/// rather than overwriting, at worst leaving a spell honestly unsatisfiable.
const ANCHORS: [(i32, i32); 4] = [(0, 0), (4, 0), (0, 4), (4, 4)];

/// How many log lines the demo keeps.
const LOG_LINES: usize = 6;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::LatticeDemo), spawn_demo_screen);
    app.add_systems(
        Update,
        (
            init_demo,
            handle_cell_clicks,
            handle_cast_buttons,
            handle_action_buttons,
            rebuild_readout,
            handle_input,
        )
            .chain()
            .run_if(in_state(Screen::LatticeDemo)),
    );
    app.add_systems(
        OnExit(Screen::LatticeDemo),
        (despawn_screen(Screen::LatticeDemo), remove_demo_state),
    );
}

/// The demo's entire mutable state: inscription, stats, battle state, and log.
///
/// Rebuilt from content on every screen entry, removed on exit. `LatticeState`
/// is a `Component` in the engine's vocabulary; wrapping it in a distinct
/// resource type here keeps "no type is both Resource and Component" true.
#[derive(Resource)]
struct DemoLattice {
    spec: LatticeSpec,
    stats: LatticeStats,
    state: LatticeState,
    log: Vec<String>,
}

/// The stable container the whole readout is rebuilt under.
#[derive(Component)]
struct DemoBody;

/// A clickable lattice cell, carrying its coordinate.
#[derive(Component)]
struct DemoCell(LatticeCoord);

/// A button that attempts the cast at the given spell cell.
#[derive(Component)]
struct CastsSpell(LatticeCoord);

/// The button that channels mana back toward capacity.
#[derive(Component)]
struct EndsTurn;

/// The button that rebuilds the battle state from the inscription.
#[derive(Component)]
struct ResetsDemo;

fn spawn_demo_screen(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn(screen_root(Screen::LatticeDemo, "Lattice Demo Screen"))
        .with_children(|parent| {
            parent.spawn(display(&assets, "The Lattice"));
            parent
                .spawn((
                    Name::new("Demo Body"),
                    DemoBody,
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(32.0),
                        align_items: AlignItems::Stretch,
                        ..default()
                    },
                ))
                .with_children(|body| {
                    body.spawn(blurb(&assets, "waiting for content..."));
                });
            parent.spawn(blurb(
                &assets,
                "cast from the right panel   ·   click a gem to strike it   ·   BACKSPACE to return",
            ));
        });
}

fn remove_demo_state(mut commands: Commands) {
    commands.remove_resource::<DemoLattice>();
}

/// Builds the demo lattice once the content tables exist, and rebuilds it if
/// they change underneath it.
///
/// The title screen is reachable before the RON files finish parsing, so this
/// runs in `Update` behind `Option` guards rather than assuming the resources
/// at `OnEnter` — the same lesson the scenario list learned. The rebuild arm
/// matters for a different reason: the content builders keep running outside
/// `Screen::Gameplay` and assign ids from *sorted names*, so a hot reload can
/// reassign every id under a baked lattice and silently reinterpret it. The
/// demo is a live lattice outside the Gameplay freeze, so it re-bakes instead.
fn init_demo(
    mut commands: Commands,
    demo: Option<Res<DemoLattice>>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
    index: Option<Res<ContentIndex>>,
) {
    let (Some(elements), Some(spells), Some(index)) = (elements, spells, index) else {
        return;
    };
    let content_moved = elements.is_changed() || spells.is_changed() || index.is_changed();
    if demo.is_some() && !content_moved {
        return;
    }

    let (spec, notes) = build_demo_spec(&elements, &spells, &index);
    let stats = build_demo_stats(&spec, &spells, &index, &elements);
    let state = LatticeState::new(&spec, &stats);
    let first_line = if demo.is_some() {
        "content changed - rebuilt the lattice from the new tables".to_owned()
    } else {
        "fresh lattice - every gem at full mana".to_owned()
    };
    let mut log = vec![first_line];
    log.extend(notes);
    commands.insert_resource(DemoLattice {
        spec,
        stats,
        state,
        log,
    });
}

/// Lays out one spell cell per demo pick, ringed by the gems its requirements
/// name; a higher-order requirement gets a fusion cell with its recipe's gems
/// placed around *that*. Never overwrites an occupied cell — when a ring runs
/// out of room the spell is left honestly unsatisfiable and the log says so.
fn build_demo_spec(
    elements: &ElementCatalog,
    spells: &SpellBook,
    index: &ContentIndex,
) -> (LatticeSpec, Vec<String>) {
    let mut cells: BTreeMap<LatticeCoord, CellKind> = BTreeMap::new();
    let mut notes = Vec::new();
    let mut anchors = ANCHORS.iter();

    for name in DEMO_SPELLS {
        let Some(spell) = spells.id(name) else {
            notes.push(format!("{name}: not in spells.ron, skipped"));
            continue;
        };
        let Some(&(q, r)) = anchors.next() else {
            notes.push(format!("{name}: no anchor left, skipped"));
            continue;
        };
        let anchor = LatticeCoord::new(q, r);
        cells.insert(anchor, CellKind::Spell { spell });

        for &(element, _mana) in index.requirements(spell).unwrap_or(&[]) {
            let Some(slot) = free_neighbor(&cells, anchor) else {
                notes.push(format!("{name}: ring full, left partly unsatisfiable"));
                break;
            };
            if let Some(recipe) = elements.recipe(element) {
                cells.insert(slot, CellKind::Fusion { output: element });
                for &(input, _cost) in recipe {
                    let Some(gem_slot) = free_neighbor(&cells, slot) else {
                        notes.push(format!("{name}: fusion ring full, left unsatisfiable"));
                        break;
                    };
                    cells.insert(gem_slot, CellKind::Gem { element: input });
                }
            } else {
                cells.insert(slot, CellKind::Gem { element });
            }
        }
    }

    (LatticeSpec::new(cells), notes)
}

fn free_neighbor(
    cells: &BTreeMap<LatticeCoord, CellKind>,
    of: LatticeCoord,
) -> Option<LatticeCoord> {
    of.neighbors()
        .into_iter()
        .find(|slot| !cells.contains_key(slot))
}

/// Fabricates generous stats for the isolated lattice demonstration.
///
/// Capacity is the largest single cost the content ever asks of that element
/// (floor 3); channelling is capacity plus two, generous enough that one end
/// of turn refills what one cast drained. Gameplay uses authored archetype stats;
/// this screen intentionally remains a rules sandbox.
fn build_demo_stats(
    spec: &LatticeSpec,
    spells: &SpellBook,
    index: &ContentIndex,
    elements: &ElementCatalog,
) -> LatticeStats {
    let mut demand: BTreeMap<ElementId, u16> = BTreeMap::new();
    for (spell, _, _) in spells.iter() {
        for &(element, mana) in index.requirements(spell).unwrap_or(&[]) {
            let highest = demand.entry(element).or_insert(0);
            *highest = (*highest).max(mana);
        }
    }
    for (_, kind) in spec.cells() {
        if let CellKind::Fusion { output } = kind {
            for &(element, mana) in elements.recipe(output).unwrap_or(&[]) {
                let highest = demand.entry(element).or_insert(0);
                *highest = (*highest).max(mana);
            }
        }
    }

    let mut capacity = BTreeMap::new();
    let mut channelling = BTreeMap::new();
    for (_, kind) in spec.cells() {
        if let CellKind::Gem { element } = kind {
            let cap = demand.get(&element).copied().unwrap_or(0).max(3);
            capacity.insert(element, cap);
            channelling.insert(element, cap.saturating_add(2));
        }
    }
    LatticeStats::new(capacity, channelling)
}

/// Clicking a gem or fusion strikes it; clicking a spell cell attempts the cast.
fn handle_cell_clicks(
    demo: Option<ResMut<DemoLattice>>,
    clicked: Query<(&Interaction, &DemoCell), Changed<Interaction>>,
    index: Option<Res<ContentIndex>>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
) {
    let Some(mut demo) = demo else { return };
    let (Some(index), Some(elements), Some(spells)) = (index, elements, spells) else {
        return;
    };
    let tables = index.tables(&elements);
    for (interaction, cell) in &clicked {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match demo.spec.get(cell.0) {
            Some(CellKind::Gem { .. } | CellKind::Fusion { .. }) => {
                strike(&mut demo, cell.0, &spells);
            }
            Some(CellKind::Spell { .. }) => try_cast(&mut demo, cell.0, &tables, &spells),
            _ => {}
        }
    }
}

fn handle_cast_buttons(
    demo: Option<ResMut<DemoLattice>>,
    clicked: Query<(&Interaction, &CastsSpell), Changed<Interaction>>,
    index: Option<Res<ContentIndex>>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
) {
    let Some(mut demo) = demo else { return };
    let (Some(index), Some(elements), Some(spells)) = (index, elements, spells) else {
        return;
    };
    let tables = index.tables(&elements);
    for (interaction, cast) in &clicked {
        if *interaction == Interaction::Pressed {
            try_cast(&mut demo, cast.0, &tables, &spells);
        }
    }
}

fn handle_action_buttons(
    demo: Option<ResMut<DemoLattice>>,
    end_turns: Query<&Interaction, (Changed<Interaction>, With<EndsTurn>)>,
    resets: Query<&Interaction, (Changed<Interaction>, With<ResetsDemo>)>,
) {
    let Some(mut demo) = demo else { return };
    if end_turns
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        let DemoLattice {
            spec,
            stats,
            state,
            log,
        } = &mut *demo;
        channel(state, spec, stats);
        // No burn tick here any more. Burn stopped being something a lattice carries —
        // it lives in `hex_combat`'s effect ledger, which this screen deliberately
        // cannot see, because the demo is a sandbox for the *rules engine* rather than
        // for the fight. A burn needs a turn order to tick against, and there is not one
        // here.
        push_log(
            log,
            "end of turn: channelled mana back toward capacity".to_owned(),
        );
    }
    if resets
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        let DemoLattice {
            spec,
            stats,
            state,
            log,
        } = &mut *demo;
        *state = LatticeState::new(spec, stats);
        push_log(
            log,
            "reset: fresh battle state from the inscription".to_owned(),
        );
    }
}

fn try_cast(
    demo: &mut DemoLattice,
    cell: LatticeCoord,
    tables: &ContentTables,
    spells: &SpellBook,
) {
    let name = match demo.spec.get(cell) {
        Some(CellKind::Spell { spell }) => spells.name(spell).unwrap_or("unknown spell").to_owned(),
        _ => return,
    };
    match castable(&demo.spec, &demo.state, cell, tables) {
        Ok(plan) => {
            let cost: u32 = plan.drains.values().map(|&mana| u32::from(mana)).sum();
            let sources = plan.drains.len();
            let enchantment = matches!(tables.casting(plan.spell), Casting::Enchantment { .. });
            if apply_cast(&mut demo.state, &plan, tables) {
                if enchantment {
                    push_log(
                        &mut demo.log,
                        format!(
                            "{name}: enchantment raised - {cost} mana locked in {sources} cell(s)"
                        ),
                    );
                } else {
                    push_log(
                        &mut demo.log,
                        format!("{name}: cast - drained {cost} mana from {sources} cell(s)"),
                    );
                }
            } else {
                push_log(
                    &mut demo.log,
                    format!("{name}: plan went stale, nothing changed"),
                );
            }
        }
        Err(blocked) => {
            let reason = blocked_reason(&blocked);
            push_log(&mut demo.log, format!("{name}: blocked - {reason}"));
        }
    }
}

fn strike(demo: &mut DemoLattice, coord: LatticeCoord, spells: &SpellBook) {
    let (q, r) = (coord.q(), coord.r());
    if demo.state.is_disabled(coord) {
        push_log(&mut demo.log, format!("({q}, {r}) is already disabled"));
        return;
    }
    let broken = apply_disables(&mut demo.state, &[coord]);
    if broken.is_empty() {
        push_log(&mut demo.log, format!("struck ({q}, {r}): cell disabled"));
    } else {
        for record in broken {
            let name = spells.name(record.spell).unwrap_or("an enchantment");
            push_log(
                &mut demo.log,
                format!(
                    "struck ({q}, {r}): {name} shattered, {} locked mana burned",
                    record.burned_mana
                ),
            );
        }
    }
}

fn push_log(log: &mut Vec<String>, line: String) {
    log.push(line);
    while log.len() > LOG_LINES {
        log.remove(0);
    }
}

/// Redraws the whole readout whenever the demo state changes.
///
/// A full despawn-and-respawn is deliberate: the demo is a verification
/// surface, not a HUD, and one dumb redraw path cannot drift out of step with
/// the state the way per-widget patching can.
fn rebuild_readout(
    mut commands: Commands,
    demo: Option<Res<DemoLattice>>,
    index: Option<Res<ContentIndex>>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
    bodies: Query<Entity, With<DemoBody>>,
    assets: Res<UiAssets>,
) {
    let Some(demo) = demo else { return };
    if !demo.is_changed() {
        return;
    }
    let (Some(index), Some(elements), Some(spells)) = (index, elements, spells) else {
        return;
    };
    let Ok(body) = bodies.single() else { return };
    let tables = index.tables(&elements);

    commands.entity(body).despawn_related::<Children>();
    commands.entity(body).with_children(|panels| {
        spawn_lattice_panel(panels, &demo, &elements, &spells, &assets);
        spawn_control_panel(panels, &demo, &tables, &spells, &assets);
    });
}

fn spawn_lattice_panel(
    panels: &mut ChildSpawnerCommands,
    demo: &DemoLattice,
    elements: &ElementCatalog,
    spells: &SpellBook,
    assets: &UiAssets,
) {
    if demo.spec.capacity() == 0 {
        panels.spawn(blurb(assets, "the content defined no demo lattice"));
        return;
    }
    let views: Vec<_> = demo
        .spec
        .cells()
        .map(|(coord, kind)| {
            live_cell_view(
                coord,
                kind,
                &demo.stats,
                &demo.state,
                elements,
                spells,
                CellInteraction::Actionable,
                false,
            )
        })
        .collect();

    panels
        .spawn((Name::new("Lattice Panel"), panel()))
        .with_children(|framed| {
            framed.spawn(heading(assets, "the inscription"));
            spawn_lattice_cells(framed, &views, assets, LatticeScale::DEMO, "Demo", DemoCell);
        });
}

fn spawn_control_panel(
    panels: &mut ChildSpawnerCommands,
    demo: &DemoLattice,
    tables: &ContentTables,
    spells: &SpellBook,
    assets: &UiAssets,
) {
    panels
        .spawn((Name::new("Demo Controls"), panel()))
        .insert(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(10.0),
            width: Val::Px(470.0),
            padding: UiRect::all(Val::Px(18.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            ..default()
        })
        .with_children(|controls| {
            controls.spawn(heading(assets, "spells"));

            for (coord, kind) in demo.spec.cells() {
                let CellKind::Spell { spell } = kind else {
                    continue;
                };
                let name = spells.name(spell).unwrap_or("unknown spell");
                let kind_line = match tables.casting(spell) {
                    Casting::Enchantment { defense } => format!("enchantment · defense {defense}"),
                    Casting::Evocation => "evocation".to_owned(),
                };
                let ritual = spells
                    .spell(spell)
                    .is_some_and(hex_assets::Spell::is_ritual);
                let headline = if ritual {
                    format!("{name} (ritual)")
                } else {
                    name.to_owned()
                };

                controls
                    .spawn((
                        Name::new("Spell Row"),
                        Node {
                            height: Val::Px(50.0),
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(14.0),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ))
                    .with_children(|row| {
                        // The action slot is a button's width whether it holds a
                        // button or a blocked reason, so every row aligns.
                        match castable(&demo.spec, &demo.state, coord, tables) {
                            Ok(plan) => {
                                let cost: u32 =
                                    plan.drains.values().map(|&mana| u32::from(mana)).sum();
                                // The spell's name in the button `Name` gives
                                // walk scripts a stable handle — entity order
                                // is not stable across UI rebuilds.
                                row.spawn((
                                    small_button(format!("Cast {name}")),
                                    CastsSpell(coord),
                                ))
                                .with_children(|cast| {
                                    cast.spawn(blurb(assets, "cast"));
                                    cast.spawn(fine(assets, format!("{cost} mana")));
                                });
                            }
                            Err(blocked) => {
                                row.spawn((
                                    Name::new("Blocked Reason"),
                                    Node {
                                        width: Val::Px(SMALL_BUTTON_WIDTH),
                                        ..default()
                                    },
                                    children![fine(
                                        assets,
                                        format!("blocked · {}", blocked_reason(&blocked))
                                    )],
                                ));
                            }
                        }
                        row.spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(2.0),
                                ..default()
                            },
                            children![label(assets, headline), fine(assets, kind_line)],
                        ));
                    });
            }

            controls.spawn(divider(430.0));
            controls.spawn(blurb(
                assets,
                format!(
                    "free mana {}   ·   locked {}   ·   enchantments {}",
                    demo.state.total_gem_mana(),
                    demo.state.total_locked_mana(),
                    demo.state.enchantment_count(),
                ),
            ));

            controls
                .spawn((
                    Name::new("Demo Actions"),
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(12.0),
                        ..default()
                    },
                ))
                .with_children(|actions| {
                    actions
                        .spawn((small_button("End Turn"), EndsTurn))
                        .with_children(|action| {
                            action.spawn(blurb(assets, "end turn"));
                            action.spawn(fine(assets, "channel mana"));
                        });
                    actions
                        .spawn((small_button("Reset"), ResetsDemo))
                        .with_children(|action| {
                            action.spawn(blurb(assets, "reset"));
                            action.spawn(fine(assets, "fresh state"));
                        });
                });

            for line in &demo.log {
                controls.spawn(fine(assets, format!("·  {line}")));
            }
        });
}

fn handle_input(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<Screen>>) {
    if keys.just_pressed(KeyCode::Backspace) || keys.just_pressed(KeyCode::Escape) {
        next.set(Screen::Title);
    }
}

#[cfg(test)]
mod tests {
    use hex_assets::{ArtPalette, ElementFile, SpellFile, SubstanceFile, SubstanceTable};

    use super::*;

    fn real_content() -> (ElementCatalog, SpellBook, ContentIndex) {
        let element_file: ElementFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/elements.ron"
        )))
        .expect("elements.ron parses");
        let spell_file: SpellFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/spells.ron"
        )))
        .expect("spells.ron parses");
        let substance_file: SubstanceFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/substances.ron"
        )))
        .expect("substances.ron parses");
        let palette: ArtPalette = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/art/palette.ron"
        )))
        .expect("palette.ron parses");

        let elements = ElementCatalog::from_file(&element_file);
        let spells = SpellBook::from_file(&spell_file);
        let substances = SubstanceTable::from_file(&substance_file, &palette)
            .expect("shipped substances resolve through the art palette");
        let index = ContentIndex::build(&elements, &spells, &substances)
            .expect("shipped content cross-references resolve");
        (elements, spells, index)
    }

    /// The whole point of the screen: every spell the demo places is castable
    /// against the real shipped content the moment the lattice is fresh.
    #[test]
    fn every_demo_spell_is_castable_when_fresh() {
        let (elements, spells, index) = real_content();
        let (spec, notes) = build_demo_spec(&elements, &spells, &index);
        assert!(notes.is_empty(), "builder had to skip content: {notes:?}");

        let stats = build_demo_stats(&spec, &spells, &index, &elements);
        let state = LatticeState::new(&spec, &stats);
        let tables = index.tables(&elements);

        let spell_cells: Vec<LatticeCoord> = spec
            .cells()
            .filter_map(|(coord, kind)| matches!(kind, CellKind::Spell { .. }).then_some(coord))
            .collect();
        assert_eq!(
            spell_cells.len(),
            DEMO_SPELLS.len(),
            "every demo pick should place"
        );
        for cell in spell_cells {
            if let Err(blocked) = castable(&spec, &state, cell, &tables) {
                panic!("spell at {cell:?} blocked on a fresh lattice: {blocked:?}");
            }
        }
    }

    /// A cast drains exactly its plan, and one end of turn channels it back.
    #[test]
    fn a_cast_drains_and_end_turn_restores() {
        let (elements, spells, index) = real_content();
        let (spec, _) = build_demo_spec(&elements, &spells, &index);
        let stats = build_demo_stats(&spec, &spells, &index, &elements);
        let mut state = LatticeState::new(&spec, &stats);
        let tables = index.tables(&elements);
        let ember = spells.id("Ember").expect("Ember ships");
        let cell = spec
            .cells()
            .find_map(|(coord, kind)| {
                matches!(kind, CellKind::Spell { spell } if spell == ember).then_some(coord)
            })
            .expect("the builder placed Ember");

        let full = state.total_gem_mana();
        let plan = castable(&spec, &state, cell, &tables).expect("Ember castable fresh");
        let cost: u32 = plan.drains.values().map(|&mana| u32::from(mana)).sum();
        assert!(apply_cast(&mut state, &plan, &tables), "fresh plan applies");
        assert_eq!(full - state.total_gem_mana(), cost);

        channel(&mut state, &spec, &stats);
        assert_eq!(
            state.total_gem_mana(),
            full,
            "one end of turn should refill a tier-1 cast"
        );
    }

    /// Casting the shipped enchantment locks mana, and striking a funding gem
    /// shatters it, burning what was locked.
    #[test]
    fn the_shipped_enchantment_locks_and_burns() {
        let (elements, spells, index) = real_content();
        let (spec, _) = build_demo_spec(&elements, &spells, &index);
        let stats = build_demo_stats(&spec, &spells, &index, &elements);
        let mut state = LatticeState::new(&spec, &stats);
        let tables = index.tables(&elements);
        let shield = spells.id("Metal Shield").expect("Metal Shield ships");
        assert!(
            matches!(tables.casting(shield), Casting::Enchantment { .. }),
            "Metal Shield should map to the enchantment axis"
        );
        let cell = spec
            .cells()
            .find_map(|(coord, kind)| {
                matches!(kind, CellKind::Spell { spell } if spell == shield).then_some(coord)
            })
            .expect("the builder placed Metal Shield");

        let plan = castable(&spec, &state, cell, &tables).expect("shield castable fresh");
        assert!(apply_cast(&mut state, &plan, &tables));
        assert_eq!(state.enchantment_count(), 1);
        let locked = state.total_locked_mana();
        assert!(locked > 0, "an enchantment should lock its funding mana");

        let funding = spec
            .cells()
            .find_map(|(coord, _)| state.is_locked(coord).then_some(coord))
            .expect("some gem funds the enchantment");
        let broken = apply_disables(&mut state, &[funding]);
        assert_eq!(
            broken.len(),
            1,
            "striking the funding gem breaks the shield"
        );
        assert_eq!(state.enchantment_count(), 0);
        assert_eq!(state.total_locked_mana(), 0);
    }

    /// An id the index does not know must stay uncastable — an empty
    /// requirement list would read as a free spell instead.
    #[test]
    fn unknown_spells_are_blocked_not_free() {
        let (elements, spells, index) = real_content();
        let tables = index.tables(&elements);
        let _ = &spells;
        let unknown = hex_core::SpellId(u16::MAX);
        let requirements = tables.requirements(unknown);
        assert_eq!(requirements.len(), 1);
        assert_eq!(
            requirements.first().map(|requirement| requirement.mana),
            Some(u16::MAX),
            "the fallback requirement must be unsatisfiable"
        );
    }
}
