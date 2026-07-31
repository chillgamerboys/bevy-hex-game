//! Screen-space badge projection. Runtime nodes belong to `hex_ui`.

use bevy::prelude::*;
use bevy::transform::TransformSystems;
use hex_core::Screen;
use hex_ui::{BadgeKind, UiSystems, UnitBadgeView, UnitBadgesView};
use hex_units::UnitRegistry;
use hex_world::PanOrbitCamera;

use super::GameplayUiContext;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        PostUpdate,
        publish_view
            .after(TransformSystems::Propagate)
            .before(UiSystems::Render)
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn publish_view(
    context: Res<GameplayUiContext>,
    registry: Res<UnitRegistry>,
    units: Query<&GlobalTransform>,
    cameras: Query<(&Camera, &GlobalTransform), With<PanOrbitCamera>>,
    mut view: ResMut<UnitBadgesView>,
) {
    let camera = cameras.single().ok();
    let acting = context.acting.as_ref().map(|actor| {
        let role = if actor.faction == hex_units::Faction::Player {
            "ACTIVE"
        } else {
            "ENEMY ACTING"
        };
        badge_view(
            actor.unit,
            BadgeKind::Acting,
            format!("{role} · {}", actor.label()),
            camera,
            &registry,
            &units,
        )
    });
    let target = context.target.as_ref().map(|(provenance, target)| {
        badge_view(
            target.unit,
            BadgeKind::Target,
            format!("{} · {}", provenance.label(), target.label()),
            camera,
            &registry,
            &units,
        )
    });
    let next = UnitBadgesView { acting, target };
    if *view != next {
        *view = next;
    }
}

fn badge_view(
    unit: hex_core::UnitId,
    kind: BadgeKind,
    label: String,
    camera: Option<(&Camera, &GlobalTransform)>,
    registry: &UnitRegistry,
    units: &Query<&GlobalTransform>,
) -> UnitBadgeView {
    let anchor = camera.and_then(|(camera, camera_transform)| {
        registry
            .entity_of(unit)
            .and_then(|entity| units.get(entity).ok())
            .and_then(|transform| {
                camera
                    .world_to_viewport(camera_transform, transform.translation() + Vec3::Y)
                    .ok()
            })
    });
    UnitBadgeView {
        unit,
        kind,
        label,
        anchor,
    }
}
