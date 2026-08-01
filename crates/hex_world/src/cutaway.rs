//! Composable review cutaways and adaptive tree fading.
//!
//! `hex_map` projects exact authored roof voxels onto disposable rendered runs as
//! [`CutawayOccluder`] components and publishes exact tree roots. `hex_objects`
//! propagates each root as a [`TreeOccluder`] on every rendered tree chunk. Ordinary
//! gameplay keeps cave roofs intact; explicit review tooling alone may expose a
//! complete interior.

use std::collections::{BTreeMap, BTreeSet};

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::VisibilitySystems;
use bevy::light::NotShadowCaster;
use bevy::picking::Pickable;
use bevy::prelude::*;
#[cfg(test)]
use bevy::transform::TransformPlugin;
use bevy::transform::TransformSystems;

use hex_core::{
    CameraFocusTarget, CutawayOccluder, InteriorRegionId, InteriorRegions, PresentationOcclusion,
    PresentationOcclusionReason, PresentationSystems, Screen, TreeFadeAmount, TreeOccluder,
};

use crate::camera::{CameraMode, PanOrbitCamera};

/// Small conservative margin around each transformed render-chunk bound.
const TREE_INTERSECTION_PADDING: f32 = 0.08;
/// Lowest opacity used while a tree blocks the close camera.
const TREE_FADED_OPACITY: f32 = 0.2;
/// Time used to ease a newly obstructing tree out of the view.
const TREE_FADE_IN_SECONDS: f32 = 0.12;
/// Time a cleared tree remains faded before restoring.
const TREE_FADE_HOLD_SECONDS: f32 = 0.2;
/// Time used to restore a fully faded tree.
const TREE_FADE_RESTORE_SECONDS: f32 = 0.3;

/// Marker installed only by deterministic capture tooling to expose a whole interior.
#[derive(Resource, Debug, Default)]
struct FullCutawayReviewOverride;

#[derive(Debug, Clone, Copy)]
struct TreeFadeTimeline {
    amount: f32,
    clear_seconds: f32,
}

#[derive(Resource, Debug, Default)]
struct TreeFadeTimelines {
    roots: BTreeMap<hex_core::TilePos, TreeFadeTimeline>,
}

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

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<TreeFadeTimelines>();
    app.configure_sets(
        PostUpdate,
        (
            PresentationSystems::ResolveCameraOcclusion,
            PresentationSystems::ApplyMaterials,
            PresentationSystems::ApplyVisibility,
        )
            .chain()
            .after(TransformSystems::Propagate)
            .after(VisibilitySystems::CalculateBounds)
            .before(VisibilitySystems::VisibilityPropagate),
    )
    .add_systems(
        PostUpdate,
        (reconcile_interior_cutaway, reconcile_tree_fades)
            .chain()
            .in_set(PresentationSystems::ResolveCameraOcclusion)
            .run_if(in_state(Screen::Gameplay)),
    )
    .add_systems(
        PostUpdate,
        apply_presentation_occlusion
            .in_set(PresentationSystems::ApplyVisibility)
            .run_if(in_state(Screen::Gameplay)),
    )
    .add_systems(OnExit(Screen::Gameplay), clear_tree_fade_timelines);
}

pub(super) fn install_full_review_override(app: &mut App) {
    app.init_resource::<FullCutawayReviewOverride>();
}

/// Owns only the explicit review-cutaway reason on exact projected roof runs.
fn reconcile_interior_cutaway(
    interiors: Option<Res<InteriorRegions>>,
    full_review_override: Option<Res<FullCutawayReviewOverride>>,
    targets: Query<&CameraFocusTarget>,
    mut candidates: Query<(Option<&CutawayOccluder>, &mut PresentationOcclusion)>,
) {
    let active = full_review_override
        .is_some()
        .then(|| active_cutaway(interiors.as_deref(), &targets))
        .flatten();

    for (occluder, occlusion) in &mut candidates {
        let should_hide =
            active.is_some_and(|region| occluder.is_some_and(|occluder| occluder.0 == region));
        set_reason(
            occlusion,
            PresentationOcclusionReason::InteriorCutaway,
            should_hide,
        );
    }
}

