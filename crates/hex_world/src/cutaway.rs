//! Local opaque-roof cutaways for generated interiors.
//!
//! `hex_map` projects exact authored roof voxels onto disposable rendered runs as
//! [`CutawayOccluder`] components. This module owns only their presentation: while the
//! selected unit occupies an interior, nearby runs belonging to that same interior are
//! hidden without changing the voxel map or traversal surface graph.

use bevy::camera::visibility::VisibilitySystems;
use bevy::light::NotShadowCaster;
use bevy::picking::Pickable;
use bevy::prelude::*;

use hex_core::{
    CameraFocusTarget, CutawayOccluder, HexCoord, InteriorRegionId, InteriorRegions, Screen,
    TilePos,
};

/// Horizontal radius of the local roof opening, measured in hexes.
const CUTAWAY_RADIUS_HEXES: u32 = 6;

/// The exact presentation state to restore when a roof leaves the local cutaway.
#[derive(Component, Debug, Clone, Copy)]
struct CutawayHidden {
    visibility: Visibility,
    pickable: Option<Pickable>,
    had_not_shadow_caster: bool,
}

type CutawayCandidates = Or<(With<CutawayOccluder>, With<CutawayHidden>)>;
type CutawayCandidateQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static TilePos,
        Option<&'static CutawayOccluder>,
        &'static mut Visibility,
        Option<&'static Pickable>,
        Has<NotShadowCaster>,
        Option<&'static CutawayHidden>,
    ),
    CutawayCandidates,
>;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        PostUpdate,
        reconcile_cutaway
            .before(VisibilitySystems::VisibilityPropagate)
            .run_if(in_state(Screen::Gameplay)),
    );
}

/// Opens one local roof window around the selected actor and restores everything else.
fn reconcile_cutaway(
    mut commands: Commands,
    interiors: Option<Res<InteriorRegions>>,
    targets: Query<(&CameraFocusTarget, &Transform)>,
    mut candidates: CutawayCandidateQuery,
) {
    let active = active_cutaway(interiors.as_deref(), &targets);

    for (entity, position, occluder, mut visibility, pickable, no_shadow, hidden) in &mut candidates
    {
        let should_hide = active.is_some_and(|(region, centre)| {
            occluder.is_some_and(|occluder| occluder.0 == region)
                && position.coord.distance(centre) <= CUTAWAY_RADIUS_HEXES
        });

        match (should_hide, hidden) {
            (true, None) => {
                let previous = CutawayHidden {
                    visibility: *visibility,
                    pickable: pickable.copied(),
                    had_not_shadow_caster: no_shadow,
                };
                *visibility = Visibility::Hidden;
                commands
                    .entity(entity)
                    .insert((previous, Pickable::IGNORE, NotShadowCaster));
            }
            (true, Some(_)) => {
                *visibility = Visibility::Hidden;
                let mut entity = commands.entity(entity);
                if pickable.copied() != Some(Pickable::IGNORE) {
                    entity.insert(Pickable::IGNORE);
                }
                if !no_shadow {
                    entity.insert(NotShadowCaster);
                }
            }
            (false, Some(previous)) => {
                *visibility = previous.visibility;
                let mut entity = commands.entity(entity);
                if let Some(pickable) = previous.pickable {
                    entity.insert(pickable);
                } else {
                    entity.remove::<Pickable>();
                }
                if previous.had_not_shadow_caster {
                    entity.insert(NotShadowCaster);
                } else {
                    entity.remove::<NotShadowCaster>();
                }
                entity.remove::<CutawayHidden>();
            }
            (false, None) => {}
        }
    }
}

