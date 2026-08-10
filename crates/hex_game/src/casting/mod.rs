//! Choosing a spell, aiming it, and seeing what it will touch.
//!
//! The interface to what `docs/systems/casting.md` specifies, and nothing more. This
//! module decides what a click *means* and pushes a [`GameCommand::Cast`] into the
//! funnel; **the applier in `hex_combat` is authoritative**. Everything here is a
//! pre-filter for click feel, and only one direction of disagreement is a bug: offering
//! a cast the applier then refuses. Every refusal shown below therefore names the
//! applier check it mirrors, so the two can be read side by side when either moves.
//!
//! # Three surfaces, one derivation
//!
//! The panel ("what can I cast"), the world preview ("what would it touch") and the
//! input handler ("what does this click do") could each compute their own answer — and
//! could then disagree. A panel offering a spell whose preview lights nothing is worse
//! than either surface alone. [`CastReadout`] is derived once per frame and all three
//! read it.
//!
//! # Aiming does not take the map away
//!
//! While a spell is aimed, every legal anchor carries a clickable marker, and a marker
//! is a picking blocker — so a click on a lit surface aims, and never also moves. Off
//! the lit set there is no marker, the click reaches the tile underneath, and it means
//! what a click has always meant: walk there.
//!
//! Repositioning to get into range is part of aiming rather than an escape from it: the
//! aim survives the walk and its anchors are recomputed from wherever the caster ends up.
//!
//! That takes distinguishing two things the panel says in the same voice. Being `Busy`
//! mid-walk, or waiting on somebody else's decision, **suspends** casting — you will be
//! able to in a moment, and the spell you picked is still the spell you want. Your turn
//! ending, or your action being spent, **ends** it. `keeps_the_aim` draws that line, and
//! without it the `Busy` a walk sets would put the aim down one frame after the click
//! that started the walk, which is the opposite of the sentence above.
//!
//! The aim is still dropped when it stops being castable, and after a walk the check that
//! earns its keep is range: stepping away can carry an anchor out of reach exactly as
//! stepping toward it brings one in.

use std::collections::BTreeSet;

use bevy::{
    input_focus::{tab_navigation::TabIndex, InputFocus},
    prelude::*,
};
use hex_assets::{
    CastingAxis, CombatSettings, ContentIndex, ElementCatalog, ManaAxis, Spell, SpellBook,
    SubstanceTable, TargetShape, TargetingReach, Trajectory,
};
use hex_combat::{FactionLatticeKnowledge, TurnOrder};
use hex_core::{
    AppSystems, Busy, CommandQueue, ControlOwner, GameCommand, GameplaySystems, HexCoord,
    InputAction, InputBindings, IssuedCommand, KnowledgeState, LatticeCoord, Mode, PausableSystems,
    PendingDecision, PlayerSeat, Screen, Sextant, SpellId, TilePos, Turn, UnitId,
};
use hex_gameplay_model::{HudComponent, HudState};
use hex_lattice::{castable, CastBlocked, CellKind, LatticeSpec, LatticeState};
use hex_perception::{FactionKnowledge, FactionMapKnowledge, SurfaceSnapshot};
use hex_units::{
    in_touch_reach, known_trajectory_is_clear, targeting, trajectory_destination, volumes, Body,
    Downed, Faction, Footing, KnownTerrainOccupancy, Player, Selected, StandsOn, UnitRegistry,
};

use hex_ui::element_color;

pub(crate) mod panel;
mod preview;

/// Height-per-range-bonus when `combat.ron` has not loaded.
///
/// The same fallback and the same number as `hex_combat`'s cast applier, deliberately:
/// the interface decides which surfaces to light and the applier decides which it
/// accepts, and the two disagreeing about the high-ground bonus would light exactly the
/// surfaces a cast is then refused for.
const DEFAULT_LEVELS_PER_BONUS: u32 = 5;

/// Registers the spell panel, the shape preview, and the cast emitter.
pub fn plugin(app: &mut App) {
    app.init_resource::<InputBindings>();
    app.init_resource::<CastReadout>();
    app.init_resource::<Aiming>();
    app.init_resource::<AimExit>();
    app.init_resource::<preview::AimVolume>();
    app.init_resource::<preview::DrawnPreviewKey>();
    // Global, like every picking observer in this codebase. It is written for that —
    // see its own docs — and registering it here rather than per marker keeps one
    // observer instead of one for each of the hundred surfaces an aim can light.
    app.add_observer(preview::on_anchor_clicked);

    app.add_systems(
        OnExit(Screen::Gameplay),
        (forget_aim, preview::clear_preview),
    );
    app.add_systems(
        Update,
        (
            refresh_readout,
            resolve_aim_input.after(hex_ui::UiSystems::EmitIntents),
        )
            .chain()
            .in_set(AppSystems::RecordInput)
            .in_set(PausableSystems)
            .run_if(in_state(Screen::Gameplay)),
    );
    // The preview runs before the panel because the panel reports what the preview
    // resolved — how many voxels the shape covers, and how many of them are surfaces
    // there is anything to paint.
    app.add_systems(
        Update,
        (preview::redraw_preview, panel::publish_view)
            .chain()
            .after(resolve_aim_input)
            .after(GameplaySystems::UiContext)
            .after(hex_units::TerrainOccupancySystems::Publish)
            .in_set(PausableSystems)
            .run_if(in_state(Screen::Gameplay)),
    );
    app.add_systems(
        Update,
        refresh_readout
            .in_set(GameplaySystems::Casting)
            .in_set(PausableSystems)
            .run_if(in_state(Screen::Gameplay)),
    );
}

/// What the casting interface knows this frame.
///
/// Derived once by [`refresh_readout`] and read by everything else, so the panel, the
/// preview and the input handler cannot answer the same question differently.
#[derive(Resource, Default, Debug, PartialEq)]
pub struct CastReadout {
    /// The unit the interface is talking about, when there is one.
    pub caster: Option<Caster>,
    /// Why no cast is possible at all, or `None` when one is.
    pub unavailable: Option<&'static str>,
    /// One row per distinct spell the caster inscribes, sorted by name.
    pub spells: Vec<SpellRow>,
    /// `combat.ron`'s levels-per-bonus-hex knob, carried so the panel, the preview and
    /// the emitter all measure range exactly the way the applier does.
    pub levels_per_bonus: u32,
}

impl CastReadout {
    /// The row for a spell, by name.
    #[must_use]
    pub fn row(&self, name: &str) -> Option<&SpellRow> {
        self.spells.iter().find(|row| row.name == name)
    }
}

/// The unit a cast would come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caster {
    /// The sim id, which is what a command names.
    pub unit: UnitId,
    /// The seat that owns it, stamped onto the command at emission.
    pub seat: PlayerSeat,
    /// Which surface it stands on — the origin of every range and shape question.
    pub standing: TilePos,
    /// Body-specific traversal policy used by exact touch reach.
    pub body: Body,
}

/// One spell, as the panel shows it and the preview reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct SpellRow {
    /// The spell's name. Also the command payload, and the button's handle.
    pub name: String,
    /// The identity line: tier, casting axis, element.
    pub detail: String,
    /// The price line: mana, range, shape.
    pub cost: String,
    /// Why the lattice refuses this spell, or `None` when it can pay.
    pub blocked: Option<&'static str>,
    /// The tint this spell is presented in, taken from its element.
    pub color: Color,
    /// Base range in hexes, before any high-ground bonus.
    pub range: u32,
    /// Whether the anchor uses ordinary ranged geometry or exact mutual-step touch.
    pub reach: TargetingReach,
    /// The shape whose volume the preview resolves.
    pub shape: TargetShape,
    /// How exact material occupancy between caster and anchor affects this spell.
    pub trajectory: Trajectory,
    /// Whether the shape begins in the air above the selected surface.
    pub creates_terrain: bool,
    /// Whether this spell may validly target a retained Downed lattice.
    pub restores: bool,
}

/// The spell currently being aimed, if any.
#[derive(Resource, Default, Debug)]
pub struct Aiming(pub Option<Aim>);

