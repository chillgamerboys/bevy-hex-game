//! Development-only adapter between the typed UI controls and the world clock.
//!
//! This module deliberately owns no clock. It projects the active scenario's
//! lighting state into `hex_ui` and mutates an existing cyclic [`TimeOfDay`] only.
//! Shipping builds do not compile this module or the corresponding UI feature.

use bevy::prelude::*;
use hex_assets::{LightingProfile, LightingSettings};
use hex_core::{AppSystems, Screen};
use hex_ui::{DevTimeIntent, DevTimeView, UiIntent, UiSystems};
use hex_world::TimeOfDay;

const STATIC_REASON: &str = "The active lighting profile has a fixed time of day.";
const MISSING_CLOCK_REASON: &str = "The cyclic lighting clock is unavailable.";
const MISSING_SETTINGS_REASON: &str = "Lighting settings are unavailable.";

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
    mut view: ResMut<DevTimeView>,
) {
    match settings.as_deref() {
        None => publish_unavailable(&mut view, MISSING_SETTINGS_REASON),
        Some(settings) => match &settings.profile {
            LightingProfile::Static => publish_unavailable(&mut view, STATIC_REASON),
            LightingProfile::Cycle(_) => match time.as_deref() {
                Some(time) => publish_available(&mut view, time.hours),
                None => publish_unavailable(&mut view, MISSING_CLOCK_REASON),
            },
        },
    }
}

