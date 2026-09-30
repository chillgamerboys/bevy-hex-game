//! Development-only adapter between typed UI controls and presentation time.
//!
//! This module deliberately owns no clock. It projects the active scenario's
//! authoritative and preview time into `hex_ui`, while mutating only the local
//! render override. Shipping builds compile neither that override nor this UI.

use bevy::prelude::*;
use hex_assets::{LightingProfile, LightingSettings};
use hex_core::{AppSystems, Screen};
use hex_ui::{DevTimeIntent, DevTimeView, UiIntent, UiSystems};
use hex_world::{PresentationTimeOverride, TimeOfDay};

const STATIC_REASON: &str = "The active lighting profile has a fixed time of day.";
const MISSING_CLOCK_REASON: &str = "The cyclic lighting clock is unavailable.";
const MISSING_PREVIEW_REASON: &str = "The local presentation-time preview is unavailable.";
const MISSING_SETTINGS_REASON: &str = "Lighting settings are unavailable.";
const INVALID_CLOCK_REASON: &str = "The cyclic lighting clock contains an invalid value.";
const INVALID_PREVIEW_REASON: &str =
    "The local presentation-time preview contains an invalid value.";
const PREVIEW_MIN_HOURS: f32 = 0.0;
const PREVIEW_MAX_HOURS: f32 = 23.75;
const PREVIEW_STEP_HOURS: f32 = 0.25;

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        handle_intents
            .after(UiSystems::EmitIntents)
            .before(AppSystems::Update)
            .run_if(in_state(Screen::Gameplay)),
    )
    .add_systems(
        Update,
        publish_view
            .after(AppSystems::Update)
            .before(UiSystems::Render)
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn publish_view(
    settings: Option<Res<LightingSettings>>,
    time: Option<Res<TimeOfDay>>,
    preview: Option<Res<PresentationTimeOverride>>,
    mut view: ResMut<DevTimeView>,
) {
    match settings.as_deref() {
        None => publish_unavailable(&mut view, MISSING_SETTINGS_REASON),
        Some(settings) => match &settings.profile {
            LightingProfile::Static => publish_unavailable(&mut view, STATIC_REASON),
            LightingProfile::Cycle(_) => {
                let Some(time) = time.as_deref() else {
                    publish_unavailable(&mut view, MISSING_CLOCK_REASON);
                    return;
                };
                let Some(preview) = preview.as_deref() else {
                    publish_unavailable(&mut view, MISSING_PREVIEW_REASON);
                    return;
                };
                if !time.hours.is_finite() {
                    publish_unavailable(&mut view, INVALID_CLOCK_REASON);
                } else if preview.hours.is_some_and(|hours| !hours.is_finite()) {
                    publish_unavailable(&mut view, INVALID_PREVIEW_REASON);
                } else {
                    publish_available(&mut view, time.hours, preview.hours);
                }
            }
        },
    }
}

fn publish_available(view: &mut ResMut<DevTimeView>, game_hours: f32, preview_hours: Option<f32>) {
    let unchanged = matches!(
        &*view.bypass_change_detection(),
        DevTimeView::Available {
            game_hours: current_game,
            preview_hours: current_preview,
        } if current_game.to_bits() == game_hours.to_bits()
            && same_optional_hours(*current_preview, preview_hours)
    );
    if !unchanged {
        **view = DevTimeView::Available {
            game_hours,
            preview_hours,
        };
    }
}

fn same_optional_hours(left: Option<f32>, right: Option<f32>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.to_bits() == right.to_bits(),
        (None, None) => true,
        _ => false,
    }
}

fn publish_unavailable(view: &mut ResMut<DevTimeView>, reason: &str) {
    let unchanged = matches!(
        &*view.bypass_change_detection(),
        DevTimeView::Unavailable { reason: current } if current == reason
    );
    if !unchanged {
        **view = DevTimeView::Unavailable {
            reason: reason.to_owned(),
        };
    }
}

fn handle_intents(
    settings: Option<Res<LightingSettings>>,
    time: Option<Res<TimeOfDay>>,
    mut preview: Option<ResMut<PresentationTimeOverride>>,
    mut intents: MessageReader<UiIntent>,
) {
    let cyclic = settings
        .as_deref()
        .is_some_and(|settings| matches!(&settings.profile, LightingProfile::Cycle(_)));

    for intent in intents.read() {
        let UiIntent::DevTime(intent) = intent else {
            continue;
        };
        if !cyclic {
            continue;
        }
        if time.is_none() {
            continue;
        }
        let Some(preview) = preview.as_mut() else {
            continue;
        };
        let current = preview.bypass_change_detection().hours;
        let next = match intent {
            DevTimeIntent::SetPreviewHours(hours) if hours.is_finite() => {
                Some(quantize_preview_hours(*hours))
            }
            DevTimeIntent::SetPreviewHours(_) => continue,
            DevTimeIntent::ResetPreview => None,
        };
        if !same_optional_hours(current, next) {
            preview.hours = next;
        }
    }
}