/// How the most recent aim ended.
///
/// A confirmed aim leaves its target available for inspection, while an explicit
/// cancellation clears it. This pulse is consumed by the lattice readout later in the
/// same frame.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AimExit {
    /// No aim ended this frame.
    #[default]
    None,
    /// The cast was emitted and its target should remain pinned.
    Confirmed,
    /// The player explicitly put the aim down.
    Cancelled,
}

/// A spell chosen, and the anchor it is pointed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aim {
    /// Which spell, **by name** — ids are session-local, and the command carries a name.
    pub spell: String,
    /// The one positional anchor the cast will name.
    pub anchor: TilePos,
}

/// The words the interface uses for a blocked cast.
///
/// Shared with the lattice demo so the two screens say the same thing about the same
/// refusal; they used to carry a copy each, which is one edit away from disagreeing.
/// These are the *lattice's* reasons — the applier's wording for the same three cases
/// is deliberately more specific, because it is talking about a command that has
/// already been issued rather than a button that has not been pressed.
#[must_use]
pub fn blocked_reason(blocked: &CastBlocked) -> &'static str {
    match blocked {
        CastBlocked::NotASpell => "no spell here",
        CastBlocked::SpellDisabled => "spell hex disabled",
        CastBlocked::Unsatisfiable => "not enough adjacent mana",
    }
}

/// Whether a caster standing at `from` can anchor a range-`range` spell on `to`.
///
/// The one range question the interface asks, routed through the same
/// [`targeting::in_reach`] the applier uses, so spells inherit high-ground-buys-range
/// here exactly as they do there.
#[must_use]
pub fn in_range(from: TilePos, to: TilePos, range: u32, levels_per_bonus: u32) -> bool {
    targeting::in_reach(from, to, range, levels_per_bonus)
}

/// Rebuilds the readout, and drops an aim the caster can no longer make good on.
///
/// Every resource is an `Option`: this runs inside gameplay, but content parses
/// asynchronously and a scenario can be entered before `spells.ron` has landed. The
/// panel then says nothing rather than the system deciding its first frame by ordering.
fn refresh_readout(
    mut readout: ResMut<CastReadout>,
    mut aiming: ResMut<Aiming>,
    mode: Option<Res<State<Mode>>>,
    order: Option<Res<TurnOrder>>,
    registry: Option<Res<UnitRegistry>>,
    pending: Option<Res<PendingDecision>>,
    spells: Option<Res<SpellBook>>,
    index: Option<Res<ContentIndex>>,
    elements: Option<Res<ElementCatalog>>,
    combat: Option<Res<CombatSettings>>,
    substances: Option<Res<SubstanceTable>>,
    knowledge: Option<Res<FactionMapKnowledge>>,
    lattice_knowledge: Option<Res<FactionLatticeKnowledge>>,
    casters: Query<CasterData, (With<Player>, Without<Downed>)>,
    restoration_targets: RestorationTargetQuery,
    selected: Query<Entity, (With<Player>, With<Selected>, Without<Downed>)>,
) {
    let next = build_readout(
        Sources {
            mode: mode.as_deref(),
            order: order.as_deref(),
            registry: registry.as_deref(),
            pending: pending.as_deref(),
            spells: spells.as_deref(),
            index: index.as_deref(),
            elements: elements.as_deref(),
            combat: combat.as_deref(),
        },
        &casters,
        &selected,
    );
    // Assign only on a real difference. The panel rebuild is driven by change
    // detection, and a `ResMut` written every frame would rebuild the whole panel every
    // frame — sixty despawn-and-respawn cycles a second behind an interface that never
    // moved.
    if *readout != next {
        *readout = next;
    }

    // An aim outlives the frame it was made in, so it is re-checked against the readout
    // rather than trusted: the turn can end, a funding gem can be struck, or the unit
    // can go down between choosing a spell and casting it.
    //
    // **Re-anchored, not dropped, when the caster moves.** Walking sets `Busy`, which is
    // a transient reason — the spell is still the spell — so the aim is kept and the
    // anchor is re-measured from wherever the caster now stands. It is put down only when
    // it has actually stopped being castable: the range check below is the one that
    // matters after a walk, because stepping *away* can carry the anchor out of reach
    // just as stepping toward it brings one in.
    let valid = aiming.0.as_ref().is_some_and(|aim| {
        let survives = readout.unavailable.is_none_or(keeps_the_aim);
        let Some(row) = readout.row(&aim.spell) else {
            return false;
        };
        let observed = knowledge.as_deref().is_some_and(|knowledge| {
            knowledge.faction(Faction::Player).state(aim.anchor) == KnowledgeState::Observed
        });
        let reachable = readout.caster.is_some_and(|caster| {
            aim_target_reachable(
                row,
                &caster,
                aim.anchor,
                knowledge
                    .as_deref()
                    .map(|knowledge| knowledge.faction(Faction::Player)),
                substances.as_deref(),
                readout.levels_per_bonus,
            )
        });
        let restoration_eligible = !row.restores
            || knowledge.as_deref().is_some_and(|spatial| {
                let allowed = lattice_knowledge
                    .as_deref()
                    .map_or_else(BTreeSet::new, |known| {
                        restorable_target_ids(
                            spatial.faction(Faction::Player),
                            Faction::Player,
                            known,
                            &restoration_targets,
                        )
                    });
                spatial
                    .faction(Faction::Player)
                    .units()
                    .any(|(id, unit)| unit.pos == aim.anchor && allowed.contains(&id))
            });
        survives && row.blocked.is_none() && reachable && observed && restoration_eligible
    });
    if aiming.0.is_some() && !valid {
        aiming.0 = None;
    }
}

/// The caster's columns, as the readout reads them.
type CasterData = (
    &'static UnitId,
    Option<&'static ControlOwner>,
    &'static StandsOn,
    &'static Body,
    &'static LatticeSpec,
    &'static LatticeState,
    Option<&'static Turn>,
    Has<Busy>,
);

type RestorationTargetQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static UnitId,
        &'static Faction,
        Option<&'static LatticeSpec>,
        Option<&'static LatticeState>,
    ),
>;

/// Everything outside the ECS queries that the readout is built from.
///
/// A struct rather than eight parameters so the derivation can be called from a test
/// without re-listing them, and so an added source cannot be silently dropped at one
/// call site.
struct Sources<'a> {
    mode: Option<&'a State<Mode>>,
    order: Option<&'a TurnOrder>,
    registry: Option<&'a UnitRegistry>,
    pending: Option<&'a PendingDecision>,
    spells: Option<&'a SpellBook>,
    index: Option<&'a ContentIndex>,
    elements: Option<&'a ElementCatalog>,
    combat: Option<&'a CombatSettings>,
}

/// Assembles the readout from whatever content and world state exists.
fn build_readout(
    sources: Sources,
    casters: &Query<CasterData, (With<Player>, Without<Downed>)>,
    selected: &Query<Entity, (With<Player>, With<Selected>, Without<Downed>)>,
) -> CastReadout {
    let levels_per_bonus = sources.combat.map_or(DEFAULT_LEVELS_PER_BONUS, |settings| {
        settings.levels_per_bonus_range
    });
    let empty = CastReadout {
        levels_per_bonus,
        ..CastReadout::default()
    };
    let (Some(spells), Some(index), Some(elements)) =
        (sources.spells, sources.index, sources.elements)
    else {
        return empty;
    };

    // The acting unit when it is one of ours, and the selection otherwise. Those are
    // the same entity in every shipped encounter — there is one player piece — but they
    // are different questions, and the panel is about whoever is going to cast.
    let acting = sources
        .order
        .and_then(TurnOrder::current)
        .and_then(|unit| sources.registry?.entity_of(unit))
        .filter(|entity| casters.contains(*entity));
    let Some(entity) = acting.or_else(|| selected.iter().next()) else {
        return empty;
    };
    let Ok((unit, owner, standing, body, spec, state, turn, busy)) = casters.get(entity) else {
        return empty;
    };

    let tables = index.tables(elements);
    let mut rows: Vec<SpellRow> = inscribed_spells(spec)
        .into_iter()
        .filter_map(|spell| {
            let name = spells.name(spell)?;
            let definition = spells.spell(spell)?;
            let cell = casting_cell(spec, state, spell)?;
            // Deliverability first, and it outranks payment. A spell whose every effect
            // is still waiting on another lane is a legal cast, so the applier charges
            // for it: the mana goes, the turn goes, and the only trace is a log line a
            // release build does not show. Saying "not built yet" here is the difference
            // between an interface that is honest about what is finished and one that
            // eats your turn. `hex_combat` owns the answer so both sides give the same
            // one — see `hex_combat::delivers_anything`.
            let blocked = if hex_combat::delivers_anything(definition) {
                castable(spec, state, cell, &tables)
                    .err()
                    .map(|reason| blocked_reason(&reason))
            } else {
                Some(hex_combat::UNDELIVERABLE)
            };
            Some(spell_row(name, definition, blocked, elements))
        })
        .collect();
    rows.sort_by(|left, right| left.name.cmp(&right.name));

    CastReadout {
        caster: Some(Caster {
            unit: *unit,
            seat: owner.copied().unwrap_or_default().0,
            standing: standing.0.pos,
            body: *body,
        }),
        unavailable: unavailable_reason(
            sources.mode.is_some_and(|mode| *mode.get() == Mode::Combat),
            sources.order.and_then(TurnOrder::current),
            sources.pending.is_some_and(PendingDecision::is_open),
            *unit,
            turn,
            busy,
        ),
        spells: rows,
        levels_per_bonus,
    }
}

