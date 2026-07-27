//! Interactive sandbox for the lattice ruleset, reachable from the title menu.
//!
//! Builds a small demonstration lattice out of the *real* content in
//! `elements.ron` / `spells.ron` and lets a human exercise every rule the pure
//! engine ships: binary casting, fusion resolution, mana capacity versus
//! channelling throughput, damage disables, and enchantment mana locking.
//! Nothing persists — the lattice is rebuilt from content on every entry, and
//! `Reset` rebuilds the battle state. Reset is deliberately the only way back
//! from a strike: the engine has no un-disable, so neither does the demo.
//!
//! # The tables adapter is demo-local by design
//!
//! [`DemoTables`] maps `ContentIndex`/`ElementCatalog` onto the engine's
//! `SpellTable`/`FusionTable` traits. The *permanent* wiring — where those
//! implementations live, and how casts reach real combat — belongs to HEX-12;
//! this screen only proves the mapping is 1:1, so it keeps its copy private
//! rather than claiming the seam.

use std::collections::BTreeMap;

use bevy::prelude::*;
use hex_assets::{CastingAxis, ContentIndex, ElementCatalog, SpellBook};
use hex_core::{ElementId, LatticeCoord, Screen};
use hex_lattice::{
    apply_cast, apply_disables, castable, channel, tick_burns, CastBlocked, Casting, CellKind,
    FusionTable, LatticeSpec, LatticeState, LatticeStats, Requirement, SpellTable,
};

use crate::menus::widgets::{compact_button, OwnColors, LABEL, MUTED};

use super::{despawn_screen, screen_root};

/// Spells the demo tries to place, chosen to cover the casting axes: a cheap
/// evocation, a defensive enchantment, a ritual, and a tier-6 evocation whose
/// cost makes `Unsatisfiable` easy to reach. Missing names are skipped and
/// reported in the demo log rather than failing the screen.
const DEMO_SPELLS: [&str; 4] = ["Ember", "Metal Shield", "Flamethrower", "Fireball"];

/// Where each placed spell sits. Spacing of four keeps one spell's gems (and a
/// fusion's recipe gems, two steps out) from colliding with the next spell's.
const ANCHORS: [(i32, i32); 4] = [(0, 0), (4, 0), (0, 4), (4, 4)];

/// Pixel size of one lattice cell button.
const CELL_SIZE: f32 = 74.0;

/// Horizontal distance between neighbouring cell centres.
const CELL_STEP: f32 = 82.0;

/// Vertical distance between rows, under `CELL_STEP` so the layout packs like hexes.
const ROW_STEP: f32 = 72.0;

/// How many log lines the demo keeps.
const LOG_LINES: usize = 6;

const GEM_COLOR: Color = Color::srgba(0.25, 0.55, 0.65, 0.35);
const FUSION_COLOR: Color = Color::srgba(0.55, 0.40, 0.75, 0.35);
const SPELL_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.10);
const LOCKED_COLOR: Color = Color::srgba(0.85, 0.65, 0.20, 0.40);
const DISABLED_COLOR: Color = Color::srgba(0.60, 0.15, 0.12, 0.50);

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

/// The button that channels mana back and ticks burns.
#[derive(Component)]
struct EndsTurn;

/// The button that rebuilds the battle state from the inscription.
#[derive(Component)]
struct ResetsDemo;

/// Bridges the loaded content tables to the engine's lookup traits.
///
/// Demo-local on purpose — see the module docs. The `None` arms matter: an
/// unknown spell id must stay *uncastable*, and an empty requirement list would
/// instead read as a tier-0 spell that costs nothing, so the fallback is a
/// single requirement no lattice can satisfy.
struct DemoTables<'a> {
    index: &'a ContentIndex,
    elements: &'a ElementCatalog,
}

impl SpellTable for DemoTables<'_> {
    fn requirements(&self, spell: hex_core::SpellId) -> Vec<Requirement> {
        match self.index.requirements(spell) {
            Some(requirements) => requirements
                .iter()
                .map(|&(element, mana)| Requirement { element, mana })
                .collect(),
            None => vec![Requirement {
                element: poison_element(self.elements),
                mana: u16::MAX,
            }],
        }
    }

    fn casting(&self, spell: hex_core::SpellId) -> Casting {
        match self.index.casting(spell) {
            Some(CastingAxis::Enchantment { defense }) => Casting::Enchantment { defense },
            Some(CastingAxis::Evocation) | None => Casting::Evocation,
        }
    }
}

