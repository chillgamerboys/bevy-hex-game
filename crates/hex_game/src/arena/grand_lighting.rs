//! Local landmark illumination from world-published Grand anchors.
//! These lights are presentation only; they never publish visibility or occupancy.
use super::ArenaFrame;
use bevy::prelude::*;
use hex_core::arena::{ArenaMap, ArenaReset, ArenaSelection, ArenaStreamInterest};
use hex_map::arena::streamed::StreamedArena;

#[derive(Component)]
struct GrandLamp;

pub(super) fn install(app: &mut App) {
    app.add_systems(Update, present.in_set(ArenaFrame::Present));
}

fn present(
    mut commands: Commands,
    selection: Res<ArenaSelection>,
    reset: Res<ArenaReset>,
    interest: Res<ArenaStreamInterest>,
    streamed: Option<Res<StreamedArena>>,
    mut lamps: Query<(Entity, &Transform, &mut Visibility), With<GrandLamp>>,
    mut identity: Local<Option<(u64, u64)>>,
) {
    let active = streamed
        .as_ref()
        .filter(|_| selection.map == ArenaMap::GrandV4);
    let next = active.map(|streamed| (reset.generation, streamed.overview.package_fingerprint));
    if *identity == next {
        for (_, transform, mut visibility) in &mut lamps {
            let near = transform.translation.distance_squared(interest.position) < 180.0 * 180.0;
            visibility.set_if_neq(if near {
                Visibility::Visible
            } else {
                Visibility::Hidden
            });
        }
        return;
    }
    for (entity, _, _) in &mut lamps {
        commands.entity(entity).despawn();
    }
    *identity = next;
    let Some(streamed) = active else {
        return;
    };
    // Local sources are calibrated against the natural-environment exposure and
    // ambient light. Their falloff makes the opaque chambers readable without changing
    // global daylight or revealing routes through missing/transparent roofs.
    for (anchor, offset, color, intensity, range) in [
        (
            "shrine_plant",
            Vec3::new(0.0, 4.0, 4.0),
            Color::srgb(0.68, 1.0, 0.72),
            400_000.0,
            28.0,
        ),
        (
            "root_temple_entrance",
            Vec3::new(0.0, 5.0, -7.0),
            Color::srgb(1.0, 0.83, 0.59),
            1_200_000.0,
            30.0,
        ),
        (
            "library_entrance",
            Vec3::new(0.0, 5.0, 0.0),
            Color::srgb(1.0, 0.84, 0.63),
            1_200_000.0,
            30.0,
        ),
        (
            "library_hall",
            Vec3::new(0.0, 6.0, 0.0),
            Color::srgb(1.0, 0.86, 0.68),
            1_800_000.0,
            38.0,
        ),
        (
            "library_upper",
            Vec3::new(0.0, 6.0, 0.0),
            Color::srgb(1.0, 0.86, 0.68),
            1_800_000.0,
            38.0,
        ),
        (
            "shadow_tunnel",
            Vec3::new(0.0, 6.0, -80.0),
            Color::srgb(0.63, 0.72, 1.0),
            1_000_000.0,
            80.0,
        ),
        (
            "shadow_tunnel",
            Vec3::new(0.0, 6.0, -20.0),
            Color::srgb(0.63, 0.72, 1.0),
            1_000_000.0,
            80.0,
        ),
        (
            "shadow_tunnel",
            Vec3::new(0.0, 6.0, 40.0),
            Color::srgb(0.63, 0.72, 1.0),
            1_000_000.0,
            80.0,
        ),
        (
            "shadow_tunnel",
            Vec3::new(0.0, 6.0, 100.0),
            Color::srgb(0.63, 0.72, 1.0),
            1_000_000.0,
            80.0,
        ),
    ] {
        let Some(position) = streamed.overview.anchors.get(anchor) else {
            continue;
        };
        let translation = Vec3::from_array(*position) + offset;
        commands.spawn((
            GrandLamp,
            Name::new(format!("Grand {anchor} light")),
            PointLight {
                color,
                intensity,
                range,
                shadow_maps_enabled: true,
                ..default()
            },
            Transform::from_translation(translation),
            if translation.distance_squared(interest.position) < 180.0 * 180.0 {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
        ));
    }
}