/// Why nothing can be cast right now, mirroring `hex_combat::commands::cast`.
///
/// Every arm here has a counterpart in the applier, and the applier is the one that
/// counts. Showing a castable spell it would then refuse is the failure this exists to
/// prevent, so the two lists have to be read together whenever either moves. The order
/// differs from the applier's deliberately: it checks combat, turn, then decision,
/// while a human reading a panel wants the most permanent reason first.
///
/// Plain values rather than the ECS types they came from, so the whole ladder can be
/// walked in a test without an `App` — the rung that goes missing here is a rung the
/// player is invited to fall off.
fn unavailable_reason(
    in_combat: bool,
    acting: Option<UnitId>,
    decision_open: bool,
    unit: UnitId,
    turn: Option<&Turn>,
    busy: bool,
) -> Option<&'static str> {
    if !in_combat {
        return Some(OUT_OF_COMBAT);
    }
    if acting != Some(unit) {
        return Some(NOT_YOUR_TURN);
    }
    // Rung 1 is "action available", and a cast is an action. Without this the panel
    // keeps offering casts for a turn that has already spent its one.
    match turn {
        None => return Some(NO_TURN),
        Some(turn) if turn.acted => return Some(ACTION_SPENT),
        Some(_) => {}
    }
    if busy {
        return Some(BUSY);
    }
    if decision_open {
        return Some(DECISION_OPEN);
    }
    None
}

/// Whether an aim survives this reason, or is put down by it.
///
/// **Two kinds of "you cannot cast right now" wear the same sentence**, and collapsing
/// them is what made walking-while-aiming impossible. A unit that is mid-walk or waiting
/// on somebody else's decision will be able to cast again in a moment, and the spell it
/// had chosen is still the spell it wants; a unit whose turn is over, or that is no
/// longer the one acting, has nothing left to aim with.
///
/// Matched on the named constants rather than the literals so a reworded message cannot
/// quietly flip an aim from surviving to not.
fn keeps_the_aim(reason: &'static str) -> bool {
    matches!(reason, BUSY | DECISION_OPEN)
}

/// Casting is combat-only in wave 3; see `docs/systems/casting.md`.
const OUT_OF_COMBAT: &str = "casting is combat-only for now";

/// Somebody else is acting.
const NOT_YOUR_TURN: &str = "not this unit's turn";

/// The unit holds no turn at all.
const NO_TURN: &str = "no turn to take the action from";

/// The turn's one action is gone.
const ACTION_SPENT: &str = "action already spent this turn";

/// Mid-walk or mid-animation. **Transient** — see [`keeps_the_aim`].
const BUSY: &str = "still finishing the last action";

/// Resolution is parked on a defender's choice. **Transient** — see [`keeps_the_aim`].
const DECISION_OPEN: &str = "a decision is still open";

/// The distinct spells a lattice inscribes, in lattice order.
fn inscribed_spells(spec: &LatticeSpec) -> Vec<SpellId> {
    let mut spells = Vec::new();
    for (_, kind) in spec.cells() {
        if let CellKind::Spell { spell } = kind {
            if !spells.contains(&spell) {
                spells.push(spell);
            }
        }
    }
    spells
}

/// The cell the applier would cast `spell` from.
///
/// **A deliberate copy of `hex_combat::commands::cast::spell_cell`**, which is private
/// to that crate. A lattice may inscribe one spell twice so that losing a hex does not
/// lose the spell, and the applier resolves the ambiguity by taking the first live cell
/// and only then the lowest disabled one. Asking a different cell here would show a
/// blocked spell the applier happily casts, or the reverse — so if that rule ever
/// changes, this is the other half of it.
fn casting_cell(spec: &LatticeSpec, state: &LatticeState, spell: SpellId) -> Option<LatticeCoord> {
    let matching = spec.cells().filter_map(|(coord, kind)| match kind {
        CellKind::Spell { spell: found } if found == spell => Some(coord),
        _ => None,
    });
    let mut fallback = None;
    for coord in matching {
        if !state.is_disabled(coord) {
            return Some(coord);
        }
        fallback = fallback.or(Some(coord));
    }
    fallback
}

/// One row's presentation, built from content alone.
fn spell_row(
    name: &str,
    definition: &Spell,
    blocked: Option<&'static str>,
    elements: &ElementCatalog,
) -> SpellRow {
    let element = definition
        .requirements
        .first()
        .map(|requirement| requirement.element.as_str());
    let axis = match definition.casting {
        CastingAxis::Evocation => "evocation".to_owned(),
        CastingAxis::Enchantment { defense } => format!("enchantment {defense}"),
    };
    let ritual = if definition.is_ritual() {
        " · ritual"
    } else {
        ""
    };
    let cost: u32 = definition
        .requirements
        .iter()
        .map(|requirement| u32::from(requirement.mana))
        .sum();
    // A variable-mana spell has no chooser yet, so the command always carries `None`
    // and the applier spends the plan the lattice already agreed. Writing the cost as a
    // floor is how that stays visible instead of reading as a wrong number.
    let mana = match definition.mana {
        ManaAxis::Fixed => format!("{cost} mana"),
        ManaAxis::Variable => format!("{cost}+ mana"),
    };

    SpellRow {
        name: name.to_owned(),
        detail: format!(
            "tier {} · {axis}{ritual} · {}",
            definition.tier(),
            element.unwrap_or("no element")
        ),
        cost: format!(
            "{mana} · {} · {}",
            reach_label(definition.targeting.reach, definition.targeting.range),
            shape_label(&definition.targeting.shape)
        ),
        blocked,
        color: element_color(element.and_then(|name| elements.id(name)), elements),
        range: u32::from(definition.targeting.range),
        reach: definition.targeting.reach,
        shape: definition.targeting.shape.clone(),
        trajectory: definition.targeting.trajectory,
        creates_terrain: definition.effects.iter().any(|effect| {
            matches!(
                effect,
                hex_assets::Effect::SetTerrain { .. } | hex_assets::Effect::SpawnWall { .. }
            )
        }),
        restores: definition
            .effects
            .iter()
            .any(|effect| matches!(effect, hex_assets::Effect::RestoreHexes { .. })),
    }
}

fn reach_label(reach: TargetingReach, range: u8) -> String {
    match reach {
        TargetingReach::Ranged => format!("range {range}"),
        TargetingReach::Touch => "touch".to_owned(),
    }
}

fn known_footing(knowledge: &FactionKnowledge, substances: &SubstanceTable, body: Body) -> Footing {
    let snapshots: Vec<SurfaceSnapshot> = knowledge
        .surfaces()
        .map(|(_, known)| known.snapshot())
        .filter(|snapshot| !snapshot.blocked)
        .collect();
    Footing::from_tiles(
        snapshots.iter().map(|snapshot| {
            (
                &snapshot.pos,
                &snapshot.span,
                &snapshot.substance,
                &snapshot.headroom,
            )
        }),
        substances,
        body,
        None,
    )
}

