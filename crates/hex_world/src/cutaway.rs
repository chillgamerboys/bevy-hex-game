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

use hex_assets::CameraSettings;
use hex_core::{
    CameraFocusTarget, CutawayOccluder, InteriorRegionId, InteriorRegions, PresentationOcclusion,
    PresentationOcclusionReason, PresentationSystems, Screen, TreeFadeAmount, TreeOccluder,
};

use crate::camera::{CameraMode, PanOrbitCamera};

/// Small conservative margin around each transformed render-chunk bound in addition
/// to the camera's configured near-plane probe radius.
const TREE_BOUNDS_PADDING: f32 = 0.08;
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

/// Shares the authored 20% opacity across render chunks when several exact trees
/// intersect the camera-focus corridor. One tree retains the exact authored opacity
/// regardless of renderer chunking; a dense multi-tree screen cannot compound every
/// translucent foliage layer at 20% independently.
fn shared_tree_fade_opacity(blocked_roots: usize, blocking_chunks: usize) -> f32 {
    if blocked_roots <= 1 {
        return TREE_FADED_OPACITY;
    }
    let Ok(count) = u16::try_from(blocking_chunks.max(1)) else {
        return 0.0;
    };
    TREE_FADED_OPACITY / f32::from(count)
}

/// The exact presentation state to restore when the final occlusion reason clears.
#[derive(Component, Debug, Clone, Copy)]
struct AppliedPresentationOcclusion {
    visibility: Visibility,
    pickable: Option<Pickable>,
    had_not_shadow_caster: bool,
}

/// Identifies the exact reason owned by Character-camera proximity reconciliation.
///
/// Cleanup follows this marker rather than [`AppliedPresentationOcclusion`], which
/// may remain active for an unrelated owner such as fog.
#[derive(Component, Debug)]
struct CharacterCameraOcclusionOwner;

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
        (
            reconcile_interior_cutaway,
            reconcile_tree_fades,
            reconcile_character_proximity,
        )
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
    .add_systems(
        OnExit(Screen::Gameplay),
        (
            (clear_character_proximity, apply_presentation_occlusion).chain(),
            clear_tree_fade_timelines,
        ),
    );
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

type CharacterOcclusionCandidates =
    Or<(With<CameraFocusTarget>, With<CharacterCameraOcclusionOwner>)>;

/// Hides only the selected unit when a collision-limited camera would sit inside it.
///
/// The camera remains fully player-authored. This is presentation help for the
/// near-first-person result of a tight-space or unusual-angle radius retraction, not
/// an alternate camera angle. Exit hysteresis prevents a one-frame visibility toggle
/// around the authored threshold.
fn reconcile_character_proximity(
    mut commands: Commands,
    mode: Res<CameraMode>,
    settings: Res<CameraSettings>,
    cameras: Query<(&PanOrbitCamera, &GlobalTransform), Without<CameraFocusTarget>>,
    mut candidates: ParamSet<(
        Query<Entity, With<CameraFocusTarget>>,
        Query<
            (
                Entity,
                Has<CharacterCameraOcclusionOwner>,
                Option<&mut PresentationOcclusion>,
            ),
            CharacterOcclusionCandidates,
        >,
    )>,
) {
    let focused = {
        let targets = candidates.p0();
        targets.single().ok()
    };
    let effective_radius = (*mode == CameraMode::Character)
        .then(|| cameras.single().ok())
        .flatten()
        .map(|(camera, transform)| transform.translation().distance(camera.focus));
    let exit_hysteresis = settings.character_collision_margin * 0.25;

    for (entity, owned, occlusion) in &mut candidates.p1() {
        let threshold =
            settings.character_self_hide_radius + if owned { exit_hysteresis } else { 0.0 };
        let should_hide =
            Some(entity) == focused && effective_radius.is_some_and(|radius| radius <= threshold);

        match occlusion {
            Some(occlusion) => set_reason(
                occlusion,
                PresentationOcclusionReason::CharacterCameraProximity,
                should_hide,
            ),
            None if should_hide => {
                commands
                    .entity(entity)
                    .insert(PresentationOcclusion::from_reason(
                        PresentationOcclusionReason::CharacterCameraProximity,
                    ));
            }
            None => {}
        }

        if should_hide && !owned {
            commands
                .entity(entity)
                .insert(CharacterCameraOcclusionOwner);
        } else if !should_hide && owned {
            commands
                .entity(entity)
                .remove::<CharacterCameraOcclusionOwner>();
        }
    }
}