fn publish_available(view: &mut ResMut<DevTimeView>, hours: f32) {
    let unchanged = matches!(
        &*view.bypass_change_detection(),
        DevTimeView::Available { hours: current } if current.to_bits() == hours.to_bits()
    );
    if !unchanged {
        **view = DevTimeView::Available { hours };
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
    mut time: Option<ResMut<TimeOfDay>>,
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
        let Some(time) = time.as_mut() else {
            continue;
        };
        let current = time.bypass_change_detection().hours;
        let next = match intent {
            DevTimeIntent::PreviousHalfHour => (current - 0.5).rem_euclid(24.0),
            DevTimeIntent::NextHalfHour => (current + 0.5).rem_euclid(24.0),
            DevTimeIntent::Midnight => 0.0,
            DevTimeIntent::Dawn => 6.0,
            DevTimeIntent::Noon => 12.0,
            DevTimeIntent::Dusk => 18.0,
        };
        if next.is_finite() && current.to_bits() != next.to_bits() {
            time.hours = next;
        }
    }
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
        IlluminationLevel, InteriorRegions, PerceptionSystems, TerrainReady, TilePos,
        TraversalBlockers,
    };
    use hex_perception::{PerceptionRuntimeStats, ResolvedIllumination};

    use super::*;

    fn cycle_settings() -> Result<LightingSettings, ron::error::SpannedError> {
        ron::from_str(include_str!("../../../assets/config/lighting.ron"))
    }

    fn test_app(settings: Option<LightingSettings>, time: Option<f32>) -> App {
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
        app
    }

    fn send(app: &mut App, intent: DevTimeIntent) {
        app.world_mut().write_message(UiIntent::DevTime(intent));
        app.update();
    }

    fn hours(app: &App) -> f32 {
        app.world().resource::<TimeOfDay>().hours
    }

    #[test]
    fn half_hour_controls_wrap_in_both_directions() {
        let mut app = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            Some(23.75),
        );

        send(&mut app, DevTimeIntent::NextHalfHour);
        assert!((hours(&app) - 0.25).abs() < f32::EPSILON);
        send(&mut app, DevTimeIntent::PreviousHalfHour);
        assert!((hours(&app) - 23.75).abs() < f32::EPSILON);
    }

    #[test]
    fn presets_select_the_authored_hours() {
        let mut app = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            Some(9.0),
        );
        let presets = [
            (DevTimeIntent::Midnight, 0.0),
            (DevTimeIntent::Dawn, 6.0),
            (DevTimeIntent::Noon, 12.0),
            (DevTimeIntent::Dusk, 18.0),
        ];

        for (intent, expected) in presets {
            send(&mut app, intent);
            assert!((hours(&app) - expected).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn unavailable_sources_are_projected_and_never_created() {
        let mut missing = test_app(None, None);
        missing.update();
        assert_eq!(
            *missing.world().resource::<DevTimeView>(),
            DevTimeView::Unavailable {
                reason: MISSING_SETTINGS_REASON.to_owned(),
            }
        );
        send(&mut missing, DevTimeIntent::Noon);
        assert!(!missing.world().contains_resource::<TimeOfDay>());

        let mut static_settings = cycle_settings().expect("shipped cyclic lighting must parse");
        static_settings.profile = LightingProfile::Static;
        let mut static_app = test_app(Some(static_settings), Some(4.0));
        static_app.update();
        assert_eq!(
            *static_app.world().resource::<DevTimeView>(),
            DevTimeView::Unavailable {
                reason: STATIC_REASON.to_owned(),
            }
        );
        send(&mut static_app, DevTimeIntent::Dusk);
        assert!((hours(&static_app) - 4.0).abs() < f32::EPSILON);

        let mut missing_clock = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            None,
        );
        missing_clock.update();
        assert_eq!(
            *missing_clock.world().resource::<DevTimeView>(),
            DevTimeView::Unavailable {
                reason: MISSING_CLOCK_REASON.to_owned(),
            }
        );
    }

    #[test]
    fn unavailable_intents_are_drained_instead_of_replayed_later() {
        let mut static_settings = cycle_settings().expect("shipped cyclic lighting must parse");
        static_settings.profile = LightingProfile::Static;
        let mut app = test_app(Some(static_settings), None);

        send(&mut app, DevTimeIntent::Dusk);
        app.insert_resource(cycle_settings().expect("shipped cyclic lighting must parse"));
        app.insert_resource(TimeOfDay { hours: 4.0 });
        app.update();

        assert!((hours(&app) - 4.0).abs() < f32::EPSILON);
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
        let mut app = test_app(None, None);
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
    fn invalid_inspector_time_does_not_churn_and_a_preset_recovers_it() {
        let mut app = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            Some(f32::NAN),
        );
        app.init_resource::<ViewObservation>()
            .add_systems(Update, observe_view.after(UiSystems::Render));

        app.update();
        app.update();
        assert_eq!(app.world().resource::<ViewObservation>().0, 1);

        send(&mut app, DevTimeIntent::Noon);
        assert!((hours(&app) - 12.0).abs() < f32::EPSILON);
        assert_eq!(app.world().resource::<ViewObservation>().0, 2);
        assert_eq!(
            *app.world().resource::<DevTimeView>(),
            DevTimeView::Available { hours: 12.0 }
        );
    }

    #[derive(Resource, Default)]
    struct UpdateObservation {
        hours: Option<f32>,
        changed_frames: u8,
    }

    fn observe_update(time: Res<TimeOfDay>, mut observed: ResMut<UpdateObservation>) {
        observed.hours = Some(time.hours);
        if time.is_changed() {
            observed.changed_frames += 1;
        }
    }

    #[test]
    fn one_intent_mutates_once_before_world_update() {
        let mut app = test_app(
            Some(cycle_settings().expect("shipped cyclic lighting must parse")),
            Some(11.5),
        );
        app.init_resource::<UpdateObservation>()
            .add_systems(Update, observe_update.in_set(AppSystems::Update));
        app.update();
        assert_eq!(
            app.world().resource::<UpdateObservation>().changed_frames,
            1
        );

        send(&mut app, DevTimeIntent::Noon);

        assert!((hours(&app) - 12.0).abs() < f32::EPSILON);
        assert_eq!(
            app.world().resource::<UpdateObservation>().hours,
            Some(12.0)
        );
        assert_eq!(
            app.world().resource::<UpdateObservation>().changed_frames,
            2
        );
        assert_eq!(
            *app.world().resource::<DevTimeView>(),
            DevTimeView::Available { hours: 12.0 }
        );

        app.update();
        send(&mut app, DevTimeIntent::Noon);
        assert_eq!(
            app.world().resource::<UpdateObservation>().changed_frames,
            2,
            "idle frames and the selected preset must not write an unchanged clock"
        );
    }

    #[test]
    fn time_intent_reaches_lighting_and_perception_in_the_same_frame() {
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
            .insert_resource(PerceptionSettings::default())
            .insert_resource(InteriorRegions::new())
            .insert_resource(TraversalBlockers::new())
            .insert_resource(TerrainReady)
            .insert_resource(table)
            .add_plugins((hex_world::sky::plugin, hex_perception::plugin, plugin));
        app.world_mut().spawn((
            HexTile,
            position,
            HexSpan::new(0.0, 1.0),
            stone,
            Headroom(2),
        ));
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();

        let before = *app.world().resource::<PerceptionRuntimeStats>();
        app.world_mut()
            .write_message(UiIntent::DevTime(DevTimeIntent::Midnight));
        app.update();

        assert!(hours(&app).abs() < f32::EPSILON);
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
            IlluminationLevel::Dim
        );
        assert_eq!(
            app.world()
                .resource::<ResolvedIllumination>()
                .get(position)
                .map(|light| light.level),
            Some(IlluminationLevel::Dim)
        );

        let after = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(after.frames_checked, before.frames_checked + 1);
        assert_eq!(after.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            after.illumination_resolutions,
            before.illumination_resolutions + 1
        );
        assert_eq!(
            after.observation_resolutions,
            before.observation_resolutions + 1
        );
        assert_eq!(
            after.knowledge_publications,
            before.knowledge_publications + 1
        );
    }
}