fn aim_target_reachable(
    row: &SpellRow,
    caster: &Caster,
    target: TilePos,
    knowledge: Option<&FactionKnowledge>,
    substances: Option<&SubstanceTable>,
    levels_per_bonus: u32,
) -> bool {
    if matches!(row.shape, TargetShape::SelfCast) {
        return target == caster.standing;
    }
    match row.reach {
        TargetingReach::Ranged => in_range(caster.standing, target, row.range, levels_per_bonus),
        TargetingReach::Touch => {
            let Some(knowledge) = knowledge else {
                return false;
            };
            let occupied = knowledge
                .units()
                .any(|(_, observed)| observed.pos == target);
            if !occupied {
                return false;
            }
            if target == caster.standing {
                return knowledge.unit(caster.unit).is_some();
            }
            let Some(substances) = substances else {
                return false;
            };
            in_touch_reach(
                &known_footing(knowledge, substances, caster.body),
                caster.standing,
                target,
            )
        }
    }
}

fn restorable_target_ids(
    spatial: &FactionKnowledge,
    caster_faction: Faction,
    knowledge: &FactionLatticeKnowledge,
    targets: &RestorationTargetQuery,
) -> BTreeSet<UnitId> {
    spatial
        .units()
        .filter_map(|(observed_id, _)| {
            let (id, faction, spec, state) = targets
                .iter()
                .find(|(candidate, ..)| **candidate == observed_id)?;
            let hostile = caster_faction.is_hostile_to(*faction);
            let hostile_knowledge = hostile
                .then(|| knowledge.view(caster_faction, *id))
                .flatten();
            if hostile && !hostile_knowledge.is_some_and(|known| known.is_complete()) {
                return None;
            }
            let (Some(spec), Some(state)) = (spec, state) else {
                return None;
            };
            if hostile && !hostile_knowledge.is_some_and(|known| known.is_complete_for(spec)) {
                return None;
            }
            spec.cells()
                .any(|(coord, _)| state.is_disabled(coord))
                .then_some(*id)
        })
        .collect()
}

/// A shape written the way a player would say it.
fn shape_label(shape: &TargetShape) -> String {
    match shape {
        TargetShape::SelfCast => "on yourself".to_owned(),
        TargetShape::Single => "one voxel".to_owned(),
        TargetShape::Sphere { radius } => format!("sphere r{radius}"),
        TargetShape::Column { height } => format!("column {height} tall"),
        TargetShape::Line { length, width } => format!("line {length} long, {width} wide"),
        TargetShape::Cone { length, spread } => format!("cone {length} long, spread {spread}"),
        TargetShape::Path { offsets } => format!("path of {}", offsets.len()),
    }
}

/// Applies whatever the player asked of the aim this frame.
///
/// One system for the buttons and the keys together, because both mean the same four
/// things and splitting them would give the confirm path two places to push a command
/// from — which is one ordering change away from casting twice.
fn resolve_aim_input(
    readout: Res<CastReadout>,
    mut aiming: ResMut<Aiming>,
    mut exit: ResMut<AimExit>,
    mut queue: ResMut<CommandQueue>,
    pending: Res<PendingDecision>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    focus: Option<Res<InputFocus>>,
    focusable_controls: Query<(), With<TabIndex>>,
    mut intents: MessageReader<hex_ui::UiIntent>,
    knowledge: Option<Res<FactionMapKnowledge>>,
    lattice_knowledge: Option<Res<FactionLatticeKnowledge>>,
    substances: Option<Res<SubstanceTable>>,
    unit_states: Query<(&UnitId, Has<Downed>)>,
    restoration_targets: RestorationTargetQuery,
    mut hud: Option<ResMut<HudState>>,
) {
    if pending.is_open() {
        return;
    }
    let focus_owns_shortcuts = focus
        .as_deref()
        .and_then(InputFocus::get)
        .is_some_and(|entity| focusable_controls.contains(entity));
    let Some(request) = requested(&keys, &bindings, focus_owns_shortcuts, &mut intents) else {
        return;
    };
    let Some(caster) = readout.caster else {
        return;
    };

    let next = match request {
        AimRequest::Cancel => {
            *exit = AimExit::Cancelled;
            None
        }
        AimRequest::Choose(spell) => {
            *exit = AimExit::None;
            let Some(row) = readout.row(&spell) else {
                return;
            };
            if readout.unavailable.is_some() || row.blocked.is_some() {
                return;
            }
            let Some(knowledge) = knowledge.as_deref() else {
                return;
            };
            let player = knowledge.faction(Faction::Player);
            let known_terrain = known_terrain(player);
            let restorable = row.restores.then(|| {
                lattice_knowledge
                    .as_deref()
                    .map_or_else(BTreeSet::new, |known| {
                        restorable_target_ids(player, Faction::Player, known, &restoration_targets)
                    })
            });
            let targets = targets_in_range(
                &readout,
                &caster,
                row,
                player,
                &known_terrain,
                substances.as_deref(),
                &unit_states,
                restorable.as_ref(),
            );
            let fallback = (player.state(caster.standing) == KnowledgeState::Observed
                && restorable
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(&caster.unit))
                && aim_target_reachable(
                    row,
                    &caster,
                    caster.standing,
                    Some(player),
                    substances.as_deref(),
                    readout.levels_per_bonus,
                )
                && trajectory_available(
                    row.trajectory,
                    caster.standing,
                    caster.standing,
                    row.creates_terrain,
                    &known_terrain,
                ))
            .then_some(caster.standing);
            let Some(anchor) = targets.first().copied().or(fallback) else {
                return;
            };
            if let Some(hud) = hud.as_deref_mut() {
                let _ = hud.dismiss_transient_component(HudComponent::ActionBar);
            }
            Some(Aim { spell, anchor })
        }
        AimRequest::Next => {
            *exit = AimExit::None;
            let Some(aim) = aiming.0.clone() else { return };
            let Some(row) = readout.row(&aim.spell) else {
                return;
            };
            let Some(knowledge) = knowledge.as_deref() else {
                return;
            };
            let player = knowledge.faction(Faction::Player);
            let known_terrain = known_terrain(player);
            let restorable = row.restores.then(|| {
                lattice_knowledge
                    .as_deref()
                    .map_or_else(BTreeSet::new, |known| {
                        restorable_target_ids(player, Faction::Player, known, &restoration_targets)
                    })
            });
            let targets = targets_in_range(
                &readout,
                &caster,
                row,
                player,
                &known_terrain,
                substances.as_deref(),
                &unit_states,
                restorable.as_ref(),
            );
            if let Some(hud) = hud.as_deref_mut() {
                let _ = hud.dismiss_transient_component(HudComponent::ActionBar);
            }
            Some(Aim {
                anchor: step_target(&targets, aim.anchor).unwrap_or(aim.anchor),
                spell: aim.spell,
            })
        }
        AimRequest::Confirm => {
            let Some(aim) = aiming.0.clone() else { return };
            if !knowledge.as_deref().is_some_and(|knowledge| {
                knowledge.faction(Faction::Player).state(aim.anchor) == KnowledgeState::Observed
            }) {
                return;
            }
            if !emit_cast(&mut queue, &readout, &caster, &aim) {
                return;
            }
            *exit = AimExit::Confirmed;
            // The intent is spent whether or not the applier likes it. Leaving the aim
            // up would invite a second confirm against a lattice that is about to
            // change, and the applier's own answer arrives a schedule later.
            None
        }
    };

    if aiming.0 != next {
        aiming.0 = next;
    }
}

/// What the player asked of the aim.
enum AimRequest {
    /// Start aiming this spell.
    Choose(String),
    /// Point at the next unit in range.
    Next,
    /// Emit the cast.
    Confirm,
    /// Put the spell down.
    Cancel,
}

/// The one request this frame, from a button or a key.
///
/// Exactly one, which is the point: a confirm arriving from both a button and its
/// keyboard shortcut in the same frame must still be one cast.
fn requested(
    keys: &ButtonInput<KeyCode>,
    bindings: &InputBindings,
    focus_owns_shortcuts: bool,
    intents: &mut MessageReader<hex_ui::UiIntent>,
) -> Option<AimRequest> {
    for intent in intents.read() {
        if let hex_ui::UiIntent::Casting(intent) = intent {
            return Some(match intent {
                hex_ui::CastingIntent::Begin(spell) => AimRequest::Choose(spell.clone()),
                hex_ui::CastingIntent::Confirm => AimRequest::Confirm,
                hex_ui::CastingIntent::NextTarget => AimRequest::Next,
                hex_ui::CastingIntent::Cancel => AimRequest::Cancel,
            });
        }
    }
    raw_aim_request(keys, bindings, focus_owns_shortcuts)
}