/// Fades a complete exact tree when any of its chunks blocks the focus corridor.
fn reconcile_tree_fades(
    mode: Res<CameraMode>,
    settings: Res<CameraSettings>,
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
    let mut blocking_chunks = 0_usize;
    for (tree, transform, bounds, _fade) in trees.p0().iter() {
        present.insert(tree.0);
        if corridor.is_some_and(|(_target, start, end)| {
            tree_chunk_intersects_focus_corridor(
                start,
                end,
                settings.character_probe_radius,
                transform,
                bounds,
            )
        }) {
            blocked.insert(tree.0);
            blocking_chunks = blocking_chunks.saturating_add(1);
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
    let faded_opacity = shared_tree_fade_opacity(blocked.len(), blocking_chunks);
    let mut completed = Vec::new();
    for (root, timeline) in &mut timelines.roots {
        advance_tree_fade_timeline(timeline, blocked.contains(root), faded_opacity, delta);
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

fn advance_tree_fade_timeline(
    timeline: &mut TreeFadeTimeline,
    blocked: bool,
    faded_opacity: f32,
    delta: f32,
) {
    if blocked {
        timeline.clear_seconds = 0.0;
        if timeline.amount > faded_opacity {
            let fade_rate = (1.0 - faded_opacity) / TREE_FADE_IN_SECONDS;
            timeline.amount = (timeline.amount - fade_rate * delta).max(faded_opacity);
        } else if timeline.amount < faded_opacity {
            let restore_rate = (1.0 - TREE_FADED_OPACITY) / TREE_FADE_RESTORE_SECONDS;
            timeline.amount = (timeline.amount + restore_rate * delta).min(faded_opacity);
        }
        return;
    }

    let previous_restore =
        (timeline.clear_seconds - TREE_FADE_HOLD_SECONDS).clamp(0.0, TREE_FADE_RESTORE_SECONDS);
    timeline.clear_seconds =
        (timeline.clear_seconds + delta).min(TREE_FADE_HOLD_SECONDS + TREE_FADE_RESTORE_SECONDS);
    let current_restore =
        (timeline.clear_seconds - TREE_FADE_HOLD_SECONDS).clamp(0.0, TREE_FADE_RESTORE_SECONDS);
    let restore_delta = current_restore - previous_restore;
    let remaining_duration = TREE_FADE_RESTORE_SECONDS - previous_restore;
    if restore_delta > 0.0 && remaining_duration > 0.0 {
        let restored_fraction = (restore_delta / remaining_duration).clamp(0.0, 1.0);
        timeline.amount += (1.0 - timeline.amount) * restored_fraction;
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

fn tree_chunk_intersects_focus_corridor(
    start: Vec3,
    end: Vec3,
    corridor_radius: f32,
    transform: &GlobalTransform,
    bounds: Option<&Aabb>,
) -> bool {
    if !start.is_finite()
        || !end.is_finite()
        || !corridor_radius.is_finite()
        || corridor_radius < 0.0
    {
        return false;
    }
    let (mut minimum, mut maximum) = transformed_world_bounds(transform, bounds);
    let padding = corridor_radius + TREE_BOUNDS_PADDING;
    minimum -= Vec3::splat(padding);
    maximum += Vec3::splat(padding);
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

fn clear_character_proximity(
    mut commands: Commands,
    mut owners: Query<
        (Entity, Option<&mut PresentationOcclusion>),
        With<CharacterCameraOcclusionOwner>,
    >,
) {
    for (entity, occlusion) in &mut owners {
        if let Some(occlusion) = occlusion {
            set_reason(
                occlusion,
                PresentationOcclusionReason::CharacterCameraProximity,
                false,
            );
        }
        commands
            .entity(entity)
            .remove::<CharacterCameraOcclusionOwner>();
    }
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

    fn camera_settings() -> CameraSettings {
        CameraSettings {
            gameplay_eye: (0.0, 48.0, 42.0),
            gameplay_focus: (0.0, 6.0, 0.0),
            character_focus_height: 0.4,
            character_radius: 7.0,
            character_probe_radius: 0.1,
            character_collision_margin: 0.35,
            character_restoration_speed: 8.0,
            character_collision_release_delay: 0.2,
            character_self_hide_radius: 1.0,
            character_pitch: 0.3,
            pan_speed: 0.4,
            pan_speed_offset: 10.0,
            min_pitch: 0.25,
            max_pitch: 0.95,
            min_zoom: 5.0,
            max_zoom: 70.0,
            zoom_sensitivity: 0.2,
        }
    }

    fn test_app(target: TilePos, region: InteriorRegionId) -> (App, Entity, Entity) {
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        builder.app_mut().add_plugins(TransformPlugin);
        builder
            .app_mut()
            .insert_resource(camera_settings())
            .init_resource::<CameraMode>()
            .init_resource::<TreeFadeTimelines>()
            .add_systems(
                PostUpdate,
                (
                    reconcile_interior_cutaway,
                    reconcile_tree_fades,
                    reconcile_character_proximity,
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
                Visibility::Inherited,
                PresentationOcclusion::default(),
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

    fn set_camera_distance(app: &mut App, camera: Entity, distance: f32) {
        let focus = app
            .world()
            .entity(camera)
            .get::<PanOrbitCamera>()
            .expect("the camera should retain its orbit state")
            .focus;
        app.world_mut()
            .entity_mut(camera)
            .get_mut::<Transform>()
            .expect("the camera should retain its transform")
            .translation = focus + Vec3::Z * distance;
    }

    fn enter_screen(app: &mut App, screen: Screen) {
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(screen);
        app.update();
    }

    #[test]
    fn close_character_occlusion_uses_entry_and_exit_hysteresis() {
        let target_pos = position(0, 0, 7);
        let (mut app, target, camera) = test_app(target_pos, InteriorRegionId(2));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;

        set_camera_distance(&mut app, camera, 0.8);
        app.update();
        assert_hidden(&app, target);
        let target_ref = app.world().entity(target);
        assert!(target_ref.contains::<CharacterCameraOcclusionOwner>());
        assert!(target_ref
            .get::<PresentationOcclusion>()
            .expect("the target should retain composable occlusion")
            .contains(PresentationOcclusionReason::CharacterCameraProximity));

        set_camera_distance(&mut app, camera, 1.05);
        app.update();
        assert_hidden(&app, target);
        assert!(
            app.world()
                .entity(target)
                .contains::<CharacterCameraOcclusionOwner>(),
            "the target must stay hidden inside the exit hysteresis band"
        );

        set_camera_distance(&mut app, camera, 1.2);
        app.update();
        assert_ordinary(&app, target);
        let target_ref = app.world().entity(target);
        assert!(!target_ref.contains::<CharacterCameraOcclusionOwner>());
        assert!(!target_ref
            .get::<PresentationOcclusion>()
            .expect("the unit owns a persistent composable reason set")
            .contains(PresentationOcclusionReason::CharacterCameraProximity));
    }

    #[test]
    fn map_mode_clears_only_camera_owned_character_occlusion() {
        let target_pos = position(0, 0, 7);
        let (mut app, target, camera) = test_app(target_pos, InteriorRegionId(2));
        app.world_mut()
            .entity_mut(target)
            .get_mut::<PresentationOcclusion>()
            .expect("the target should retain composable occlusion")
            .insert(PresentationOcclusionReason::Fog);
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        set_camera_distance(&mut app, camera, 0.8);
        app.update();
        assert_hidden(&app, target);

        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Map;
        app.update();
        assert_hidden(&app, target);
        let target_ref = app.world().entity(target);
        let reasons = target_ref
            .get::<PresentationOcclusion>()
            .expect("fog must retain the composable reason set");
        assert!(reasons.contains(PresentationOcclusionReason::Fog));
        assert!(!reasons.contains(PresentationOcclusionReason::CharacterCameraProximity));
        assert!(!target_ref.contains::<CharacterCameraOcclusionOwner>());

        app.world_mut()
            .entity_mut(target)
            .get_mut::<PresentationOcclusion>()
            .expect("fog must retain the composable reason set")
            .remove(PresentationOcclusionReason::Fog);
        app.update();
        assert_ordinary(&app, target);
    }

    #[test]
    fn sandbox_deployment_composes_with_character_camera_and_restores_original_visibility() {
        let target_pos = position(0, 0, 7);
        let (mut app, target, camera) = test_app(target_pos, InteriorRegionId(2));
        app.world_mut()
            .entity_mut(target)
            .get_mut::<PresentationOcclusion>()
            .expect("the actor should retain composable occlusion")
            .insert(PresentationOcclusionReason::SandboxDeployment);
        app.update();
        assert_hidden(&app, target);

        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        set_camera_distance(&mut app, camera, 0.8);
        app.update();
        let reasons = app
            .world()
            .entity(target)
            .get::<PresentationOcclusion>()
            .expect("both independent occlusion owners share one reason set");
        assert!(reasons.contains(PresentationOcclusionReason::SandboxDeployment));
        assert!(reasons.contains(PresentationOcclusionReason::CharacterCameraProximity));

        app.world_mut()
            .entity_mut(target)
            .get_mut::<PresentationOcclusion>()
            .expect("the actor should retain composable occlusion")
            .remove(PresentationOcclusionReason::SandboxDeployment);
        app.update();
        assert_hidden(&app, target);

        set_camera_distance(&mut app, camera, 1.2);
        app.update();
        assert_ordinary(&app, target);
    }

    #[test]
    fn retarget_and_target_loss_restore_the_previous_character() {
        let target_pos = position(0, 0, 7);
        let (mut app, first, camera) = test_app(target_pos, InteriorRegionId(2));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        set_camera_distance(&mut app, camera, 0.8);
        app.update();
        assert_hidden(&app, first);

        app.world_mut()
            .entity_mut(first)
            .remove::<CameraFocusTarget>();
        let second = app
            .world_mut()
            .spawn((
                Transform::from_translation(Vec3::ZERO),
                Visibility::Inherited,
                PresentationOcclusion::default(),
                CameraFocusTarget::new(target_pos),
            ))
            .id();
        app.update();
        assert_ordinary(&app, first);
        assert_hidden(&app, second);

        app.world_mut()
            .entity_mut(second)
            .remove::<CameraFocusTarget>();
        app.update();
        assert_ordinary(&app, first);
        assert_ordinary(&app, second);
        assert!(!app
            .world()
            .entity(first)
            .contains::<CharacterCameraOcclusionOwner>());
        assert!(!app
            .world()
            .entity(second)
            .contains::<CharacterCameraOcclusionOwner>());
    }

    #[test]
    fn repeated_gameplay_exits_clear_character_camera_occlusion() {
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_state_plugin();
        builder.app_mut().add_plugins(TransformPlugin);
        builder
            .app_mut()
            .init_state::<Screen>()
            .insert_resource(camera_settings())
            .init_resource::<CameraMode>()
            .add_plugins(plugin);
        let target = builder
            .app_mut()
            .world_mut()
            .spawn((
                Transform::default(),
                Visibility::Inherited,
                PresentationOcclusion::default(),
                CameraFocusTarget::new(TilePos::ORIGIN),
            ))
            .id();
        let focus = Vec3::Y * camera_settings().character_focus_height;
        let camera = builder
            .app_mut()
            .world_mut()
            .spawn((
                Transform::from_translation(focus + Vec3::Z * 0.8),
                PanOrbitCamera { focus, radius: 7.0 },
            ))
            .id();
        let mut app = builder.build();
        app.update();

        for cycle in 0..100 {
            enter_screen(&mut app, Screen::Gameplay);
            *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
            set_camera_distance(&mut app, camera, 0.8);
            app.update();
            assert_hidden(&app, target);

            enter_screen(&mut app, Screen::Title);
            assert_ordinary(&app, target);
            let target_ref = app.world().entity(target);
            assert!(
                !target_ref.contains::<CharacterCameraOcclusionOwner>(),
                "cycle {cycle} leaked camera proximity ownership"
            );
            assert!(
                !target_ref
                    .get::<PresentationOcclusion>()
                    .expect("the unit should retain its composable reason set")
                    .contains(PresentationOcclusionReason::CharacterCameraProximity),
                "cycle {cycle} leaked the camera proximity reason"
            );
        }
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
        let stacked_other = spawn_tree_chunk(
            &mut app,
            other_root,
            on_segment + Vec3::X * 4.0,
            Vec3::splat(0.5),
        );
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
        advance_tree_fade_timeline(
            &mut timeline,
            true,
            TREE_FADED_OPACITY,
            TREE_FADE_IN_SECONDS,
        );
        assert!((timeline.amount - TREE_FADED_OPACITY).abs() < f32::EPSILON);

        advance_tree_fade_timeline(
            &mut timeline,
            false,
            TREE_FADED_OPACITY,
            TREE_FADE_HOLD_SECONDS,
        );
        assert!((timeline.amount - TREE_FADED_OPACITY).abs() < f32::EPSILON);

        advance_tree_fade_timeline(
            &mut timeline,
            false,
            TREE_FADED_OPACITY,
            TREE_FADE_RESTORE_SECONDS * 0.5,
        );
        assert!((timeline.amount - 0.6).abs() < 1e-5);
        advance_tree_fade_timeline(
            &mut timeline,
            false,
            TREE_FADED_OPACITY,
            TREE_FADE_RESTORE_SECONDS * 0.5,
        );
        assert!((timeline.amount - 1.0).abs() < 1e-5);

        advance_tree_fade_timeline(
            &mut timeline,
            true,
            TREE_FADED_OPACITY,
            TREE_FADE_IN_SECONDS * 0.5,
        );
        assert!((timeline.amount - 0.6).abs() < 1e-5);
        assert!(timeline.clear_seconds.abs() < f32::EPSILON);
    }

    #[test]
    fn dense_tree_corridors_share_opacity_without_changing_the_lone_tree_contract() {
        assert!((shared_tree_fade_opacity(1, 6) - TREE_FADED_OPACITY).abs() < f32::EPSILON);
        assert!((shared_tree_fade_opacity(2, 2) - 0.1).abs() < f32::EPSILON);
        assert!((shared_tree_fade_opacity(3, 4) - 0.05).abs() < f32::EPSILON);
        assert!(shared_tree_fade_opacity(usize::MAX, usize::MAX).abs() < f32::EPSILON);
    }

    #[test]
    fn a_shrinking_blocker_group_restores_toward_twenty_percent_without_jumping() {
        let mut timeline = TreeFadeTimeline {
            amount: 0.05,
            clear_seconds: 0.0,
        };

        advance_tree_fade_timeline(&mut timeline, true, TREE_FADED_OPACITY, 0.03);
        assert!(timeline.amount > 0.05 && timeline.amount < TREE_FADED_OPACITY);

        advance_tree_fade_timeline(
            &mut timeline,
            true,
            TREE_FADED_OPACITY,
            TREE_FADE_RESTORE_SECONDS,
        );
        assert!((timeline.amount - TREE_FADED_OPACITY).abs() < f32::EPSILON);
    }

    #[test]
    fn a_group_faded_tree_still_restores_in_the_authored_duration() {
        let grouped_opacity = shared_tree_fade_opacity(4, 5);
        let mut timeline = TreeFadeTimeline {
            amount: grouped_opacity,
            clear_seconds: 0.0,
        };

        advance_tree_fade_timeline(
            &mut timeline,
            false,
            grouped_opacity,
            TREE_FADE_HOLD_SECONDS,
        );
        assert!((timeline.amount - grouped_opacity).abs() < f32::EPSILON);
        advance_tree_fade_timeline(
            &mut timeline,
            false,
            grouped_opacity,
            TREE_FADE_RESTORE_SECONDS,
        );
        assert!((timeline.amount - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn stable_dense_tree_corridor_does_not_republish_chunk_requests() {
        let target = position(0, 0, 7);
        let first_root = position(1, 0, 7);
        let second_root = position(2, 0, 7);
        let unrelated_root = position(3, 0, 7);
        let (mut app, _, _) = test_app(target, InteriorRegionId(1));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        let eye = Vec3::new(0.0, 4.0, 7.0);
        let focus = target.coord.to_world(0.4);
        let on_segment = eye.lerp(focus, 0.5);
        let first_a = spawn_tree_chunk(&mut app, first_root, on_segment, Vec3::splat(0.5));
        let first_b = spawn_tree_chunk(
            &mut app,
            first_root,
            on_segment + Vec3::Y * 0.2,
            Vec3::splat(0.5),
        );
        let first_clear = spawn_tree_chunk(
            &mut app,
            first_root,
            on_segment + Vec3::X * 4.0,
            Vec3::splat(0.5),
        );
        let second = spawn_tree_chunk(
            &mut app,
            second_root,
            on_segment - Vec3::Y * 0.2,
            Vec3::splat(0.5),
        );
        let unrelated = spawn_tree_chunk(
            &mut app,
            unrelated_root,
            on_segment + Vec3::X * 5.0,
            Vec3::splat(0.5),
        );

        app.update();
        app.update();
        app.update();

        let expected = shared_tree_fade_opacity(2, 3);
        for chunk in [first_a, first_b, first_clear, second] {
            assert!((fade_amount(&app, chunk) - expected).abs() < 1e-5);
        }
        assert!((fade_amount(&app, unrelated) - 1.0).abs() < f32::EPSILON);

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
    fn transformed_chunk_bounds_and_probe_drive_corridor_intersection() {
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

        assert!(tree_chunk_intersects_focus_corridor(
            start,
            end,
            0.4,
            &on_segment,
            Some(&bounds)
        ));
        assert!(!tree_chunk_intersects_focus_corridor(
            start,
            end,
            0.4,
            &off_segment,
            Some(&bounds)
        ));

        let off_axis_near_plane =
            GlobalTransform::from(Transform::from_translation(Vec3::new(0.65, 0.0, 2.0)));
        assert!(tree_chunk_intersects_focus_corridor(
            start,
            end,
            0.4,
            &off_axis_near_plane,
            Some(&bounds),
        ));
        assert!(!tree_chunk_intersects_focus_corridor(
            start,
            end,
            0.0,
            &off_axis_near_plane,
            Some(&bounds),
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