/// Fades a complete exact tree when any of its chunks blocks the focus corridor.
fn reconcile_tree_fades(
    mode: Res<CameraMode>,
    targets: Query<&CameraFocusTarget>,
    cameras: Query<(&PanOrbitCamera, &GlobalTransform), Without<CameraFocusTarget>>,
    time: Res<Time>,
    mut timelines: ResMut<TreeFadeTimelines>,
    mut trees: ParamSet<(
        Query<(
            &TreeOccluder,
            &GlobalTransform,
            Option<&Aabb>,
            &TreeFadeAmount,
        )>,
        Query<(&TreeOccluder, &mut TreeFadeAmount)>,
    )>,
    mut reported_invalid_cardinality: Local<Option<(usize, usize)>>,
) {
    let corridor =
        if *mode != CameraMode::Character {
            *reported_invalid_cardinality = None;
            None
        } else {
            let cardinality = (targets.iter().count(), cameras.iter().count());
            if cardinality == (1, 1) {
                *reported_invalid_cardinality = None;
                targets.single().ok().zip(cameras.single().ok()).map(
                    |(target, (camera, transform))| {
                        (target.surface, transform.translation(), camera.focus)
                    },
                )
            } else {
                if reported_invalid_cardinality.as_ref() != Some(&cardinality) {
                    warn!(
                    "tree fading requires exactly one focus target and one orbit camera; found \
                     {} targets and {} cameras",
                    cardinality.0, cardinality.1
                );
                    *reported_invalid_cardinality = Some(cardinality);
                }
                None
            }
        };

    if corridor.is_none() && timelines.roots.is_empty() {
        return;
    }

    let mut present = BTreeSet::new();
    let mut blocked = BTreeSet::new();
    for (tree, transform, bounds, _fade) in trees.p0().iter() {
        present.insert(tree.0);
        if corridor.is_some_and(|(_target, start, end)| {
            tree_chunk_intersects_focus_segment(start, end, transform, bounds)
        }) {
            blocked.insert(tree.0);
        }
    }

    timelines.roots.retain(|root, _| present.contains(root));
    for root in &blocked {
        timelines.roots.entry(*root).or_insert(TreeFadeTimeline {
            amount: 1.0,
            clear_seconds: 0.0,
        });
    }

    let delta = time.delta_secs().max(0.0);
    let mut completed = Vec::new();
    for (root, timeline) in &mut timelines.roots {
        advance_tree_fade_timeline(timeline, blocked.contains(root), delta);
        if !blocked.contains(root) && timeline.amount >= 1.0 {
            completed.push(*root);
        }
    }

    for (tree, mut fade) in trees.p1().iter_mut() {
        let amount = timelines
            .roots
            .get(&tree.0)
            .map_or(1.0, |state| state.amount);
        let Some(wanted) = TreeFadeAmount::new(amount) else {
            continue;
        };
        if (fade.amount() - wanted.amount()).abs() > f32::EPSILON {
            *fade = wanted;
        }
    }
    for root in completed {
        timelines.roots.remove(&root);
    }
}

fn advance_tree_fade_timeline(timeline: &mut TreeFadeTimeline, blocked: bool, delta: f32) {
    if blocked {
        timeline.clear_seconds = 0.0;
        let fade_rate = (1.0 - TREE_FADED_OPACITY) / TREE_FADE_IN_SECONDS;
        timeline.amount = (timeline.amount - fade_rate * delta).max(TREE_FADED_OPACITY);
        return;
    }

    let remaining_hold = (TREE_FADE_HOLD_SECONDS - timeline.clear_seconds).max(0.0);
    let hold_delta = delta.min(remaining_hold);
    timeline.clear_seconds += hold_delta;
    let restore_delta = delta - hold_delta;
    if restore_delta > 0.0 {
        let restore_rate = (1.0 - TREE_FADED_OPACITY) / TREE_FADE_RESTORE_SECONDS;
        timeline.amount = (timeline.amount + restore_rate * restore_delta).min(1.0);
    }
}

