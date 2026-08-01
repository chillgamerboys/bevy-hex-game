//! Composable local cutaways for generated interiors and tree canopies.
//!
//! `hex_map` projects exact authored roof voxels onto disposable rendered runs as
//! [`CutawayOccluder`] components and obstructing tree parts as
//! [`CanopyOccluder`] components.
//! Each cutaway owns only its [`PresentationOcclusionReason`]; one adapter applies the
//! combined result to visibility, picking, and shadows without changing map semantics.

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::VisibilitySystems;
use bevy::light::NotShadowCaster;
use bevy::picking::Pickable;
use bevy::prelude::*;
#[cfg(test)]
use bevy::transform::TransformPlugin;
use bevy::transform::TransformSystems;

use hex_core::{
    CameraFocusTarget, CanopyOccluder, CutawayOccluder, HexCoord, InteriorRegionId,
    InteriorRegions, PresentationOcclusion, PresentationOcclusionReason, Screen, TilePos,
};

use crate::camera::{CameraMode, PanOrbitCamera};

/// Horizontal radius of the local roof opening, measured in hexes.
const CUTAWAY_RADIUS_HEXES: u32 = 6;
/// Canopies outside this selected-character neighbourhood never join the cutaway.
const CANOPY_CUTAWAY_RADIUS_HEXES: u32 = 4;
/// Small visual margin around the transformed canopy's unit-sphere bound.
const CANOPY_INTERSECTION_PADDING: f32 = 0.08;

/// Marker installed only by deterministic capture tooling to expose a whole interior.
#[derive(Resource, Debug, Default)]
struct FullCutawayReviewOverride;

/// The exact presentation state to restore when the final occlusion reason clears.
#[derive(Component, Debug, Clone, Copy)]
struct AppliedPresentationOcclusion {
    visibility: Visibility,
    pickable: Option<Pickable>,
    had_not_shadow_caster: bool,
}

type OcclusionCandidates = Or<(
    With<PresentationOcclusion>,
    With<AppliedPresentationOcclusion>,
)>;
type OcclusionCandidateQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Visibility,
        Option<&'static Pickable>,
        Has<NotShadowCaster>,
        Option<&'static PresentationOcclusion>,
        Option<&'static AppliedPresentationOcclusion>,
    ),
    OcclusionCandidates,
>;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CutawaySystems {
    ResolveReasons,
    ApplyPresentation,
}

pub(super) fn plugin(app: &mut App) {
    app.configure_sets(
        PostUpdate,
        (
            CutawaySystems::ResolveReasons,
            CutawaySystems::ApplyPresentation,
        )
            .chain()
            .after(TransformSystems::Propagate)
            .before(VisibilitySystems::VisibilityPropagate),
    )
    .add_systems(
        PostUpdate,
        (reconcile_interior_cutaway, reconcile_canopy_cutaway)
            .chain()
            .in_set(CutawaySystems::ResolveReasons)
            .run_if(in_state(Screen::Gameplay)),
    )
    .add_systems(
        PostUpdate,
        apply_presentation_occlusion
            .in_set(CutawaySystems::ApplyPresentation)
            .run_if(in_state(Screen::Gameplay)),
    );
}

pub(super) fn install_full_review_override(app: &mut App) {
    app.init_resource::<FullCutawayReviewOverride>();
}

/// Owns only the interior-cutaway reason on exact projected roof runs.
fn reconcile_interior_cutaway(
    interiors: Option<Res<InteriorRegions>>,
    full_review_override: Option<Res<FullCutawayReviewOverride>>,
    targets: Query<(&CameraFocusTarget, &GlobalTransform)>,
    mut candidates: Query<(
        &TilePos,
        Option<&CutawayOccluder>,
        &mut PresentationOcclusion,
    )>,
) {
    let active = active_cutaway(interiors.as_deref(), &targets);

    for (position, occluder, occlusion) in &mut candidates {
        let should_hide = active.is_some_and(|(region, centre)| {
            occluder.is_some_and(|occluder| occluder.0 == region)
                && (full_review_override.is_some()
                    || position.coord.distance(centre) <= CUTAWAY_RADIUS_HEXES)
        });
        set_reason(
            occlusion,
            PresentationOcclusionReason::InteriorCutaway,
            should_hide,
        );
    }
}