fn raw_aim_request(
    keys: &ButtonInput<KeyCode>,
    bindings: &InputBindings,
    focus_owns_shortcuts: bool,
) -> Option<AimRequest> {
    if !focus_owns_shortcuts && bindings.just_pressed(keys, InputAction::Confirm) {
        return Some(AimRequest::Confirm);
    }
    if !focus_owns_shortcuts && bindings.just_pressed(keys, InputAction::NextTarget) {
        return Some(AimRequest::Next);
    }
    if bindings.just_pressed(keys, InputAction::CancelCast) {
        return Some(AimRequest::Cancel);
    }
    None
}

/// Pushes the cast, or reports that it was folded.
///
/// An emitter, like every other input in this codebase: legality belongs to the one
/// applier, so a cast it refuses is refused there with a reason rather than silently
/// doing nothing here.
fn emit_cast(queue: &mut CommandQueue, readout: &CastReadout, caster: &Caster, aim: &Aim) -> bool {
    if readout.unavailable.is_some() {
        return false;
    }
    let Some(row) = readout.row(&aim.spell) else {
        return false;
    };
    if row.blocked.is_some() {
        return false;
    }
    // Two confirms in one frame are one intent — the same fold the click-to-move
    // emitter does, and for the same reason: the second would reach the applier only to
    // die in its busy gate as a warned drop.
    if queue.holds_command_for(caster.unit) {
        return false;
    }

    queue.push(IssuedCommand {
        seat: caster.seat,
        command: GameCommand::Cast {
            unit: caster.unit,
            spell: aim.spell.clone(),
            target: aim.anchor,
            // Only the shapes that point somewhere carry a facing. Sending one anyway
            // would put a direction nobody chose into the future recorded command
            // stream, whose wire fields are save/replay commitments.
            facing: volumes::needs_facing(&row.shape)
                .then(|| facing_toward(caster.standing.coord, aim.anchor.coord)),
            // Variable mana has no chooser yet; see `spell_row`.
            mana: None,
        },
    });
    true
}

/// The surfaces of units this spell could be aimed at, nearest hostile first.
///
/// The cycle list, and deliberately not the whole legal anchor set: a range-4 spell
/// covers dozens of surfaces, and stepping a key through all of them is not a targeting
/// interface. Clicking is how a bare surface is chosen; this is how a *unit* is.
///
/// Ordered by hostility, then grid distance, then the whole [`TilePos`] — never by the
/// bare coordinate, which would collapse a bridge onto the ground beneath it and leave
/// the order depending on query iteration.
///
/// The caster is in its own list when it is in its own range, because there is no
/// ally-or-enemy targeting filter and there will not be one: you may heal an enemy and
/// immolate a friend.
///
/// Downed units remain spatially visible. The target-cycle shortcut omits them for
/// ordinary spells and includes them for restoration, whose purpose includes revival.
/// The live-unit query is only intersected with authorized observations; it can never
/// add a hidden identity.
fn targets_in_range(
    readout: &CastReadout,
    caster: &Caster,
    row: &SpellRow,
    knowledge: &FactionKnowledge,
    terrain: &KnownTerrainOccupancy,
    substances: Option<&SubstanceTable>,
    unit_states: &Query<(&UnitId, Has<Downed>)>,
    restorable: Option<&BTreeSet<UnitId>>,
) -> Vec<TilePos> {
    if matches!(row.shape, TargetShape::SelfCast) {
        return (knowledge.state(caster.standing) == KnowledgeState::Observed
            && trajectory_available(
                row.trajectory,
                caster.standing,
                caster.standing,
                row.creates_terrain,
                terrain,
            ))
        .then_some(caster.standing)
        .into_iter()
        .collect();
    }
    let mut ranked: Vec<(bool, u32, TilePos)> = knowledge
        .units()
        .filter(|(_, unit)| {
            unit_states
                .iter()
                .any(|(id, downed)| *id == unit.id && (row.restores || !downed))
        })
        .filter(|(_, unit)| restorable.is_none_or(|allowed| allowed.contains(&unit.id)))
        .map(|(_, unit)| (unit.faction, unit.pos))
        .filter(|(_, pos)| {
            knowledge.state(*pos) == KnowledgeState::Observed
                && aim_target_reachable(
                    row,
                    caster,
                    *pos,
                    Some(knowledge),
                    substances,
                    readout.levels_per_bonus,
                )
                && trajectory_available(
                    row.trajectory,
                    caster.standing,
                    *pos,
                    row.creates_terrain,
                    terrain,
                )
        })
        .map(|(faction, pos)| {
            (
                !Faction::Player.is_hostile_to(faction),
                volumes::grid_distance(caster.standing, pos),
                pos,
            )
        })
        .collect();
    ranked.sort_unstable();
    ranked.into_iter().map(|(_, _, pos)| pos).collect()
}

fn trajectory_available(
    trajectory: Trajectory,
    standing: TilePos,
    target: TilePos,
    creates_terrain: bool,
    terrain: &KnownTerrainOccupancy,
) -> bool {
    matches!(trajectory, Trajectory::None)
        || known_trajectory_is_clear(
            trajectory,
            standing.above(),
            trajectory_destination(target, creates_terrain),
            terrain,
        )
}

fn known_terrain(knowledge: &FactionKnowledge) -> KnownTerrainOccupancy {
    KnownTerrainOccupancy::from_observed_surfaces(
        knowledge
            .surfaces()
            .filter(|(_, known)| known.state() == KnowledgeState::Observed)
            .map(|(position, _)| position),
    )
}

/// The entry after `current` in a cycle, wrapping.
///
/// An anchor that is not in the list — a bare surface the player clicked — steps to the
/// first entry rather than nowhere, so the key always moves.
fn step_target(targets: &[TilePos], current: TilePos) -> Option<TilePos> {
    match targets.iter().position(|target| *target == current) {
        Some(at) => targets.get((at + 1) % targets.len()).copied(),
        None => targets.first().copied(),
    }
}

/// The sextant pointing from one coordinate toward another.
///
/// The direction a `Line`, `Cone` or `Path` is fired in, taken from where the player
/// aimed rather than asked for separately: pointing a flamethrower at somebody is the
/// same gesture as choosing them.
///
/// Picked by trying all six and keeping the step that lands nearest, which is exact on
/// cube coordinates and needs no angle. Ties break by [`Sextant::ALL`]'s order, so a
/// target exactly between two directions always resolves the same way — including the
/// degenerate case of aiming at your own coordinate, which is every direction at once.
#[must_use]
pub fn facing_toward(from: HexCoord, to: HexCoord) -> Sextant {
    let mut best = Sextant::A;
    let mut nearest = u32::MAX;
    for sextant in Sextant::ALL {
        let distance = from.neighbor(sextant).distance(to);
        if distance < nearest {
            nearest = distance;
            best = sextant;
        }
    }
    best
}