fn active_cutaway(
    interiors: Option<&InteriorRegions>,
    targets: &Query<&CameraFocusTarget>,
) -> Option<InteriorRegionId> {
    let interiors = interiors?;
    let target = targets.single().ok()?;
    interiors.get(target.surface)
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

fn tree_chunk_intersects_focus_segment(
    start: Vec3,
    end: Vec3,
    transform: &GlobalTransform,
    bounds: Option<&Aabb>,
) -> bool {
    if !start.is_finite() || !end.is_finite() {
        return false;
    }
    let (mut minimum, mut maximum) = transformed_world_bounds(transform, bounds);
    minimum -= Vec3::splat(TREE_INTERSECTION_PADDING);
    maximum += Vec3::splat(TREE_INTERSECTION_PADDING);
    let direction = end - start;
    let mut enter: f32 = 0.0;
    let mut exit: f32 = 1.0;
    for (origin, delta, low, high) in [
        (start.x, direction.x, minimum.x, maximum.x),
        (start.y, direction.y, minimum.y, maximum.y),
        (start.z, direction.z, minimum.z, maximum.z),
    ] {
        let Some((axis_enter, axis_exit)) = segment_axis_interval(origin, delta, low, high) else {
            return false;
        };
        enter = enter.max(axis_enter);
        exit = exit.min(axis_exit);
    }
    enter <= exit
}

fn transformed_world_bounds(transform: &GlobalTransform, bounds: Option<&Aabb>) -> (Vec3, Vec3) {
    let Some(bounds) = bounds else {
        let half = Vec3::splat(transform.scale().abs().max_element());
        return (
            transform.translation() - half,
            transform.translation() + half,
        );
    };
    let centre: Vec3 = bounds.center.into();
    let half: Vec3 = bounds.half_extents.into();
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);
    for x in [-1.0, 1.0] {
        for y in [-1.0, 1.0] {
            for z in [-1.0, 1.0] {
                let point = transform.transform_point(centre + half * Vec3::new(x, y, z));
                minimum = minimum.min(point);
                maximum = maximum.max(point);
            }
        }
    }
    (minimum, maximum)
}

fn segment_axis_interval(
    origin: f32,
    direction: f32,
    minimum: f32,
    maximum: f32,
) -> Option<(f32, f32)> {
    if direction.abs() <= f32::EPSILON {
        return (minimum <= origin && origin <= maximum).then_some((0.0, 1.0));
    }
    let first = (minimum - origin) / direction;
    let second = (maximum - origin) / direction;
    let enter = first.min(second).max(0.0);
    let exit = first.max(second).min(1.0);
    (enter <= exit).then_some((enter, exit))
}