/// Owns only the character-camera reason on nearby tree parts intersecting its focus ray.
fn reconcile_canopy_cutaway(
    mode: Res<CameraMode>,
    targets: Query<&CameraFocusTarget>,
    cameras: Query<(&PanOrbitCamera, &GlobalTransform), Without<CameraFocusTarget>>,
    mut canopies: Query<(
        &CanopyOccluder,
        &GlobalTransform,
        Option<&Aabb>,
        &mut PresentationOcclusion,
    )>,
    mut reported_invalid_cardinality: Local<Option<(usize, usize)>>,
) {
    let active = if *mode != CameraMode::Character {
        *reported_invalid_cardinality = None;
        None
    } else {
        let cardinality = (targets.iter().count(), cameras.iter().count());
        if cardinality == (1, 1) {
            *reported_invalid_cardinality = None;
            targets.single().ok().zip(cameras.single().ok())
        } else {
            if reported_invalid_cardinality.as_ref() != Some(&cardinality) {
                warn!(
                    "canopy cutaway requires exactly one focus target and one orbit camera; found \
                     {} targets and {} cameras",
                    cardinality.0, cardinality.1
                );
                *reported_invalid_cardinality = Some(cardinality);
            }
            None
        }
    };

    for (canopy, transform, bounds, occlusion) in &mut canopies {
        let should_hide = active.is_some_and(|(target, (camera, camera_transform))| {
            let (centre, radius) = canopy_world_sphere(transform, bounds);
            canopy.0.coord.distance(target.surface.coord) <= CANOPY_CUTAWAY_RADIUS_HEXES
                && canopy_intersects_focus_segment(
                    camera_transform.translation(),
                    camera.focus,
                    centre,
                    radius,
                )
        });
        set_reason(
            occlusion,
            PresentationOcclusionReason::CanopyCutaway,
            should_hide,
        );
    }
}

fn canopy_world_sphere(transform: &GlobalTransform, bounds: Option<&Aabb>) -> (Vec3, f32) {
    bounds.map_or_else(
        || {
            (
                transform.translation(),
                transform.scale().abs().max_element() + CANOPY_INTERSECTION_PADDING,
            )
        },
        |bounds| {
            let centre = transform.transform_point(bounds.center.into());
            let radius = bounds.half_extents.length() * transform.scale().abs().max_element()
                + CANOPY_INTERSECTION_PADDING;
            (centre, radius)
        },
    )
}

fn active_cutaway(
    interiors: Option<&InteriorRegions>,
    targets: &Query<(&CameraFocusTarget, &GlobalTransform)>,
) -> Option<(InteriorRegionId, HexCoord)> {
    let interiors = interiors?;
    let (target, transform) = targets.single().ok()?;
    let region = interiors.get(target.surface)?;
    let centre = HexCoord::from_world(transform.translation());
    Some((region, centre))
}

fn set_reason(
    mut occlusion: Mut<'_, PresentationOcclusion>,
    reason: PresentationOcclusionReason,
    active: bool,
) {
    if occlusion.contains(reason) == active {
        return;
    }
    if active {
        occlusion.insert(reason);
    } else {
        occlusion.remove(reason);
    }
}

