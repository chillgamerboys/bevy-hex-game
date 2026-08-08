//! Initiative application adapter. Rendering belongs to `hex_ui`.

use bevy::prelude::*;
use hex_combat::{CombatSystems, TurnOrder};
use hex_core::{AppSystems, GameplaySystems, Screen};
use hex_perception::FactionMapKnowledge;
use hex_ui::{InitiativeEntryView, InitiativeSide, InitiativeView};
use hex_units::{Faction, UnitRegistry};

use crate::readouts::GameplayUiContext;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        publish_view
            .in_set(AppSystems::Update)
            .after(CombatSystems::Advance)
            .after(GameplaySystems::UiContext)
            .before(hex_ui::UiSystems::Render)
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn publish_view(
    order: Res<TurnOrder>,
    registry: Res<UnitRegistry>,
    identities: Query<(&Name, &Faction)>,
    knowledge: Option<Res<FactionMapKnowledge>>,
    context: Option<Res<GameplayUiContext>>,
    mut view: ResMut<InitiativeView>,
) {
    let current = order.current();
    let entries = order
        .order()
        .iter()
        .filter_map(|unit| {
            let entity = registry.entity_of(*unit)?;
            let (name, faction) = identities.get(entity).ok()?;
            let inspectable = *faction == Faction::Player
                || knowledge.as_deref().is_some_and(|knowledge| {
                    knowledge.faction(Faction::Player).unit(*unit).is_some()
                });
            Some(InitiativeEntryView {
                unit: *unit,
                name: if inspectable {
                    name.as_str().to_owned()
                } else {
                    "Unobserved hostile".to_owned()
                },
                side: match faction {
                    Faction::Player => InitiativeSide::Ally,
                    Faction::Hostile => InitiativeSide::Hostile,
                },
                current: current == Some(*unit),
                inspectable,
            })
        })
        .collect();
    let heading = context
        .as_deref()
        .and_then(|context| context.acting.as_ref())
        .map_or_else(
            || "turn order".to_owned(),
            |actor| match actor.faction {
                Faction::Player => "your turn".to_owned(),
                Faction::Hostile => "enemy turn".to_owned(),
            },
        );
    let next = InitiativeView { heading, entries };
    if *view != next {
        *view = next;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hex_combat::Initiative;
    use hex_core::{Headroom, HexCoord, HexSpan, LightDomain, Mode, SubstanceId, TilePos, UnitId};
    use hex_perception::{
        apply_observations, FactionObservation, FactionObservations, ObservedUnit, SurfaceSnapshot,
        SurfaceSnapshots,
    };
    use hex_test_support::TestAppBuilder;

    use super::*;

    #[derive(Resource, Debug, Default)]
    struct RenderedInitiative(InitiativeView);

    fn capture_rendered_view(view: Res<InitiativeView>, mut rendered: ResMut<RenderedInitiative>) {
        rendered.0.clone_from(&view);
    }

    fn test_app() -> App {
        let mut builder = TestAppBuilder::new().with_fixed_step(Duration::ZERO);
        let app = builder.app_mut();
        app.insert_resource(hex_assets::CombatSettings::default())
            .init_resource::<InitiativeView>()
            .init_resource::<RenderedInitiative>()
            .add_plugins(hex_combat::plugin)
            .add_plugins(plugin)
            .add_systems(
                Update,
                capture_rendered_view
                    .in_set(hex_ui::UiSystems::Render)
                    .run_if(in_state(Screen::Gameplay)),
            );
        builder.build()
    }

    fn observed_hostile_knowledge(hostile: UnitId) -> FactionMapKnowledge {
        let pos = TilePos::new(HexCoord::ORIGIN, 1);
        let surfaces = SurfaceSnapshots::try_from_iter([SurfaceSnapshot {
            pos,
            span: HexSpan::new(0.0, 1.0),
            substance: SubstanceId(1),
            headroom: Headroom(2),
            is_solid: true,
            blocked: false,
            domain: LightDomain::Exterior,
        }])
        .expect("the disclosure fixture has one unique surface");
        let mut player = FactionObservation::new();
        player.insert_surface(pos);
        player
            .try_insert_unit(ObservedUnit {
                id: hostile,
                faction: Faction::Hostile,
                pos,
                provides_sight: true,
            })
            .expect("the disclosure fixture has one unique hostile");
        let observations = FactionObservations::with_faction(Faction::Player, player);
        let mut knowledge = FactionMapKnowledge::new();
        apply_observations(&mut knowledge, &surfaces, &observations);
        knowledge
    }

    #[test]
    fn projected_order_keeps_canonical_identity_and_side_labels() {
        let view = InitiativeView {
            heading: "your turn".to_owned(),
            entries: vec![
                InitiativeEntryView {
                    unit: UnitId(4),
                    name: "mage #4".to_owned(),
                    side: InitiativeSide::Ally,
                    current: true,
                    inspectable: true,
                },
                InitiativeEntryView {
                    unit: UnitId(9),
                    name: "wolf #9".to_owned(),
                    side: InitiativeSide::Hostile,
                    current: false,
                    inspectable: false,
                },
            ],
        };
        assert_eq!(
            view.entries.first().map(|entry| entry.unit),
            Some(UnitId(4))
        );
        assert_eq!(
            view.entries.last().map(|entry| entry.side),
            Some(InitiativeSide::Hostile)
        );
    }

    #[test]
    fn rendered_initiative_anonymizes_and_reveals_hostiles_in_the_same_update() {
        let mut app = test_app();
        let ally = UnitId(4);
        let hostile = UnitId(9);
        app.world_mut()
            .spawn((ally, Name::new("mage #4"), Faction::Player, Initiative(20)));
        app.world_mut().spawn((
            hostile,
            Name::new("wolf #9"),
            Faction::Hostile,
            Initiative(10),
        ));
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();
        app.world_mut()
            .resource_mut::<NextState<Mode>>()
            .set(Mode::Combat);
        app.update();

        let rendered = &app.world().resource::<RenderedInitiative>().0;
        assert_eq!(rendered.entries.len(), 2);
        assert!(matches!(
            rendered.entries.as_slice(),
            [ally_entry, hostile_entry]
                if ally_entry.unit == ally
                    && ally_entry.name == "mage #4"
                    && hostile_entry.unit == hostile
                    && hostile_entry.name == "Unobserved hostile"
                    && !hostile_entry.inspectable
        ));

        app.world_mut()
            .insert_resource(observed_hostile_knowledge(hostile));
        app.update();
        let rendered = &app.world().resource::<RenderedInitiative>().0;
        assert!(rendered.entries.iter().any(|entry| {
            entry.unit == hostile && entry.name == "wolf #9" && entry.inspectable
        }));

        app.world_mut().remove_resource::<FactionMapKnowledge>();
        app.update();
        let rendered = &app.world().resource::<RenderedInitiative>().0;
        assert!(rendered.entries.iter().any(|entry| {
            entry.unit == hostile && entry.name == "Unobserved hostile" && !entry.inspectable
        }));
    }
}