fn clear_tree_fade_timelines(mut timelines: ResMut<TreeFadeTimelines>) {
    timelines.roots.clear();
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
    use std::time::{Duration, Instant};

    use super::*;
    use hex_core::{HexCoord, TilePos};
    use hex_test_app::HeadlessAppBuilder;

    #[derive(Resource, Default)]
    struct PresentationChangeCounts {
        fades: usize,
        occlusions: usize,
        visibilities: usize,
    }

    fn count_presentation_changes(
        fades: Query<Ref<TreeFadeAmount>>,
        candidates: Query<(Ref<PresentationOcclusion>, Ref<Visibility>)>,
        mut counts: ResMut<PresentationChangeCounts>,
    ) {
        counts.fades += fades.iter().filter(|fade| fade.is_changed()).count();
        for (occlusion, visibility) in &candidates {
            counts.occlusions += usize::from(occlusion.is_changed());
            counts.visibilities += usize::from(visibility.is_changed());
        }
    }

    fn position(x: i32, y: i32, level: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(x, y), level)
    }

    fn test_app(target: TilePos, region: InteriorRegionId) -> (App, Entity, Entity) {
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        builder.app_mut().add_plugins(TransformPlugin);
        builder
            .app_mut()
            .init_resource::<CameraMode>()
            .init_resource::<TreeFadeTimelines>()
            .add_systems(
                PostUpdate,
                (
                    reconcile_interior_cutaway,
                    reconcile_tree_fades,
                    apply_presentation_occlusion,
                )
                    .chain()
                    .after(TransformSystems::Propagate),
            );

        let mut interiors = InteriorRegions::new();
        interiors.insert_surface(target, region);
        builder.app_mut().insert_resource(interiors);

        let focus = target.coord.to_world(0.4);
        let target_entity = builder
            .app_mut()
            .world_mut()
            .spawn((
                Transform::from_translation(target.coord.to_world(0.0)),
                CameraFocusTarget::new(target),
            ))
            .id();
        let camera = builder
            .app_mut()
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 4.0, 7.0),
                PanOrbitCamera { focus, radius: 7.0 },
            ))
            .id();
        (builder.build(), target_entity, camera)
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
        let entity = app.world().entity(entity);
        assert_eq!(entity.get::<Visibility>(), Some(&Visibility::Hidden));
        assert_eq!(entity.get::<Pickable>(), Some(&Pickable::IGNORE));
        assert!(entity.contains::<NotShadowCaster>());
        assert!(entity.contains::<AppliedPresentationOcclusion>());
    }

    fn assert_ordinary(app: &App, entity: Entity) {
        let entity = app.world().entity(entity);
        assert_eq!(entity.get::<Visibility>(), Some(&Visibility::Inherited));
        assert!(!entity.contains::<Pickable>());
        assert!(!entity.contains::<NotShadowCaster>());
        assert!(!entity.contains::<AppliedPresentationOcclusion>());
    }

    #[test]
    fn ordinary_gameplay_keeps_every_cave_roof_in_map_and_character_modes() {
        let target = position(0, 0, 7);
        let region = InteriorRegionId(2);
        let (mut app, _, _) = test_app(target, region);
        let near = spawn_roof(&mut app, position(0, 0, 13), region);
        let distant = spawn_roof(&mut app, position(12, 0, 13), region);

        app.update();
        assert_ordinary(&app, near);
        assert_ordinary(&app, distant);

        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        app.update();
        assert_ordinary(&app, near);
        assert_ordinary(&app, distant);
    }

    #[test]
    fn full_review_override_hides_the_whole_exact_region_and_restores_it() {
        let target = position(0, 0, 7);
        let region = InteriorRegionId(2);
        let other_region = InteriorRegionId(9);
        let (mut app, _, _) = test_app(target, region);
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
        assert_ordinary(&app, near);
        assert_ordinary(&app, distant);
        assert_ordinary(&app, unrelated);
    }

    #[test]
    fn full_review_uses_the_exact_stacked_surface_region() {
        let lower_surface = position(0, 0, 6);
        let upper_surface = position(0, 0, 15);
        let lower_region = InteriorRegionId(3);
        let upper_region = InteriorRegionId(4);
        let (mut app, target, _) = test_app(lower_surface, lower_region);
        app.world_mut()
            .resource_mut::<InteriorRegions>()
            .insert_surface(upper_surface, upper_region);
        let lower_roof = spawn_roof(&mut app, position(0, 0, 10), lower_region);
        let upper_roof = spawn_roof(&mut app, position(0, 0, 20), upper_region);
        install_full_review_override(&mut app);

        app.update();
        assert_hidden(&app, lower_roof);
        assert_ordinary(&app, upper_roof);

        app.world_mut()
            .entity_mut(target)
            .get_mut::<CameraFocusTarget>()
            .expect("the test target should retain its exact focus surface")
            .surface = upper_surface;
        app.update();
        assert_ordinary(&app, lower_roof);
        assert_hidden(&app, upper_roof);
    }

    #[test]
    fn stable_review_cutaway_does_not_republish_state() {
        let target = position(0, 0, 7);
        let region = InteriorRegionId(2);
        let (mut app, _, _) = test_app(target, region);
        install_full_review_override(&mut app);
        app.init_resource::<PresentationChangeCounts>().add_systems(
            PostUpdate,
            count_presentation_changes.after(apply_presentation_occlusion),
        );
        let roof = spawn_roof(&mut app, position(0, 0, 13), region);

        app.update();
        assert_hidden(&app, roof);
        *app.world_mut().resource_mut::<PresentationChangeCounts>() =
            PresentationChangeCounts::default();

        for _ in 0..100 {
            app.update();
        }

        let counts = app.world().resource::<PresentationChangeCounts>();
        assert_eq!(counts.occlusions, 0);
        assert_eq!(counts.visibilities, 0);
    }

    fn spawn_tree_chunk(
        app: &mut App,
        root: TilePos,
        translation: Vec3,
        half_extents: Vec3,
    ) -> Entity {
        app.world_mut()
            .spawn((
                Transform::from_translation(translation),
                Aabb {
                    center: Vec3A::ZERO,
                    half_extents: half_extents.into(),
                },
                TreeOccluder(root),
                TreeFadeAmount::OPAQUE,
            ))
            .id()
    }

    fn fade_amount(app: &App, entity: Entity) -> f32 {
        app.world()
            .entity(entity)
            .get::<TreeFadeAmount>()
            .expect("tree chunks should retain a fade request")
            .amount()
    }

    #[test]
    fn one_blocking_chunk_fades_the_complete_exact_tree_only() {
        let target = position(0, 0, 7);
        let root = position(1, 0, 7);
        let other_root = position(1, 0, 17);
        let (mut app, _, _) = test_app(target, InteriorRegionId(1));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        let eye = Vec3::new(0.0, 4.0, 7.0);
        let focus = target.coord.to_world(0.4);
        let on_segment = eye.lerp(focus, 0.5);
        let blocking = spawn_tree_chunk(&mut app, root, on_segment, Vec3::splat(0.5));
        let same_tree_clear =
            spawn_tree_chunk(&mut app, root, on_segment + Vec3::X * 4.0, Vec3::splat(0.5));
        let stacked_other = spawn_tree_chunk(&mut app, other_root, on_segment, Vec3::splat(0.5));
        app.world_mut()
            .resource_mut::<TreeFadeTimelines>()
            .roots
            .insert(
                root,
                TreeFadeTimeline {
                    amount: TREE_FADED_OPACITY,
                    clear_seconds: 0.0,
                },
            );

        app.update();

        assert!((fade_amount(&app, blocking) - TREE_FADED_OPACITY).abs() < f32::EPSILON);
        assert!((fade_amount(&app, same_tree_clear) - TREE_FADED_OPACITY).abs() < f32::EPSILON);
        assert!((fade_amount(&app, stacked_other) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_distant_tree_on_a_long_zoom_corridor_is_not_culled_by_its_root_distance() {
        let target = position(0, 0, 7);
        let root = position(0, 8, 7);
        let (mut app, _, camera) = test_app(target, InteriorRegionId(1));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        let focus = target.coord.to_world(0.4);
        let tree_base = root.coord.to_world(0.4);
        let eye = tree_base * 2.0 + Vec3::Y * 4.0;
        app.world_mut()
            .entity_mut(camera)
            .insert(Transform::from_translation(eye));
        let tree = spawn_tree_chunk(&mut app, root, focus.lerp(eye, 0.5), Vec3::splat(0.5));

        app.update();

        assert!(
            app.world()
                .resource::<TreeFadeTimelines>()
                .roots
                .contains_key(&root),
            "every chunk on the actual corridor must participate at long zoom"
        );
        assert!((fade_amount(&app, tree) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn tree_fade_holds_then_restores_over_the_authored_duration() {
        let mut timeline = TreeFadeTimeline {
            amount: 1.0,
            clear_seconds: 0.0,
        };
        advance_tree_fade_timeline(&mut timeline, true, TREE_FADE_IN_SECONDS);
        assert!((timeline.amount - TREE_FADED_OPACITY).abs() < f32::EPSILON);

        advance_tree_fade_timeline(&mut timeline, false, TREE_FADE_HOLD_SECONDS);
        assert!((timeline.amount - TREE_FADED_OPACITY).abs() < f32::EPSILON);

        advance_tree_fade_timeline(&mut timeline, false, TREE_FADE_RESTORE_SECONDS * 0.5);
        assert!((timeline.amount - 0.6).abs() < 1e-5);
        advance_tree_fade_timeline(&mut timeline, false, TREE_FADE_RESTORE_SECONDS * 0.5);
        assert!((timeline.amount - 1.0).abs() < 1e-5);

        advance_tree_fade_timeline(&mut timeline, true, TREE_FADE_IN_SECONDS * 0.5);
        assert!((timeline.amount - 0.6).abs() < 1e-5);
        assert!(timeline.clear_seconds.abs() < f32::EPSILON);
    }

    #[test]
    fn stable_faded_tree_does_not_republish_chunk_requests() {
        let target = position(0, 0, 7);
        let root = position(1, 0, 7);
        let (mut app, _, _) = test_app(target, InteriorRegionId(1));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        let eye = Vec3::new(0.0, 4.0, 7.0);
        let focus = target.coord.to_world(0.4);
        let tree = spawn_tree_chunk(&mut app, root, eye.lerp(focus, 0.5), Vec3::splat(0.5));
        app.world_mut()
            .entity_mut(tree)
            .insert(TreeFadeAmount::new(TREE_FADED_OPACITY).expect("valid opacity"));
        app.world_mut()
            .resource_mut::<TreeFadeTimelines>()
            .roots
            .insert(
                root,
                TreeFadeTimeline {
                    amount: TREE_FADED_OPACITY,
                    clear_seconds: 0.0,
                },
            );
        app.init_resource::<PresentationChangeCounts>().add_systems(
            PostUpdate,
            count_presentation_changes.after(reconcile_tree_fades),
        );

        app.update();
        *app.world_mut().resource_mut::<PresentationChangeCounts>() =
            PresentationChangeCounts::default();
        for _ in 0..10_000 {
            app.update();
        }

        assert_eq!(
            app.world().resource::<PresentationChangeCounts>().fades,
            0,
            "a stable obstruction must not mark fade requests changed"
        );
    }

    #[test]
    #[ignore = "manual release-mode 2,048-chunk Character tree-fade timing diagnostic"]
    fn production_scale_tree_fade_release_timing() {
        let target = position(0, 0, 7);
        let (mut app, _, _) = test_app(target, InteriorRegionId(1));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        for index in 0..2_048 {
            let root = position(index, 20, 7);
            let translation = root.coord.to_world(8.0) + Vec3::Z * 40.0;
            let _chunk = spawn_tree_chunk(&mut app, root, translation, Vec3::splat(0.5));
        }
        app.update();

        let mut timings = Vec::with_capacity(200);
        for _ in 0..200 {
            let started = Instant::now();
            app.update();
            timings.push(started.elapsed());
        }
        timings.sort_unstable();
        let p95 = timings.get(190).copied().unwrap_or_default();
        let worst = timings.last().copied().unwrap_or_default();
        eprintln!(
            "2,048-chunk Character tree-fade diagnostic (release): p95={p95:?}, worst={worst:?}"
        );
        assert!(
            p95 < Duration::from_millis(1),
            "tree-fade reconciliation p95 {p95:?} breached the 1 ms release budget"
        );
    }

    #[test]
    fn transformed_chunk_bounds_drive_corridor_intersection() {
        let start = Vec3::ZERO;
        let end = Vec3::Z * 4.0;
        let on_segment = GlobalTransform::from(
            Transform::from_translation(Vec3::new(0.0, 0.0, 2.0))
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_3))
                .with_scale(Vec3::new(1.5, 0.75, 1.0)),
        );
        let off_segment = GlobalTransform::from(
            Transform::from_translation(Vec3::new(2.0, 0.0, 2.0))
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_3)),
        );
        let bounds = Aabb {
            center: Vec3A::ZERO,
            half_extents: Vec3A::splat(0.25),
        };

        assert!(tree_chunk_intersects_focus_segment(
            start,
            end,
            &on_segment,
            Some(&bounds)
        ));
        assert!(!tree_chunk_intersects_focus_segment(
            start,
            end,
            &off_segment,
            Some(&bounds)
        ));
    }

    #[test]
    fn fog_reason_remains_independent_from_review_cutaway() {
        let target = position(0, 0, 7);
        let region = InteriorRegionId(8);
        let (mut app, _, _) = test_app(target, region);
        install_full_review_override(&mut app);
        let roof = spawn_roof(&mut app, position(0, 0, 13), region);
        app.world_mut()
            .entity_mut(roof)
            .get_mut::<PresentationOcclusion>()
            .expect("the roof should participate in occlusion")
            .insert(PresentationOcclusionReason::Fog);

        app.update();
        assert_hidden(&app, roof);

        app.world_mut()
            .remove_resource::<FullCutawayReviewOverride>();
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
    fn removing_the_reason_set_restores_the_exact_previous_state() {
        let target = position(0, 0, 7);
        let (mut app, _, _) = test_app(target, InteriorRegionId(1));
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
}