/// Puts the aimed spell down on leaving gameplay.
///
/// The aim names a `TilePos` and a unit that will not exist in the next session — unit
/// ids are dealt from zero at every launch — so carrying one over would point the next
/// fight at whatever happens to be standing there.
fn forget_aim(mut aiming: ResMut<Aiming>) {
    aiming.0 = None;
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::SystemState;
    use hex_assets::{ArtPalette, ElementFile, SpellFile, SubstanceFile, SubstanceTable};
    use hex_combat::BaseVisibility;
    use hex_core::{ElementId, Headroom, HexSpan, Level, LightDomain, SubstanceId};
    use hex_lattice::{apply_disables, LatticeStats};
    use hex_perception::{
        apply_observations, FactionObservation, FactionObservations, ObservedUnit, SurfaceSnapshots,
    };

    use super::*;

    fn shipped_substances() -> SubstanceTable {
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
        SubstanceTable::from_file(&substance_file, &palette)
            .expect("shipped substances resolve through the art palette")
    }

    #[test]
    fn focused_ui_owns_enter_and_tab_without_hiding_the_explicit_cancel_key() {
        let bindings = InputBindings::default();

        for key in [KeyCode::Enter, KeyCode::Tab] {
            let mut keys = ButtonInput::default();
            keys.press(key);
            assert!(raw_aim_request(&keys, &bindings, true).is_none());
        }

        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyQ);
        assert!(matches!(
            raw_aim_request(&keys, &bindings, true),
            Some(AimRequest::Cancel)
        ));
    }
    use hex_ui::{FUSION_COLOR, GEM_COLOR};

    fn shipped_content() -> (ElementCatalog, SpellBook) {
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
        let elements = ElementCatalog::from_file(&element_file);
        let spells = SpellBook::from_file(&spell_file);
        let substances = shipped_substances();
        ContentIndex::build(&elements, &spells, &substances)
            .expect("shipped content cross-references resolve");
        (elements, spells)
    }

    #[test]
    fn heal_readout_says_touch_instead_of_range_zero() {
        let (elements, spells) = shipped_content();
        let heal = spells
            .id("Heal")
            .and_then(|id| spells.spell(id))
            .expect("Heal is shipped");
        let row = spell_row("Heal", heal, None, &elements);

        assert_eq!(row.reach, TargetingReach::Touch);
        assert!(row.restores);
        assert!(row.cost.contains("touch"));
        assert!(!row.cost.contains("range 0"));
    }

    fn observed_spatial(units: &[(UnitId, Faction, TilePos)]) -> FactionMapKnowledge {
        let snapshots =
            SurfaceSnapshots::try_from_iter(units.iter().map(|(_, _, pos)| SurfaceSnapshot {
                pos: *pos,
                span: HexSpan::new(0.0, 1.0),
                substance: SubstanceId(10),
                headroom: Headroom(3),
                is_solid: true,
                blocked: false,
                domain: LightDomain::Exterior,
            }))
            .expect("restoration fixtures occupy distinct surfaces");
        let mut observation = FactionObservation::new();
        for (id, faction, pos) in units {
            observation.insert_surface(*pos);
            observation
                .try_insert_unit(ObservedUnit {
                    id: *id,
                    faction: *faction,
                    pos: *pos,
                    provides_sight: true,
                })
                .expect("restoration fixtures use distinct unit ids");
        }
        let observations = FactionObservations::with_faction(Faction::Player, observation);
        let mut spatial = FactionMapKnowledge::new();
        apply_observations(&mut spatial, &snapshots, &observations);
        spatial
    }

    #[test]
    fn restoration_presentation_excludes_healthy_and_opaque_hostile_targets() {
        let damaged_ally = UnitId(1);
        let healthy_ally = UnitId(2);
        let opaque_hostile = UnitId(3);
        let units = [
            (damaged_ally, Faction::Player, at(0, 0, 4)),
            (healthy_ally, Faction::Player, at(1, 0, 4)),
            (opaque_hostile, Faction::Hostile, at(0, 1, 4)),
        ];
        let spatial = observed_spatial(&units);
        let spec = LatticeSpec::default().with(LatticeCoord::ORIGIN, CellKind::Blank);
        let healthy = LatticeState::new(&spec, &LatticeStats::default());
        let mut damaged = healthy.clone();
        apply_disables(&mut damaged, &[LatticeCoord::ORIGIN]);

        let mut world = World::new();
        world.spawn((damaged_ally, Faction::Player, spec.clone(), damaged.clone()));
        world.spawn((healthy_ally, Faction::Player, spec.clone(), healthy));
        world.spawn((opaque_hostile, Faction::Hostile, spec, damaged));

        let mut lattice_knowledge = FactionLatticeKnowledge::default();
        lattice_knowledge.observe_base(
            Faction::Player,
            opaque_hostile,
            BaseVisibility {
                faction: Faction::Hostile,
            },
        );

        let mut query_state =
            SystemState::<RestorationTargetQuery<'static, 'static>>::new(&mut world);
        let targets = query_state
            .get(&world)
            .expect("the restoration query has no fallible parameters");
        let allowed = restorable_target_ids(
            spatial.faction(Faction::Player),
            Faction::Player,
            &lattice_knowledge,
            &targets,
        );

        assert_eq!(allowed, BTreeSet::from([damaged_ally]));
        assert!(
            !allowed.contains(&healthy_ally),
            "restoration has no no-op marker"
        );
        assert!(
            !allowed.contains(&opaque_hostile),
            "live hostile lattice damage must not leak through an opaque view"
        );
    }

    #[test]
    fn restoration_target_cycle_includes_damaged_self_and_touch_ally() {
        type UnitStateQuery<'w, 's> = Query<'w, 's, (&'static UnitId, Has<Downed>)>;

        let (readout, caster) = readout_of(&["Heal"]);
        let row = readout.row("Heal").expect("the Heal row should exist");
        let ally = UnitId(2);
        let ally_pos = at(1, 0, caster.standing.level);
        let units = [
            (caster.unit, Faction::Player, caster.standing),
            (ally, Faction::Player, ally_pos),
        ];
        let spatial = observed_spatial(&units);
        let spec = LatticeSpec::default().with(LatticeCoord::ORIGIN, CellKind::Blank);
        let mut damaged = LatticeState::new(&spec, &LatticeStats::default());
        apply_disables(&mut damaged, &[LatticeCoord::ORIGIN]);

        let mut world = World::new();
        world.spawn((caster.unit, Faction::Player, spec.clone(), damaged.clone()));
        world.spawn((ally, Faction::Player, spec, damaged));

        let lattice_knowledge = FactionLatticeKnowledge::default();
        let mut query_state = SystemState::<(
            UnitStateQuery<'static, 'static>,
            RestorationTargetQuery<'static, 'static>,
        )>::new(&mut world);
        let (unit_states, restoration_targets) = query_state
            .get(&world)
            .expect("the restoration queries have no fallible parameters");
        let player = spatial.faction(Faction::Player);
        let allowed = restorable_target_ids(
            player,
            Faction::Player,
            &lattice_knowledge,
            &restoration_targets,
        );
        let terrain = known_terrain(player);
        let substances = shipped_substances();

        assert_eq!(allowed, BTreeSet::from([caster.unit, ally]));
        assert_eq!(
            targets_in_range(
                &readout,
                &caster,
                row,
                player,
                &terrain,
                Some(&substances),
                &unit_states,
                Some(&allowed),
            ),
            vec![caster.standing, ally_pos],
        );
    }

    #[test]
    fn restoration_ui_intent_aims_damaged_self_then_touch_ally() {
        let (readout, caster) = readout_of(&["Heal"]);
        let ally = UnitId(2);
        let ally_pos = at(1, 0, caster.standing.level);
        let units = [
            (caster.unit, Faction::Player, caster.standing),
            (ally, Faction::Player, ally_pos),
        ];
        let spatial = observed_spatial(&units);
        let spec = LatticeSpec::default().with(LatticeCoord::ORIGIN, CellKind::Blank);
        let mut damaged = LatticeState::new(&spec, &LatticeStats::default());
        apply_disables(&mut damaged, &[LatticeCoord::ORIGIN]);
        let mut hud = HudState::default();
        let preferences = hud.preferences();
        let compact = hex_gameplay_model::HudContext::compact(
            hex_gameplay_model::HudContextEligibility::all(),
        );
        assert_eq!(
            hud.activate_component(HudComponent::ActionBar, compact),
            hex_gameplay_model::HudActionResult::RuntimeChanged
        );

        let mut app = App::new();
        app.insert_resource(readout)
            .insert_resource(hud)
            .init_resource::<Aiming>()
            .init_resource::<AimExit>()
            .init_resource::<CommandQueue>()
            .init_resource::<PendingDecision>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<InputBindings>()
            .insert_resource(spatial)
            .init_resource::<FactionLatticeKnowledge>()
            .insert_resource(shipped_substances())
            .add_message::<hex_ui::UiIntent>()
            .add_systems(Update, resolve_aim_input);
        app.world_mut()
            .spawn((caster.unit, Faction::Player, spec.clone(), damaged.clone()));
        app.world_mut()
            .spawn((ally, Faction::Player, spec, damaged));

        app.world_mut()
            .write_message(hex_ui::UiIntent::Casting(hex_ui::CastingIntent::Begin(
                "Heal".to_owned(),
            )));
        app.update();
        assert_eq!(
            app.world().resource::<Aiming>().0.as_ref(),
            Some(&Aim {
                spell: "Heal".to_owned(),
                anchor: caster.standing,
            })
        );
        assert_eq!(app.world().resource::<HudState>().raw_transient(), None);
        assert_eq!(
            app.world().resource::<HudState>().preferences(),
            preferences,
            "revealing the map for an aim must not rewrite HUD preferences"
        );

        app.world_mut()
            .write_message(hex_ui::UiIntent::Casting(hex_ui::CastingIntent::NextTarget));
        app.update();
        assert_eq!(
            app.world().resource::<Aiming>().0.as_ref(),
            Some(&Aim {
                spell: "Heal".to_owned(),
                anchor: ally_pos,
            })
        );
    }

    fn at(x: i32, y: i32, level: Level) -> TilePos {
        TilePos::new(HexCoord::from_axial(x, y), level)
    }

    /// Aiming at a hex several steps out resolves to the direction it lies in.
    ///
    /// The property a directed spell depends on: if this and `hex_units::volumes`'s
    /// rotation ever disagree, a flamethrower burns a different sextant from the one
    /// the player pointed at, and nothing else in the game would notice.
    #[test]
    fn a_facing_points_at_what_was_aimed_at() {
        for sextant in Sextant::ALL {
            let mut target = HexCoord::ORIGIN;
            for step in 1..=5 {
                target = target.neighbor(sextant);
                assert_eq!(
                    facing_toward(HexCoord::ORIGIN, target),
                    sextant,
                    "{step} steps along {sextant:?} should read back as {sextant:?}"
                );
            }
        }
    }

    /// Aiming at your own coordinate is every direction at once, and must still be one.
    #[test]
    fn a_facing_at_no_distance_is_deterministic() {
        assert_eq!(
            facing_toward(HexCoord::ORIGIN, HexCoord::ORIGIN),
            Sextant::A
        );
    }

    /// Cycling visits every target and comes back, from any starting point.
    #[test]
    fn stepping_targets_wraps_and_starts_from_an_unlisted_anchor() {
        let targets = [at(1, 0, 4), at(2, 0, 4), at(1, 0, 9)];
        assert_eq!(step_target(&targets, at(1, 0, 4)), Some(at(2, 0, 4)));
        assert_eq!(step_target(&targets, at(2, 0, 4)), Some(at(1, 0, 9)));
        // Two surfaces stacked at one coordinate are separate targets, so the wrap has
        // to come off the third rather than the second.
        assert_eq!(step_target(&targets, at(1, 0, 9)), Some(at(1, 0, 4)));
        assert_eq!(step_target(&targets, at(7, 7, 0)), Some(at(1, 0, 4)));
        assert_eq!(step_target(&[], at(1, 0, 4)), None);
    }

    /// Every canonical element gets its own authored colour, including fusions.
    #[test]
    fn canonical_elements_have_distinct_authored_colours() {
        let (elements, _) = shipped_content();
        let mut seen: Vec<Color> = Vec::new();
        for raw_id in 0..elements.len() {
            let id = ElementId(u16::try_from(raw_id).expect("element catalog fits in an id"));
            let name = elements.name(id).expect("a catalog element has a name");
            let color = element_color(elements.id(name), &elements);
            assert!(
                !seen.contains(&color),
                "{name} shares a colour with another canonical element"
            );
            seen.push(color);
        }
        assert_ne!(
            element_color(elements.id("Lightning"), &elements),
            FUSION_COLOR,
            "canonical fusions use their authored school tint"
        );
        assert_eq!(
            element_color(elements.id("not an element"), &elements),
            GEM_COLOR,
            "an unresolvable element falls back rather than vanishing"
        );
    }

    /// A custom catalog remains legible even though it has no authored icon catalog.
    #[test]
    fn custom_elements_keep_deterministic_fallback_colours() {
        let custom: ElementFile = ron::from_str(
            r#"(
                wheel: ["Aether", "Flame", "Stone", "Void", "Frost", "Bloom"],
                fusions: {
                    "Tempest": [
                        (element: "Aether", mana: 1),
                        (element: "Flame", mana: 1),
                    ],
                },
            )"#,
        )
        .expect("custom elements parse");
        let elements = ElementCatalog::from_file(&custom);
        let wheel = elements.wheel();
        let half = wheel.len() / 2;
        for (step, id) in wheel.iter().enumerate() {
            let Some(opposite) = wheel.get((step + half) % wheel.len()) else {
                continue;
            };
            let name = elements.name(*id).expect("a wheel element has a name");
            let against = elements.name(*opposite).expect("its opposite has a name");
            let hue = Hsla::from(element_color(elements.id(name), &elements)).hue;
            let other = Hsla::from(element_color(elements.id(against), &elements)).hue;
            let apart = (hue - other).abs();
            assert!(
                (apart - 180.0).abs() < 0.5,
                "{name} and {against} are {apart} degrees apart, not 180"
            );
        }
        assert_eq!(
            element_color(elements.id("Tempest"), &elements),
            FUSION_COLOR,
            "a custom fusion retains the generic fusion fallback"
        );
    }

    /// Every shipped spell becomes a row that says what it costs and how it reaches.
    ///
    /// Content-driven end to end: if a spell is renamed or its targeting changes, this
    /// is what notices before a player does.
    #[test]
    fn every_shipped_spell_becomes_a_row() {
        let (elements, spells) = shipped_content();
        for (_, name, definition) in spells.iter() {
            let row = spell_row(name, definition, None, &elements);
            assert_eq!(row.name, name);
            assert!(!row.detail.is_empty(), "{name} has no identity line");
            let expected_reach = match definition.targeting.reach {
                TargetingReach::Ranged => "range",
                TargetingReach::Touch => "touch",
            };
            assert!(
                row.cost.contains(expected_reach),
                "{name} does not say its {expected_reach} reach"
            );
            assert_eq!(row.range, u32::from(definition.targeting.range));
            assert_eq!(row.shape, definition.targeting.shape);
        }
    }

    /// A blocked row carries the reason, not just the absence of a button.
    #[test]
    fn a_blocked_row_says_why() {
        let (elements, spells) = shipped_content();
        let ember = spells.id("Ember").expect("Ember ships");
        let definition = spells.spell(ember).expect("Ember has a definition");
        let row = spell_row(
            "Ember",
            definition,
            Some(blocked_reason(&CastBlocked::Unsatisfiable)),
            &elements,
        );
        assert_eq!(row.blocked, Some("not enough adjacent mana"));
    }

    /// Height buys range for a spell exactly as it does for engagement.
    ///
    /// The interface has to agree with the applier about this or it lights surfaces a
    /// cast is then refused for — the one direction of disagreement that is a bug.
    #[test]
    fn the_interface_measures_range_the_way_the_applier_does() {
        let high = at(0, 0, 9);
        let low = at(4, -4, 4);
        assert!(
            !in_range(at(0, 0, 4), low, 3, 5),
            "level ground falls short"
        );
        assert!(in_range(high, low, 3, 5), "five levels up buys the hex");
        assert!(!in_range(low, high, 3, 5), "the low ground gains nothing");
    }

    #[test]
    fn target_cycle_trajectory_filter_ignores_unknown_material() {
        let standing = at(0, 0, 1);
        let target = at(3, 0, 1);
        let blocker = at(1, 0, 2);
        let hidden = KnownTerrainOccupancy::default();
        let observed = KnownTerrainOccupancy::from_observed_surfaces([blocker]);

        assert!(trajectory_available(
            Trajectory::Direct,
            standing,
            target,
            false,
            &hidden,
        ));
        assert!(!trajectory_available(
            Trajectory::Direct,
            standing,
            target,
            false,
            &observed,
        ));
    }

    /// Every rung the applier refuses a cast on is a rung the panel refuses to offer
    /// one on, and it says which.
    ///
    /// The direction that matters: a panel that offered a cast the applier then refused
    /// would spend a player's attention on a button that does nothing.
    #[test]
    fn the_panel_stops_where_the_applier_would() {
        let unit = UnitId(1);
        let ready = Turn {
            movement_left: 4,
            acted: false,
        };
        let spent = Turn {
            movement_left: 4,
            acted: true,
        };
        assert_eq!(
            unavailable_reason(false, Some(unit), false, unit, Some(&ready), false),
            Some("casting is combat-only for now")
        );
        assert_eq!(
            unavailable_reason(true, Some(UnitId(2)), false, unit, Some(&ready), false),
            Some("not this unit's turn")
        );
        assert_eq!(
            unavailable_reason(true, Some(unit), false, unit, None, false),
            Some("no turn to take the action from")
        );
        assert_eq!(
            unavailable_reason(true, Some(unit), false, unit, Some(&spent), false),
            Some("action already spent this turn")
        );
        assert_eq!(
            unavailable_reason(true, Some(unit), false, unit, Some(&ready), true),
            Some("still finishing the last action")
        );
        assert_eq!(
            unavailable_reason(true, Some(unit), true, unit, Some(&ready), false),
            Some("a decision is still open")
        );
        assert_eq!(
            unavailable_reason(true, Some(unit), false, unit, Some(&ready), false),
            None,
            "an acting unit with its action still in hand should be offered its spells"
        );
    }

    /// A readout for one caster holding the named spells, as the panel would show them.
    fn readout_of(names: &[&str]) -> (CastReadout, Caster) {
        let (elements, spells) = shipped_content();
        let caster = Caster {
            unit: UnitId(1),
            seat: PlayerSeat::default(),
            standing: at(0, 0, 4),
            body: Body::new(hex_core::TraversalProfile::WALKER),
        };
        let rows = names
            .iter()
            .map(|name| {
                let id = spells.id(name).expect("the test names a shipped spell");
                let definition = spells.spell(id).expect("a shipped spell has a definition");
                spell_row(name, definition, None, &elements)
            })
            .collect();
        (
            CastReadout {
                caster: Some(caster),
                unavailable: None,
                spells: rows,
                levels_per_bonus: 5,
            },
            caster,
        )
    }

    /// A confirmed cast carries the exact anchor, and a facing only when the shape
    /// points somewhere.
    ///
    /// The facing is a future save/replay commitment, so an anchored shape must not
    /// acquire a direction nobody chose.
    #[test]
    fn a_confirmed_cast_names_its_anchor_and_only_the_facing_it_needs() {
        let (readout, caster) = readout_of(&["Ember", "Flamethrower"]);
        let mut queue = CommandQueue::default();
        let anchor = at(2, -2, 4);

        assert!(emit_cast(
            &mut queue,
            &readout,
            &caster,
            &Aim {
                spell: "Ember".to_owned(),
                anchor,
            }
        ));
        match queue.pop().map(|issued| issued.command) {
            Some(GameCommand::Cast {
                unit,
                spell,
                target,
                facing,
                mana,
            }) => {
                assert_eq!(unit, caster.unit);
                assert_eq!(spell, "Ember");
                assert_eq!(target, anchor);
                assert_eq!(facing, None, "a Single shape points nowhere");
                assert_eq!(mana, None);
            }
            other => panic!("expected a cast, got {other:?}"),
        }

        assert!(emit_cast(
            &mut queue,
            &readout,
            &caster,
            &Aim {
                spell: "Flamethrower".to_owned(),
                anchor,
            }
        ));
        match queue.pop().map(|issued| issued.command) {
            Some(GameCommand::Cast { facing, .. }) => assert_eq!(
                facing,
                Some(facing_toward(caster.standing.coord, anchor.coord)),
                "a Line shape fires the way the player aimed"
            ),
            other => panic!("expected a cast, got {other:?}"),
        }
    }

    /// Nothing is emitted for a spell the interface is not offering, and one intent is
    /// one command however many times confirm arrives in a frame.
    #[test]
    fn a_cast_is_not_emitted_twice_or_for_a_spell_that_is_not_offered() {
        let (mut readout, caster) = readout_of(&["Ember"]);
        let aim = Aim {
            spell: "Ember".to_owned(),
            anchor: at(1, -1, 4),
        };
        let mut queue = CommandQueue::default();

        assert!(emit_cast(&mut queue, &readout, &caster, &aim));
        assert!(
            !emit_cast(&mut queue, &readout, &caster, &aim),
            "a second confirm against an unconsumed queue is the same intent"
        );
        assert_eq!(queue.len(), 1);

        let unknown = Aim {
            spell: "Not A Spell".to_owned(),
            anchor: aim.anchor,
        };
        assert!(!emit_cast(
            &mut CommandQueue::default(),
            &readout,
            &caster,
            &unknown
        ));

        readout.unavailable = Some("not this unit's turn");
        assert!(!emit_cast(
            &mut CommandQueue::default(),
            &readout,
            &caster,
            &aim
        ));

        readout.unavailable = None;
        if let Some(row) = readout.spells.first_mut() {
            row.blocked = Some("not enough adjacent mana");
        }
        assert!(!emit_cast(
            &mut CommandQueue::default(),
            &readout,
            &caster,
            &aim
        ));
    }

    /// A walk suspends casting without putting the aim down; the turn ending ends it.
    ///
    /// The distinction this asserts is the whole of `keeps_the_aim`, and collapsing it is
    /// what made the module's advertised "reposition, then cast" flow impossible: the
    /// `Busy` a walk sets arrives one frame after the click that started it, so an aim
    /// dropped on any unavailability is an aim dropped by the act of walking.
    #[test]
    fn transient_unavailability_suspends_an_aim_and_a_finished_turn_ends_it() {
        assert!(keeps_the_aim(BUSY), "mid-walk, and about to arrive");
        assert!(keeps_the_aim(DECISION_OPEN), "waiting on somebody else");
        assert!(!keeps_the_aim(ACTION_SPENT), "the action is gone");
        assert!(!keeps_the_aim(NOT_YOUR_TURN));
        assert!(!keeps_the_aim(NO_TURN));
        assert!(!keeps_the_aim(OUT_OF_COMBAT));
    }

    /// Walking carries the aim, and the anchor is re-measured from where the caster lands.
    ///
    /// Both directions matter, and only the second is obvious. Stepping *toward* a target
    /// brings an out-of-reach anchor into range, which is the point of repositioning.
    /// Stepping *away* carries a chosen anchor out of it, and an aim that survived that
    /// would let the player confirm a cast the applier then refuses — the one direction
    /// of disagreement this module's header calls a bug.
    #[test]
    fn an_aim_is_re_measured_from_wherever_the_caster_lands() {
        let (mut readout, _) = readout_of(&["Ember"]);
        let range = readout.row("Ember").expect("Ember is offered").range;
        let levels = readout.levels_per_bonus;

        // An anchor just inside Ember's range from the origin.
        let anchor = at(i32::try_from(range).expect("a small range"), 0, 4);
        let reachable = |from: TilePos| in_range(from, anchor, range, levels);
        assert!(
            reachable(at(0, 0, 4)),
            "precondition: in reach to begin with"
        );

        // Walking one hex further away puts it out, and the aim goes with it.
        let retreated = at(-1, 0, 4);
        assert!(
            !reachable(retreated),
            "precondition: the fixture must actually leave range"
        );

        // And being Busy alone — the state a walk leaves behind — does not.
        readout.unavailable = Some(BUSY);
        assert!(
            readout.unavailable.is_none_or(keeps_the_aim),
            "a walk in progress must not be a reason to drop the aim"
        );
        readout.unavailable = Some(ACTION_SPENT);
        assert!(
            !readout.unavailable.is_none_or(keeps_the_aim),
            "a spent action must be"
        );
    }

    /// A spell whose every effect is unbuilt is offered as blocked, never as a button.
    ///
    /// The failure without this is invisible and expensive: the cast is *legal*, so the
    /// applier charges for it — the mana goes, the turn goes — and the only trace is a
    /// `warn!` a release build does not show a console for. The canonical roster keeps
    /// no such placeholder spell, so every shipped definition is the live regression.
    #[test]
    fn every_shipped_spell_has_a_delivered_effect_path() {
        let (_, spells) = shipped_content();
        for (id, name, definition) in spells.iter() {
            assert!(
                hex_combat::delivers_anything(definition),
                "{name} ({id:?}) has no delivered combat result and must not ship"
            );
        }
        assert!(
            spells.id("Daylight").is_none(),
            "Illuminate remains deferred"
        );
    }
}