fn active_cutaway(
    interiors: Option<&InteriorRegions>,
    targets: &Query<(&CameraFocusTarget, &Transform)>,
) -> Option<(InteriorRegionId, HexCoord)> {
    let interiors = interiors?;
    let (target, transform) = targets.single().ok()?;
    let region = interiors.get(target.surface)?;
    let centre = HexCoord::from_world(transform.translation);
    Some((region, centre))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(x: i32, y: i32, level: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(x, y), level)
    }

    fn test_app(target: TilePos, region: InteriorRegionId) -> (App, Entity) {
        let mut app = App::new();
        app.add_systems(PostUpdate, reconcile_cutaway);

        let mut interiors = InteriorRegions::new();
        interiors.insert_surface(target, region);
        app.insert_resource(interiors);

        let target_entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(target.coord.to_world(0.0)),
                CameraFocusTarget::new(target),
            ))
            .id();
        (app, target_entity)
    }

    fn spawn_roof(app: &mut App, position: TilePos, region: InteriorRegionId) -> Entity {
        app.world_mut()
            .spawn((position, CutawayOccluder(region), Visibility::Inherited))
            .id()
    }

    fn assert_hidden(app: &App, entity: Entity) {
        let roof = app.world().entity(entity);
        assert_eq!(roof.get::<Visibility>(), Some(&Visibility::Hidden));
        assert_eq!(roof.get::<Pickable>(), Some(&Pickable::IGNORE));
        assert!(roof.contains::<NotShadowCaster>());
        assert!(roof.contains::<CutawayHidden>());
    }

    fn assert_ordinary(app: &App, entity: Entity) {
        let roof = app.world().entity(entity);
        assert_eq!(roof.get::<Visibility>(), Some(&Visibility::Inherited));
        assert!(!roof.contains::<Pickable>());
        assert!(!roof.contains::<NotShadowCaster>());
        assert!(!roof.contains::<CutawayHidden>());
    }

    #[test]
    fn hides_only_matching_roofs_inside_the_inclusive_radius() {
        let target = position(0, 0, 7);
        let region = InteriorRegionId(2);
        let other_region = InteriorRegionId(9);
        let (mut app, _) = test_app(target, region);
        let near = spawn_roof(&mut app, position(0, 0, 13), region);
        let boundary = spawn_roof(&mut app, position(6, 0, 13), region);
        let outside = spawn_roof(&mut app, position(7, 0, 13), region);
        let unrelated = spawn_roof(&mut app, position(1, 0, 13), other_region);

        app.update();

        assert_hidden(&app, near);
        assert_hidden(&app, boundary);
        assert_ordinary(&app, outside);
        assert_ordinary(&app, unrelated);

        app.world_mut().remove_resource::<InteriorRegions>();
        app.update();

        assert_ordinary(&app, near);
        assert_ordinary(&app, boundary);
    }

    #[test]
    fn exact_stacked_surface_selects_the_region_and_restores_the_old_one() {
        let lower_surface = position(0, 0, 6);
        let upper_surface = position(0, 0, 15);
        let lower_region = InteriorRegionId(3);
        let upper_region = InteriorRegionId(4);
        let (mut app, target) = test_app(lower_surface, lower_region);
        app.world_mut()
            .resource_mut::<InteriorRegions>()
            .insert_surface(upper_surface, upper_region);
        let lower_roof = spawn_roof(&mut app, position(0, 0, 10), lower_region);
        let upper_roof = spawn_roof(&mut app, position(0, 0, 20), upper_region);

        app.update();

        assert_hidden(&app, lower_roof);
        assert_ordinary(&app, upper_roof);

        app.world_mut()
            .entity_mut(target)
            .get_mut::<CameraFocusTarget>()
            .expect("the test target should have a focus projection")
            .surface = upper_surface;
        app.update();

        assert_ordinary(&app, lower_roof);
        assert_hidden(&app, upper_roof);
    }

    #[test]
    fn restoration_preserves_preexisting_visibility_picking_and_shadow_state() {
        let target = position(0, 0, 7);
        let region = InteriorRegionId(5);
        let (mut app, target_entity) = test_app(target, region);
        let previous_pickable = Pickable {
            should_block_lower: false,
            is_hoverable: true,
        };
        let roof = app
            .world_mut()
            .spawn((
                position(0, 0, 13),
                CutawayOccluder(region),
                Visibility::Visible,
                previous_pickable,
                NotShadowCaster,
            ))
            .id();

        app.update();
        assert_hidden(&app, roof);

        app.world_mut()
            .entity_mut(target_entity)
            .remove::<CameraFocusTarget>();
        app.update();

        let restored = app.world().entity(roof);
        assert_eq!(restored.get::<Visibility>(), Some(&Visibility::Visible));
        assert_eq!(restored.get::<Pickable>(), Some(&previous_pickable));
        assert!(restored.contains::<NotShadowCaster>());
        assert!(!restored.contains::<CutawayHidden>());
    }

    #[test]
    fn animated_transform_centres_the_window_while_surface_selects_the_region() {
        let target = position(0, 0, 7);
        let region = InteriorRegionId(6);
        let (mut app, target_entity) = test_app(target, region);
        let logical_surface_roof = spawn_roof(&mut app, position(0, 0, 13), region);
        let rendered_position_roof = spawn_roof(&mut app, position(7, 0, 13), region);
        app.world_mut()
            .entity_mut(target_entity)
            .get_mut::<Transform>()
            .expect("the test target should have a transform")
            .translation = position(7, 0, 7).coord.to_world(0.0);

        app.update();

        assert_ordinary(&app, logical_surface_roof);
        assert_hidden(&app, rendered_position_roof);

        app.world_mut()
            .spawn((Transform::default(), CameraFocusTarget::new(target)));
        app.update();

        assert_ordinary(&app, rendered_position_roof);
    }
}