fn canopy_intersects_focus_segment(start: Vec3, end: Vec3, centre: Vec3, radius: f32) -> bool {
    if !start.is_finite()
        || !end.is_finite()
        || !centre.is_finite()
        || !radius.is_finite()
        || radius <= 0.0
    {
        return false;
    }

    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f32::EPSILON {
        return start.distance_squared(centre) <= radius * radius;
    }
    let amount = ((centre - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    let nearest = start + segment * amount;
    nearest.distance_squared(centre) <= radius * radius
}

/// Applies the combined reason set and is the sole owner of concrete presentation state.
fn apply_presentation_occlusion(mut commands: Commands, mut candidates: OcclusionCandidateQuery) {
    for (entity, mut visibility, pickable, no_shadow, occlusion, applied) in &mut candidates {
        let should_hide = occlusion.is_some_and(|occlusion| occlusion.is_hidden());
        match (should_hide, applied) {
            (true, None) => {
                let previous = AppliedPresentationOcclusion {
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
                if *visibility != Visibility::Hidden {
                    *visibility = Visibility::Hidden;
                }
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
                entity.remove::<AppliedPresentationOcclusion>();
            }
            (false, None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_test_app::HeadlessAppBuilder;

    #[derive(Resource, Default)]
    struct CutawayChangeCounts {
        occlusions: usize,
        visibilities: usize,
    }

    fn count_cutaway_changes(
        candidates: Query<(Ref<PresentationOcclusion>, Ref<Visibility>)>,
        mut counts: ResMut<CutawayChangeCounts>,
    ) {
        for (occlusion, visibility) in &candidates {
            counts.occlusions += usize::from(occlusion.is_changed());
            counts.visibilities += usize::from(visibility.is_changed());
        }
    }

    fn position(x: i32, y: i32, level: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(x, y), level)
    }

    fn test_app(target: TilePos, region: InteriorRegionId) -> (App, Entity) {
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        builder.app_mut().add_plugins(TransformPlugin);
        builder.app_mut().init_resource::<CameraMode>();
        builder.app_mut().add_systems(
            PostUpdate,
            (
                reconcile_interior_cutaway,
                reconcile_canopy_cutaway,
                apply_presentation_occlusion,
            )
                .chain()
                .after(TransformSystems::Propagate),
        );

        let mut interiors = InteriorRegions::new();
        interiors.insert_surface(target, region);
        builder.app_mut().insert_resource(interiors);

        let target_entity = builder
            .app_mut()
            .world_mut()
            .spawn((
                Transform::from_translation(target.coord.to_world(0.0)),
                CameraFocusTarget::new(target),
            ))
            .id();
        builder.app_mut().world_mut().spawn((
            Transform::from_xyz(0.0, 4.0, 7.0),
            PanOrbitCamera {
                focus: target.coord.to_world(0.4),
                radius: 7.0,
            },
        ));
        (builder.build(), target_entity)
    }

    fn spawn_roof(app: &mut App, position: TilePos, region: InteriorRegionId) -> Entity {
        app.world_mut()
            .spawn((
                position,
                CutawayOccluder(region),
                PresentationOcclusion::default(),
                Visibility::Inherited,
            ))
            .id()
    }

    fn assert_hidden(app: &App, entity: Entity) {
        let roof = app.world().entity(entity);
        assert_eq!(roof.get::<Visibility>(), Some(&Visibility::Hidden));
        assert_eq!(roof.get::<Pickable>(), Some(&Pickable::IGNORE));
        assert!(roof.contains::<NotShadowCaster>());
        assert!(roof.contains::<AppliedPresentationOcclusion>());
    }

    fn assert_ordinary(app: &App, entity: Entity) {
        let roof = app.world().entity(entity);
        assert_eq!(roof.get::<Visibility>(), Some(&Visibility::Inherited));
        assert!(!roof.contains::<Pickable>());
        assert!(!roof.contains::<NotShadowCaster>());
        assert!(!roof.contains::<AppliedPresentationOcclusion>());
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
    fn one_hundred_idle_frames_do_not_republish_cutaway_state() {
        let target = position(0, 0, 7);
        let region = InteriorRegionId(2);
        let (mut app, _) = test_app(target, region);
        app.init_resource::<CutawayChangeCounts>().add_systems(
            PostUpdate,
            count_cutaway_changes.after(apply_presentation_occlusion),
        );
        let roof = spawn_roof(&mut app, position(0, 0, 13), region);

        app.update();
        assert_hidden(&app, roof);
        *app.world_mut().resource_mut::<CutawayChangeCounts>() = CutawayChangeCounts::default();

        for _ in 0..100 {
            app.update();
        }

        assert_hidden(&app, roof);
        let counts = app.world().resource::<CutawayChangeCounts>();
        assert_eq!(counts.occlusions, 0);
        assert_eq!(counts.visibilities, 0);
    }

    #[test]
    fn full_review_override_hides_the_whole_active_region_and_restores_normally() {
        let target = position(0, 0, 7);
        let region = InteriorRegionId(2);
        let other_region = InteriorRegionId(9);
        let (mut app, _) = test_app(target, region);
        let near = spawn_roof(&mut app, position(1, 0, 13), region);
        let distant = spawn_roof(&mut app, position(12, 0, 13), region);
        let unrelated = spawn_roof(&mut app, position(12, 0, 13), other_region);
        install_full_review_override(&mut app);

        app.update();

        assert_hidden(&app, near);
        assert_hidden(&app, distant);
        assert_ordinary(&app, unrelated);

        app.world_mut()
            .remove_resource::<FullCutawayReviewOverride>();
        app.update();

        assert_hidden(&app, near);
        assert_ordinary(&app, distant);
        assert_ordinary(&app, unrelated);
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
                PresentationOcclusion::default(),
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
        assert!(!restored.contains::<AppliedPresentationOcclusion>());
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

    fn spawn_canopy(app: &mut App, root: TilePos, translation: Vec3, radius: f32) -> Entity {
        app.world_mut()
            .spawn((
                Transform::from_translation(translation).with_scale(Vec3::splat(radius)),
                Visibility::Inherited,
                Pickable::IGNORE,
                PresentationOcclusion::default(),
                CanopyOccluder(root),
            ))
            .id()
    }

    fn assert_canopy_ordinary(app: &App, entity: Entity) {
        let canopy = app.world().entity(entity);
        assert_eq!(
            canopy.get::<Visibility>(),
            Some(&Visibility::Inherited),
            "canopy transform={:?}, occlusion={:?}",
            canopy.get::<GlobalTransform>(),
            canopy.get::<PresentationOcclusion>()
        );
        assert_eq!(canopy.get::<Pickable>(), Some(&Pickable::IGNORE));
        assert!(!canopy.contains::<NotShadowCaster>());
        assert!(!canopy.contains::<AppliedPresentationOcclusion>());
    }

    #[test]
    fn character_camera_hides_only_near_canopies_intersecting_its_focus_segment() {
        let target = position(0, 0, 7);
        let (mut app, _) = test_app(target, InteriorRegionId(1));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        let eye = Vec3::new(0.0, 4.0, 7.0);
        let focus = target.coord.to_world(0.4);
        let on_segment = eye.lerp(focus, 0.5);

        let obstructing = spawn_canopy(&mut app, position(1, 0, 7), on_segment, 0.9);
        let clear = spawn_canopy(
            &mut app,
            position(1, -1, 7),
            on_segment + Vec3::X * 2.0,
            0.9,
        );
        let distant = spawn_canopy(&mut app, position(5, 0, 7), on_segment, 0.9);

        app.update();

        assert_hidden(&app, obstructing);
        assert_canopy_ordinary(&app, clear);
        assert_canopy_ordinary(&app, distant);

        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Map;
        app.update();

        assert_canopy_ordinary(&app, obstructing);
    }

    #[test]
    fn character_camera_uses_the_complete_authored_canopy_bounds() {
        let target = position(0, 0, 7);
        let (mut app, _) = test_app(target, InteriorRegionId(1));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        let eye = Vec3::new(0.0, 4.0, 7.0);
        let focus = target.coord.to_world(0.4);
        let on_segment = eye.lerp(focus, 0.5);
        let translation = on_segment + Vec3::X * 4.0;
        let canopy = spawn_canopy(&mut app, position(1, 0, 7), translation, 1.0);
        app.world_mut().entity_mut(canopy).insert(Aabb {
            center: (-Vec3::X * 4.0).into(),
            half_extents: Vec3A::splat(0.5),
        });

        app.update();

        assert_hidden(&app, canopy);
    }

    #[test]
    fn canopy_cutaway_tracks_camera_orbit_and_selected_surface() {
        let target = position(0, 0, 7);
        let (mut app, target_entity) = test_app(target, InteriorRegionId(1));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        let canopy = spawn_canopy(&mut app, position(1, 0, 7), Vec3::new(0.0, 2.2, 3.5), 1.0);

        app.update();
        assert_hidden(&app, canopy);

        let camera = app
            .world_mut()
            .query_filtered::<Entity, With<PanOrbitCamera>>()
            .single(app.world())
            .expect("the test camera should exist");
        app.world_mut()
            .entity_mut(camera)
            .get_mut::<Transform>()
            .expect("the camera should have a transform")
            .translation = Vec3::new(8.0, 4.0, 7.0);
        app.update();
        assert_canopy_ordinary(&app, canopy);

        app.world_mut()
            .entity_mut(target_entity)
            .get_mut::<CameraFocusTarget>()
            .expect("the selected target should have an exact surface")
            .surface = position(9, 0, 7);
        app.update();
        assert_canopy_ordinary(&app, canopy);
    }

    #[test]
    fn ambiguous_camera_wiring_disables_and_then_recovers_canopy_cutaway() {
        let target = position(0, 0, 7);
        let (mut app, _) = test_app(target, InteriorRegionId(1));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        let canopy = spawn_canopy(&mut app, position(1, 0, 7), Vec3::new(0.0, 2.2, 3.5), 1.0);

        app.update();
        assert_hidden(&app, canopy);

        let duplicate = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 4.0, 7.0),
                PanOrbitCamera::default(),
            ))
            .id();
        app.update();
        assert_canopy_ordinary(&app, canopy);

        app.world_mut().despawn(duplicate);
        app.update();
        assert_hidden(&app, canopy);
    }

    #[test]
    fn fog_keeps_roof_hidden_after_the_interior_reason_clears() {
        let target = position(0, 0, 7);
        let region = InteriorRegionId(8);
        let (mut app, _) = test_app(target, region);
        let roof = spawn_roof(&mut app, position(0, 0, 13), region);
        app.world_mut()
            .entity_mut(roof)
            .get_mut::<PresentationOcclusion>()
            .expect("the roof should participate in occlusion")
            .insert(PresentationOcclusionReason::Fog);

        app.update();
        assert_hidden(&app, roof);

        app.world_mut().remove_resource::<InteriorRegions>();
        app.update();
        assert_hidden(&app, roof);
        let reasons = app
            .world()
            .entity(roof)
            .get::<PresentationOcclusion>()
            .expect("the roof should retain its reason set");
        assert!(reasons.contains(PresentationOcclusionReason::Fog));
        assert!(!reasons.contains(PresentationOcclusionReason::InteriorCutaway));

        app.world_mut()
            .entity_mut(roof)
            .get_mut::<PresentationOcclusion>()
            .expect("the roof should retain its reason set")
            .remove(PresentationOcclusionReason::Fog);
        app.update();
        assert_ordinary(&app, roof);
    }

    #[test]
    fn fog_keeps_canopy_hidden_after_character_mode_ends() {
        let target = position(0, 0, 7);
        let (mut app, _) = test_app(target, InteriorRegionId(1));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        let canopy = spawn_canopy(&mut app, position(1, 0, 7), Vec3::new(0.0, 2.2, 3.5), 1.0);
        app.world_mut()
            .entity_mut(canopy)
            .get_mut::<PresentationOcclusion>()
            .expect("the canopy should participate in occlusion")
            .insert(PresentationOcclusionReason::Fog);

        app.update();
        assert_hidden(&app, canopy);

        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Map;
        app.update();
        assert_hidden(&app, canopy);

        app.world_mut()
            .entity_mut(canopy)
            .get_mut::<PresentationOcclusion>()
            .expect("the canopy should retain its reason set")
            .remove(PresentationOcclusionReason::Fog);
        app.update();
        assert_canopy_ordinary(&app, canopy);
    }

    #[test]
    fn removing_the_reason_set_restores_the_exact_previous_state() {
        let target = position(0, 0, 7);
        let (mut app, _) = test_app(target, InteriorRegionId(1));
        let previous_pickable = Pickable {
            should_block_lower: false,
            is_hoverable: true,
        };
        let entity = app
            .world_mut()
            .spawn((
                Visibility::Visible,
                previous_pickable,
                NotShadowCaster,
                PresentationOcclusion::from_reason(PresentationOcclusionReason::Fog),
            ))
            .id();

        app.update();
        assert_hidden(&app, entity);

        app.world_mut()
            .entity_mut(entity)
            .remove::<PresentationOcclusion>();
        app.update();

        let restored = app.world().entity(entity);
        assert_eq!(restored.get::<Visibility>(), Some(&Visibility::Visible));
        assert_eq!(restored.get::<Pickable>(), Some(&previous_pickable));
        assert!(restored.contains::<NotShadowCaster>());
        assert!(!restored.contains::<AppliedPresentationOcclusion>());
    }

    #[test]
    fn focus_segment_intersection_rejects_invalid_or_off_segment_bounds() {
        let start = Vec3::ZERO;
        let end = Vec3::Z * 4.0;
        assert!(canopy_intersects_focus_segment(
            start,
            end,
            Vec3::Z * 2.0,
            0.5
        ));
        assert!(!canopy_intersects_focus_segment(
            start,
            end,
            Vec3::new(1.0, 0.0, 2.0),
            0.5
        ));
        assert!(!canopy_intersects_focus_segment(
            start,
            end,
            Vec3::Z * 2.0,
            f32::NAN
        ));
    }
}