fn quantize_preview_hours(hours: f32) -> f32 {
    let bounded = hours.clamp(PREVIEW_MIN_HOURS, PREVIEW_MAX_HOURS);
    ((bounded / PREVIEW_STEP_HOURS).round() * PREVIEW_STEP_HOURS)
        .clamp(PREVIEW_MIN_HOURS, PREVIEW_MAX_HOURS)
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use hex_assets::{
        ArtPalette, CelestialBody, PerceptionSettings, ResolvedLighting, SubstanceFile,
        SubstanceTable,
    };
    use hex_core::{
        ExteriorIllumination, GameplaySetup, Headroom, HexCoord, HexSpan, HexTile,
        IlluminationLevel, InteriorRegions, LocalMapKnowledge, PerceptionSystems,
        PresentationOcclusion, PresentationOcclusionReason, RunBottom, TerrainReady, TilePos,
        TraversalBlockers, UnitId,
    };
    use hex_perception::{
        FactionMapKnowledge, FactionObservations, PerceptionRuntimeStats, ResolvedIllumination,
    };
    use hex_units::Enemy;

    use super::*;

    fn cycle_settings() -> Result<LightingSettings, ron::error::SpannedError> {
        ron::from_str(include_str!("../../../assets/config/lighting.ron"))
    }

    fn test_app(
        settings: Option<LightingSettings>,
        time: Option<f32>,
        preview: Option<Option<f32>>,
    ) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .init_state::<Screen>()
            .insert_state(Screen::Gameplay)
            .add_message::<UiIntent>()
            .init_resource::<DevTimeView>()
            .configure_sets(Update, (UiSystems::EmitIntents, AppSystems::Update).chain())
            .add_plugins(plugin);
        if let Some(settings) = settings {
            app.insert_resource(settings);
        }
        if let Some(hours) = time {
            app.insert_resource(TimeOfDay { hours });
        }
        if let Some(hours) = preview {
            app.insert_resource(PresentationTimeOverride { hours });
        }
        app
    }

    fn send(app: &mut App, intent: DevTimeIntent) {
        app.world_mut().write_message(UiIntent::DevTime(intent));
        app.update();
    }

    fn game_hours(app: &App) -> f32 {
        app.world().resource::<TimeOfDay>().hours
    }

    fn preview_hours(app: &App) -> Option<f32> {
        app.world().resource::<PresentationTimeOverride>().hours
    }

    fn assert_hours_eq(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= f32::EPSILON,
            "expected {expected} hours, got {actual}"
        );
    }

    #[derive(Debug, Clone, PartialEq)]
    struct TacticalFogBatchSnapshot {
        name: String,
        mesh: Handle<Mesh>,
        material: Handle<StandardMaterial>,
        transform: Transform,
        visibility: Visibility,
        occlusion: PresentationOcclusion,
        base_color: Color,
        alpha_mode: AlphaMode,
        unlit: bool,
        depth_bias: f32,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct TacticalFogSnapshot {
        batches: Vec<TacticalFogBatchSnapshot>,
        hostile_occlusions: Vec<(Entity, UnitId, PresentationOcclusion)>,
        mesh_asset_count: usize,
        material_asset_count: usize,
    }

    fn tactical_fog_snapshot(world: &mut World) -> TacticalFogSnapshot {
        let raw_batches = {
            let mut query = world.query::<(
                &Name,
                &Mesh3d,
                &MeshMaterial3d<StandardMaterial>,
                &Transform,
                &Visibility,
                &PresentationOcclusion,
            )>();
            query
                .iter(world)
                .filter(|(name, ..)| name.as_str().starts_with("FogOverlayBatch["))
                .map(|(name, mesh, material, transform, visibility, occlusion)| {
                    (
                        name.as_str().to_owned(),
                        mesh.0.clone(),
                        material.0.clone(),
                        *transform,
                        *visibility,
                        *occlusion,
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut hostile_occlusions = {
            let mut query =
                world.query_filtered::<(Entity, &UnitId, &PresentationOcclusion), With<Enemy>>();
            query
                .iter(world)
                .map(|(entity, unit, occlusion)| (entity, *unit, *occlusion))
                .collect::<Vec<_>>()
        };
        hostile_occlusions.sort_by_key(|(entity, _, _)| *entity);
        let materials = world.resource::<Assets<StandardMaterial>>();
        let mut batches = raw_batches
            .into_iter()
            .map(|(name, mesh, material, transform, visibility, occlusion)| {
                let resolved = materials
                    .get(&material)
                    .expect("every tactical-fog batch should retain its material");
                TacticalFogBatchSnapshot {
                    name,
                    mesh,
                    material,
                    transform,
                    visibility,
                    occlusion,
                    base_color: resolved.base_color,
                    alpha_mode: resolved.alpha_mode,
                    unlit: resolved.unlit,
                    depth_bias: resolved.depth_bias,
                }
            })
            .collect::<Vec<_>>();
        batches.sort_by(|left, right| left.name.cmp(&right.name));
        TacticalFogSnapshot {
            batches,
            hostile_occlusions,
            mesh_asset_count: world.resource::<Assets<Mesh>>().len(),
            material_asset_count: materials.len(),
        }
    }

    #[test]
    fn preview_values_are_quarter_hour_bounded_without_wrapping() {
        let mut app = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            Some(9.0),
            Some(None),
        );

        for (requested, expected) in [
            (-0.5, 0.0),
            (0.13, 0.25),
            (16.62, 16.5),
            (23.9, 23.75),
            (30.0, 23.75),
        ] {
            send(&mut app, DevTimeIntent::SetPreviewHours(requested));
            assert_eq!(preview_hours(&app), Some(expected));
            assert_hours_eq(game_hours(&app), 9.0);
        }

        send(&mut app, DevTimeIntent::SetPreviewHours(f32::NAN));
        assert_eq!(preview_hours(&app), Some(23.75));
        assert_hours_eq(game_hours(&app), 9.0);
    }

    #[test]
    fn authored_presets_and_reset_update_only_the_preview() {
        let mut app = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            Some(9.0),
            Some(None),
        );
        let presets = [0.0, 6.5, 12.0, 16.5, 18.5, 20.0];

        for expected in presets {
            send(&mut app, DevTimeIntent::SetPreviewHours(expected));
            assert_eq!(preview_hours(&app), Some(expected));
            assert_hours_eq(game_hours(&app), 9.0);
        }
        send(&mut app, DevTimeIntent::ResetPreview);
        assert_eq!(preview_hours(&app), None);
        assert_hours_eq(game_hours(&app), 9.0);
    }

    #[test]
    fn unavailable_sources_are_projected_and_never_created() {
        let mut missing = test_app(None, None, None);
        missing.update();
        assert_eq!(
            *missing.world().resource::<DevTimeView>(),
            DevTimeView::Unavailable {
                reason: MISSING_SETTINGS_REASON.to_owned(),
            }
        );
        send(&mut missing, DevTimeIntent::SetPreviewHours(12.0));
        assert!(!missing.world().contains_resource::<TimeOfDay>());
        assert!(!missing
            .world()
            .contains_resource::<PresentationTimeOverride>());

        let mut static_settings = cycle_settings().expect("shipped cyclic lighting must parse");
        static_settings.profile = LightingProfile::Static;
        let mut static_app = test_app(Some(static_settings), Some(4.0), Some(None));
        static_app.update();
        assert_eq!(
            *static_app.world().resource::<DevTimeView>(),
            DevTimeView::Unavailable {
                reason: STATIC_REASON.to_owned(),
            }
        );
        send(&mut static_app, DevTimeIntent::SetPreviewHours(18.5));
        assert_hours_eq(game_hours(&static_app), 4.0);
        assert_eq!(preview_hours(&static_app), None);

        let mut missing_clock = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            None,
            Some(None),
        );
        missing_clock.update();
        assert_eq!(
            *missing_clock.world().resource::<DevTimeView>(),
            DevTimeView::Unavailable {
                reason: MISSING_CLOCK_REASON.to_owned(),
            }
        );

        let mut missing_preview = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            Some(4.0),
            None,
        );
        missing_preview.update();
        assert_eq!(
            *missing_preview.world().resource::<DevTimeView>(),
            DevTimeView::Unavailable {
                reason: MISSING_PREVIEW_REASON.to_owned(),
            }
        );
    }

    #[test]
    fn unavailable_intents_are_drained_instead_of_replayed_later() {
        let mut static_settings = cycle_settings().expect("shipped cyclic lighting must parse");
        static_settings.profile = LightingProfile::Static;
        let mut app = test_app(Some(static_settings), Some(4.0), Some(None));

        send(&mut app, DevTimeIntent::SetPreviewHours(18.5));
        app.insert_resource(cycle_settings().expect("shipped cyclic lighting must parse"));
        app.update();

        assert_eq!(preview_hours(&app), None);
        assert_hours_eq(game_hours(&app), 4.0);
    }

    #[derive(Resource, Default)]
    struct ViewObservation(u8);

    fn observe_view(view: Res<DevTimeView>, mut observed: ResMut<ViewObservation>) {
        if view.is_changed() {
            observed.0 += 1;
        }
    }

    #[test]
    fn unchanged_unavailable_view_does_not_churn() {
        let mut app = test_app(None, None, None);
        app.init_resource::<ViewObservation>()
            .add_systems(Update, observe_view.after(UiSystems::Render));

        app.update();
        app.update();

        assert_eq!(app.world().resource::<ViewObservation>().0, 1);
        assert_eq!(
            *app.world().resource::<DevTimeView>(),
            DevTimeView::Unavailable {
                reason: MISSING_SETTINGS_REASON.to_owned(),
            }
        );
    }

    #[test]
    fn invalid_inspector_values_are_explicit_and_do_not_churn() {
        let mut invalid_clock = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            Some(f32::NAN),
            Some(None),
        );
        invalid_clock
            .init_resource::<ViewObservation>()
            .add_systems(Update, observe_view.after(UiSystems::Render));

        invalid_clock.update();
        invalid_clock.update();
        assert_eq!(invalid_clock.world().resource::<ViewObservation>().0, 1);
        assert_eq!(
            *invalid_clock.world().resource::<DevTimeView>(),
            DevTimeView::Unavailable {
                reason: INVALID_CLOCK_REASON.to_owned(),
            }
        );

        let mut invalid_preview = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            Some(12.0),
            Some(Some(f32::INFINITY)),
        );
        invalid_preview.update();
        assert_eq!(
            *invalid_preview.world().resource::<DevTimeView>(),
            DevTimeView::Unavailable {
                reason: INVALID_PREVIEW_REASON.to_owned(),
            }
        );
    }

    #[derive(Resource, Default)]
    struct UpdateObservation {
        game_hours: Option<f32>,
        preview_hours: Option<f32>,
        game_changed_frames: u8,
        preview_changed_frames: u8,
    }

    fn observe_update(
        time: Res<TimeOfDay>,
        preview: Res<PresentationTimeOverride>,
        mut observed: ResMut<UpdateObservation>,
    ) {
        observed.game_hours = Some(time.hours);
        observed.preview_hours = preview.hours;
        if time.is_changed() {
            observed.game_changed_frames += 1;
        }
        if preview.is_changed() {
            observed.preview_changed_frames += 1;
        }
    }

    #[test]
    fn one_intent_mutates_only_the_preview_once_before_world_update() {
        let mut app = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            Some(11.5),
            Some(None),
        );
        app.init_resource::<UpdateObservation>()
            .add_systems(Update, observe_update.in_set(AppSystems::Update));
        app.update();
        assert_eq!(
            app.world()
                .resource::<UpdateObservation>()
                .game_changed_frames,
            1
        );
        assert_eq!(
            app.world()
                .resource::<UpdateObservation>()
                .preview_changed_frames,
            1
        );

        send(&mut app, DevTimeIntent::SetPreviewHours(12.0));

        assert_hours_eq(game_hours(&app), 11.5);
        assert_eq!(preview_hours(&app), Some(12.0));
        assert_eq!(
            app.world().resource::<UpdateObservation>().game_hours,
            Some(11.5)
        );
        assert_eq!(
            app.world().resource::<UpdateObservation>().preview_hours,
            Some(12.0)
        );
        assert_eq!(
            app.world()
                .resource::<UpdateObservation>()
                .game_changed_frames,
            1
        );
        assert_eq!(
            app.world()
                .resource::<UpdateObservation>()
                .preview_changed_frames,
            2
        );
        assert_eq!(
            *app.world().resource::<DevTimeView>(),
            DevTimeView::Available {
                game_hours: 11.5,
                preview_hours: Some(12.0),
            }
        );

        app.update();
        send(&mut app, DevTimeIntent::SetPreviewHours(12.0));
        assert_eq!(
            app.world()
                .resource::<UpdateObservation>()
                .preview_changed_frames,
            2,
            "idle frames and a repeated value must not rewrite the preview"
        );
        assert_eq!(
            app.world()
                .resource::<UpdateObservation>()
                .game_changed_frames,
            1
        );
    }

    #[test]
    fn preview_intent_reaches_presentation_in_the_same_frame_without_changing_perception() {
        let palette: ArtPalette = ron::from_str(include_str!("../../../assets/art/palette.ron"))
            .expect("shipped art palette must parse");
        let substances: SubstanceFile =
            ron::from_str(include_str!("../../../assets/config/substances.ron"))
                .expect("shipped substances must parse");
        let table = SubstanceTable::from_file(&substances, &palette)
            .expect("shipped substances must resolve through the art palette");
        let stone = table.id("stone").expect("shipped stone substance");
        let position = TilePos::new(HexCoord::from_axial(0, 0), 0);

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .init_state::<Screen>()
            .add_message::<UiIntent>()
            .init_resource::<DevTimeView>()
            .configure_sets(
                Update,
                (
                    AppSystems::TickTimers,
                    AppSystems::RecordInput,
                    AppSystems::Update,
                )
                    .chain(),
            )
            .configure_sets(
                Update,
                (
                    PerceptionSystems::PublishAmbient,
                    PerceptionSystems::ResolveIllumination,
                    PerceptionSystems::ResolveObservation,
                    PerceptionSystems::PublishKnowledge,
                    PerceptionSystems::ApplyPresentation,
                )
                    .chain()
                    .in_set(AppSystems::Update),
            )
            .configure_sets(
                OnEnter(Screen::Gameplay),
                (
                    GameplaySetup::Resources,
                    GameplaySetup::Terrain,
                    GameplaySetup::Actors,
                    GameplaySetup::Restore,
                    GameplaySetup::Perception,
                    GameplaySetup::View,
                    GameplaySetup::Finalize,
                )
                    .chain(),
            )
            .configure_sets(
                OnEnter(Screen::Gameplay),
                (
                    PerceptionSystems::PublishAmbient,
                    PerceptionSystems::ResolveIllumination,
                    PerceptionSystems::ResolveObservation,
                    PerceptionSystems::PublishKnowledge,
                    PerceptionSystems::ApplyPresentation,
                )
                    .chain()
                    .in_set(GameplaySetup::Perception),
            )
            .insert_resource(cycle_settings().expect("shipped cyclic lighting must parse"))
            .insert_resource(TimeOfDay { hours: 12.0 })
            .insert_resource(GlobalAmbientLight::default())
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .insert_resource(PerceptionSettings::default())
            .insert_resource(InteriorRegions::new())
            .insert_resource(TraversalBlockers::new())
            .insert_resource(TerrainReady)
            .insert_resource(table)
            .add_plugins((
                hex_world::sky::plugin,
                hex_units::terrain_occupancy::plugin,
                hex_units::authored_object_occupancy::plugin,
                hex_perception::plugin,
                crate::fog::plugin,
                plugin,
            ));
        app.world_mut().spawn((
            HexTile,
            position,
            RunBottom(position.level),
            HexSpan::new(0.0, 1.0),
            stone,
            Headroom(2),
        ));
        app.world_mut()
            .spawn((Enemy, UnitId(7), PresentationOcclusion::default()));
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();

        let exterior_before = *app.world().resource::<ExteriorIllumination>();
        let illumination_before = app.world().resource::<ResolvedIllumination>().clone();
        let observations_before = app.world().resource::<FactionObservations>().clone();
        let knowledge_before = app.world().resource::<FactionMapKnowledge>().clone();
        let local_knowledge_before = app
            .world()
            .resource::<LocalMapKnowledge>()
            .iter()
            .collect::<Vec<_>>();
        let fog_before = tactical_fog_snapshot(app.world_mut());
        assert!(
            !fog_before.batches.is_empty(),
            "the integration fixture must exercise a real tactical-fog overlay"
        );
        assert_eq!(
            fog_before.hostile_occlusions.len(),
            1,
            "the integration fixture must exercise hostile fog ownership"
        );
        let (_, hostile, occlusion) = fog_before
            .hostile_occlusions
            .first()
            .expect("the hostile fog snapshot should retain its fixture");
        assert_eq!(*hostile, UnitId(7));
        assert!(occlusion.contains(PresentationOcclusionReason::Fog));
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        app.world_mut()
            .write_message(UiIntent::DevTime(DevTimeIntent::SetPreviewHours(0.0)));
        app.update();

        assert_hours_eq(game_hours(&app), 12.0);
        assert_eq!(preview_hours(&app), Some(0.0));
        let lighting = app.world().resource::<ResolvedLighting>();
        assert!(
            lighting
                .time_hours
                .is_some_and(|hours| hours.abs() < f32::EPSILON),
            "the renderer-safe lighting frame must resolve the new clock"
        );
        assert_eq!(lighting.key_body, Some(CelestialBody::Moon));
        assert_eq!(
            app.world().resource::<ExteriorIllumination>().level,
            IlluminationLevel::Bright,
            "local visual preview must not change authoritative gameplay illumination"
        );
        assert_eq!(
            *app.world().resource::<ExteriorIllumination>(),
            exterior_before,
            "the authoritative illumination resource must remain byte-for-byte equal"
        );
        assert_eq!(
            *app.world().resource::<ResolvedIllumination>(),
            illumination_before,
            "the exact gameplay-light projection must remain byte-for-byte equal"
        );
        assert_eq!(
            *app.world().resource::<FactionObservations>(),
            observations_before,
            "faction observations must remain byte-for-byte equal"
        );
        assert_eq!(
            *app.world().resource::<FactionMapKnowledge>(),
            knowledge_before,
            "faction knowledge must remain byte-for-byte equal"
        );
        assert_eq!(
            app.world()
                .resource::<LocalMapKnowledge>()
                .iter()
                .collect::<Vec<_>>(),
            local_knowledge_before,
            "the movement-facing knowledge projection must remain byte-for-byte equal"
        );
        assert_eq!(
            tactical_fog_snapshot(app.world_mut()),
            fog_before,
            "the tactical-fog batches and materials must remain byte-for-byte equal"
        );
        assert_eq!(
            app.world()
                .resource::<ResolvedIllumination>()
                .get(position)
                .map(|light| light.level),
            Some(IlluminationLevel::Bright)
        );

        let after = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(after.frames_checked, before.frames_checked + 1);
        assert_eq!(after.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            after.illumination_resolutions, before.illumination_resolutions,
            "a render-only preview must not invalidate gameplay illumination"
        );
        assert_eq!(
            after.observation_resolutions, before.observation_resolutions,
            "a render-only preview must not invalidate gameplay observation"
        );
        assert_eq!(
            after.knowledge_publications, before.knowledge_publications,
            "a render-only preview must not republish gameplay knowledge"
        );

        app.world_mut()
            .write_message(UiIntent::DevTime(DevTimeIntent::ResetPreview));
        app.update();
        assert_hours_eq(game_hours(&app), 12.0);
        assert_eq!(preview_hours(&app), None);
        let lighting = app.world().resource::<ResolvedLighting>();
        assert_eq!(lighting.time_hours, Some(12.0));
        assert_eq!(lighting.key_body, Some(CelestialBody::Sun));
        assert_eq!(
            *app.world().resource::<ExteriorIllumination>(),
            exterior_before,
            "reset must preserve authoritative illumination exactly"
        );
        assert_eq!(
            *app.world().resource::<ResolvedIllumination>(),
            illumination_before,
            "reset must preserve the gameplay-light projection exactly"
        );
        assert_eq!(
            *app.world().resource::<FactionObservations>(),
            observations_before,
            "reset must preserve faction observations exactly"
        );
        assert_eq!(
            *app.world().resource::<FactionMapKnowledge>(),
            knowledge_before,
            "reset must preserve faction knowledge exactly"
        );
        assert_eq!(
            app.world()
                .resource::<LocalMapKnowledge>()
                .iter()
                .collect::<Vec<_>>(),
            local_knowledge_before,
            "reset must preserve the movement-facing knowledge projection exactly"
        );
        assert_eq!(
            tactical_fog_snapshot(app.world_mut()),
            fog_before,
            "reset must preserve terrain and hostile tactical-fog presentation exactly"
        );
        let reset_stats = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(reset_stats.frames_checked, after.frames_checked + 1);
        assert_eq!(reset_stats.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            reset_stats.illumination_resolutions,
            before.illumination_resolutions
        );
        assert_eq!(
            reset_stats.observation_resolutions,
            before.observation_resolutions
        );
        assert_eq!(
            reset_stats.knowledge_publications,
            before.knowledge_publications
        );
    }
}