impl FusionTable for DemoTables<'_> {
    fn recipe(&self, output: ElementId) -> Option<Vec<Requirement>> {
        self.elements.recipe(output).map(|inputs| {
            inputs
                .iter()
                .map(|&(element, mana)| Requirement { element, mana })
                .collect()
        })
    }
}

/// An element for the unknown-spell fallback requirement.
///
/// Any real element works — the `u16::MAX` cost is what blocks the cast — and
/// an empty catalog blocks everything anyway, so the default id is fine there.
fn poison_element(elements: &ElementCatalog) -> ElementId {
    elements.wheel().first().copied().unwrap_or_default()
}

fn spawn_demo_screen(mut commands: Commands) {
    commands
        .spawn(screen_root(Screen::LatticeDemo, "Lattice Demo Screen"))
        .with_children(|parent| {
            parent.spawn((
                Text::new("the lattice"),
                TextFont::from_font_size(40.0),
                TextColor(LABEL),
            ));
            parent
                .spawn((
                    Name::new("Demo Body"),
                    DemoBody,
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(56.0),
                        align_items: AlignItems::FlexStart,
                        ..default()
                    },
                ))
                .with_children(|body| {
                    body.spawn((
                        Text::new("waiting for content..."),
                        TextFont::from_font_size(16.0),
                        TextColor(MUTED),
                    ));
                });
            parent.spawn((
                Text::new(
                    "cast from the right panel - click a gem to strike it - BACKSPACE to return",
                ),
                TextFont::from_font_size(14.0),
                TextColor(MUTED),
            ));
        });
}

fn remove_demo_state(mut commands: Commands) {
    commands.remove_resource::<DemoLattice>();
}

/// Builds the demo lattice once the content tables exist.
///
/// The title screen is reachable before the RON files finish parsing, so this
/// runs in `Update` behind `Option` guards rather than assuming the resources
/// at `OnEnter` — the same lesson the scenario list learned.
fn init_demo(
    mut commands: Commands,
    demo: Option<Res<DemoLattice>>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
    index: Option<Res<ContentIndex>>,
) {
    if demo.is_some() {
        return;
    }
    let (Some(elements), Some(spells), Some(index)) = (elements, spells, index) else {
        return;
    };

    let (spec, notes) = build_demo_spec(&elements, &spells, &index);
    let stats = build_demo_stats(&spec, &spells, &index, &elements);
    let state = LatticeState::new(&spec, &stats);
    let mut log = vec!["fresh lattice - every gem at full mana".to_owned()];
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
            break;
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

/// Fabricates the character stats the content pipeline does not yet supply.
///
/// Capacity is the largest single cost the content ever asks of that element
/// (floor 3); channelling is capacity plus two, generous enough that one end
/// of turn refills what one cast drained. When real attunement stats land
/// (HEX-12 and beyond), this function is what they replace.
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
    let tables = DemoTables {
        index: &index,
        elements: &elements,
    };
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
    let tables = DemoTables {
        index: &index,
        elements: &elements,
    };
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
        let due = tick_burns(state);
        if due == 0 {
            push_log(
                log,
                "end of turn: channelled mana back toward capacity".to_owned(),
            );
        } else {
            push_log(
                log,
                format!("end of turn: channelled, and {due} burn(s) came due"),
            );
        }
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

fn try_cast(demo: &mut DemoLattice, cell: LatticeCoord, tables: &DemoTables, spells: &SpellBook) {
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

fn blocked_reason(blocked: &CastBlocked) -> &'static str {
    match blocked {
        CastBlocked::NotASpell => "that cell holds no spell",
        CastBlocked::SpellDisabled => "the spell hex is disabled",
        CastBlocked::Unsatisfiable => "cannot draw its full cost from adjacent sources",
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
) {
    let Some(demo) = demo else { return };
    if !demo.is_changed() {
        return;
    }
    let (Some(index), Some(elements), Some(spells)) = (index, elements, spells) else {
        return;
    };
    let Ok(body) = bodies.single() else { return };
    let tables = DemoTables {
        index: &index,
        elements: &elements,
    };

    commands.entity(body).despawn_related::<Children>();
    commands.entity(body).with_children(|panels| {
        spawn_lattice_panel(panels, &demo, &elements, &spells);
        spawn_control_panel(panels, &demo, &tables, &spells);
    });
}

fn cell_position(coord: LatticeCoord) -> (f32, f32) {
    #[expect(
        clippy::cast_precision_loss,
        reason = "demo coordinates are single digits; f32 is exact far beyond them"
    )]
    let (q, r) = (coord.q() as f32, coord.r() as f32);
    (CELL_STEP * (q + r * 0.5), ROW_STEP * r)
}

