//! Camera-owned water presentation and the reusable fixed-time sky.
use super::{ArenaCamera, ArenaFrame, ViewState};
use bevy::prelude::*;
use hex_arena::ArenaSession;
use hex_core::arena::{ArenaMap, ArenaSelection, ArenaTerrainView, ArenaVoxelGeometry};
use hex_world::battle_sky::BattleSkyFrame;

#[derive(Component)]
struct UnderwaterTint;

pub(super) fn sun_direction() -> Vec3 {
    let elevation = 37.145_f32.to_radians();
    let azimuth = 76.057_f32.to_radians();
    Vec3::new(
        azimuth.sin() * elevation.cos(),
        elevation.sin(),
        azimuth.cos() * elevation.cos(),
    )
}
pub(super) fn install(app: &mut App) {
    hex_world::battle_sky::install(app);
    app.add_systems(Startup, spawn_tint).add_systems(
        Update,
        present
            .in_set(ArenaFrame::Present)
            .after(super::worm_capture::camera),
    );
}
fn spawn_tint(mut commands: Commands) {
    // This layer covers the world image before combat/menu UI (indices10/20).
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::NONE),
        GlobalZIndex(5),
        Pickable::IGNORE,
        UnderwaterTint,
    ));
}
fn water_color(
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    camera: Vec3,
    session: &ArenaSession,
) -> Option<Color> {
    let at = geometry.voxel_at(camera)?;
    if !view.liquids.iter().any(|span| {
        span.bottom.coord == at.coord && (span.bottom.level..=span.top_level).contains(&at.level)
    }) {
        return None;
    }
    let fountain = view.expedition.as_ref().and_then(|sites| {
        sites
            .fountains
            .iter()
            .find(|(_, pool)| pool.cells.contains(&at))
            .map(|(name, _)| name)
    });
    let charged = fountain.is_some_and(|name| {
        session.expedition_progress().is_some_and(|progress| {
            progress
                .fountains
                .iter()
                .any(|pool| &pool.name == name && !pool.consumed)
        })
    });
    Some(if charged {
        Color::srgb(0.08, 0.65, 0.52)
    } else {
        Color::srgb(0.07, 0.34, 0.56)
    })
}
fn present(
    view: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    selection: Res<ArenaSelection>,
    session: Res<ArenaSession>,
    time: Res<Time>,
    state: Res<ViewState>,
    mut sky: ResMut<BattleSkyFrame>,
    mut cameras: Query<(&Transform, &mut DistanceFog), With<ArenaCamera>>,
    mut overlays: Query<(&mut Node, &mut BackgroundColor), With<UnderwaterTint>>,
) {
    let forest = selection.map == ArenaMap::ForestMassif;
    let mut water = None;
    sky.enabled = forest;
    sky.sun_direction = sun_direction();
    sky.cloud_phase = if state.capture.is_some() {
        0.0
    } else {
        time.elapsed_secs()
    };
    if let Ok((camera, mut fog)) = cameras.single_mut() {
        sky.center = camera.translation;
        water = forest
            .then(|| water_color(&view, *geometry, camera.translation, &session))
            .flatten();
        fog.color = water.unwrap_or(Color::srgb(0.62, 0.72, 0.82));
        fog.falloff = FogFalloff::Exponential {
            density: if water.is_some() { 0.10 } else { 0.0003 },
        };
        fog.directional_light_color = if water.is_some() {
            Color::NONE
        } else {
            Color::srgb(1.0, 0.78, 0.50)
        };
    }
    for (mut node, mut background) in &mut overlays {
        super::ux::set_display(
            &mut node,
            if water.is_some() {
                Display::Flex
            } else {
                Display::None
            },
        );
        background.set_if_neq(BackgroundColor(
            water.map_or(Color::NONE, |color| color.with_alpha(0.20)),
        ));
    }
}