fn spawn_lattice_panel(
    panels: &mut ChildSpawnerCommands,
    demo: &DemoLattice,
    elements: &ElementCatalog,
    spells: &SpellBook,
) {
    let mut min = (f32::MAX, f32::MAX);
    let mut max = (f32::MIN, f32::MIN);
    for (coord, _) in demo.spec.cells() {
        let (x, y) = cell_position(coord);
        min = (min.0.min(x), min.1.min(y));
        max = (max.0.max(x), max.1.max(y));
    }
    if demo.spec.capacity() == 0 {
        panels.spawn((
            Text::new("the content defined no demo lattice"),
            TextFont::from_font_size(16.0),
            TextColor(MUTED),
        ));
        return;
    }

    panels
        .spawn((
            Name::new("Demo Lattice"),
            Node {
                width: Val::Px(max.0 - min.0 + CELL_SIZE),
                height: Val::Px(max.1 - min.1 + CELL_SIZE),
                ..default()
            },
        ))
        .with_children(|lattice| {
            for (coord, kind) in demo.spec.cells() {
                let (x, y) = cell_position(coord);
                let (color, title, line) = cell_face(coord, kind, demo, elements, spells);
                lattice
                    .spawn((
                        Name::new("Demo Cell"),
                        Button,
                        DemoCell(coord),
                        OwnColors,
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(x - min.0),
                            top: Val::Px(y - min.1),
                            width: Val::Px(CELL_SIZE),
                            height: Val::Px(CELL_SIZE),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            flex_direction: FlexDirection::Column,
                            border_radius: BorderRadius::all(Val::Px(CELL_SIZE / 2.0)),
                            ..default()
                        },
                        BackgroundColor(color),
                    ))
                    .with_children(|cell| {
                        cell.spawn((
                            Text::new(title),
                            TextFont::from_font_size(13.0),
                            TextColor(LABEL),
                            Pickable::IGNORE,
                        ));
                        cell.spawn((
                            Text::new(line),
                            TextFont::from_font_size(11.0),
                            TextColor(MUTED),
                            Pickable::IGNORE,
                        ));
                    });
            }
        });
}

/// What one cell shows: its colour, headline, and detail line.
fn cell_face(
    coord: LatticeCoord,
    kind: CellKind,
    demo: &DemoLattice,
    elements: &ElementCatalog,
    spells: &SpellBook,
) -> (Color, String, String) {
    let disabled = demo.state.is_disabled(coord);
    let locked = demo.state.is_locked(coord);
    let color = if disabled {
        DISABLED_COLOR
    } else if locked {
        LOCKED_COLOR
    } else {
        match kind {
            CellKind::Gem { .. } => GEM_COLOR,
            CellKind::Fusion { .. } => FUSION_COLOR,
            _ => SPELL_COLOR,
        }
    };
    let (title, mut line) = match kind {
        CellKind::Gem { element } => (
            elements.name(element).unwrap_or("gem").to_owned(),
            format!(
                "{}/{}",
                demo.state.mana(coord),
                demo.stats.capacity(element)
            ),
        ),
        CellKind::Fusion { output } => (
            "fusion".to_owned(),
            elements.name(output).unwrap_or("?").to_owned(),
        ),
        CellKind::Spell { spell } => (
            spells.name(spell).unwrap_or("spell").to_owned(),
            spells
                .spell(spell)
                .map(|entry| format!("tier {}", entry.tier()))
                .unwrap_or_default(),
        ),
        CellKind::Blank => ("-".to_owned(), String::new()),
    };
    if disabled {
        line = format!("{line} - disabled");
    } else if locked {
        line = format!("{line} - locked");
    }
    (color, title, line)
}

fn spawn_control_panel(
    panels: &mut ChildSpawnerCommands,
    demo: &DemoLattice,
    tables: &DemoTables,
    spells: &SpellBook,
) {
    panels
        .spawn((
            Name::new("Demo Controls"),
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                min_width: Val::Px(420.0),
                ..default()
            },
        ))
        .with_children(|controls| {
            controls.spawn((
                Text::new("spells"),
                TextFont::from_font_size(18.0),
                TextColor(LABEL),
            ));

            for (coord, kind) in demo.spec.cells() {
                let CellKind::Spell { spell } = kind else {
                    continue;
                };
                let name = spells.name(spell).unwrap_or("unknown spell");
                let kind_line = match tables.casting(spell) {
                    Casting::Enchantment { defense } => format!("enchantment, defense {defense}"),
                    Casting::Evocation => "evocation".to_owned(),
                };
                let ritual = spells
                    .spell(spell)
                    .is_some_and(hex_assets::Spell::is_ritual);
                let headline = if ritual {
                    format!("{name} (ritual) - {kind_line}")
                } else {
                    format!("{name} - {kind_line}")
                };

                controls
                    .spawn((
                        Name::new("Spell Row"),
                        Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(12.0),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ))
                    .with_children(|row| {
                        match castable(&demo.spec, &demo.state, coord, tables) {
                            Ok(plan) => {
                                let cost: u32 =
                                    plan.drains.values().map(|&mana| u32::from(mana)).sum();
                                row.spawn((compact_button("Cast"), CastsSpell(coord)))
                                    .with_children(|cast| {
                                        cast.spawn((
                                            Text::new("cast"),
                                            TextFont::from_font_size(14.0),
                                            TextColor(LABEL),
                                            Pickable::IGNORE,
                                        ));
                                        cast.spawn((
                                            Text::new(format!("{cost} mana")),
                                            TextFont::from_font_size(11.0),
                                            TextColor(MUTED),
                                            Pickable::IGNORE,
                                        ));
                                    });
                            }
                            Err(blocked) => {
                                row.spawn((
                                    Text::new(format!("blocked: {}", blocked_reason(&blocked))),
                                    TextFont::from_font_size(12.0),
                                    TextColor(MUTED),
                                    Node {
                                        max_width: Val::Px(150.0),
                                        ..default()
                                    },
                                ));
                            }
                        }
                        row.spawn((
                            Text::new(headline),
                            TextFont::from_font_size(14.0),
                            TextColor(LABEL),
                        ));
                    });
            }

            controls.spawn((
                Text::new(format!(
                    "free mana {}  -  locked {}  -  enchantments {}  -  burns {}",
                    demo.state.total_gem_mana(),
                    demo.state.total_locked_mana(),
                    demo.state.enchantment_count(),
                    demo.state.burns().len(),
                )),
                TextFont::from_font_size(14.0),
                TextColor(LABEL),
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
                        .spawn((compact_button("End Turn"), EndsTurn))
                        .with_children(|action| {
                            action.spawn((
                                Text::new("end turn"),
                                TextFont::from_font_size(14.0),
                                TextColor(LABEL),
                                Pickable::IGNORE,
                            ));
                            action.spawn((
                                Text::new("channel + burns"),
                                TextFont::from_font_size(11.0),
                                TextColor(MUTED),
                                Pickable::IGNORE,
                            ));
                        });
                    actions
                        .spawn((compact_button("Reset"), ResetsDemo))
                        .with_children(|action| {
                            action.spawn((
                                Text::new("reset"),
                                TextFont::from_font_size(14.0),
                                TextColor(LABEL),
                                Pickable::IGNORE,
                            ));
                            action.spawn((
                                Text::new("fresh state"),
                                TextFont::from_font_size(11.0),
                                TextColor(MUTED),
                                Pickable::IGNORE,
                            ));
                        });
                });

            for line in &demo.log {
                controls.spawn((
                    Text::new(format!("> {line}")),
                    TextFont::from_font_size(12.0),
                    TextColor(MUTED),
                ));
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
    use hex_assets::{ElementFile, SpellFile, SubstanceFile, SubstanceTable};

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

        let elements = ElementCatalog::from_file(&element_file);
        let spells = SpellBook::from_file(&spell_file);
        let substances = SubstanceTable::from_file(&substance_file);
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
        let tables = DemoTables {
            index: &index,
            elements: &elements,
        };

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
        let tables = DemoTables {
            index: &index,
            elements: &elements,
        };
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
        let tables = DemoTables {
            index: &index,
            elements: &elements,
        };
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
        let tables = DemoTables {
            index: &index,
            elements: &elements,
        };
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
