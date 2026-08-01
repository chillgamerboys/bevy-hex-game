use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, button, despawn_screen, display, fine, heading, label, panel, screen_root, screen_title,
    GameplayAction, PauseView, ResolvedUiMetrics, UiAssets, UiIntent, UiSetting, UiSettingsView,
    UiSystems, UiViewportClass,
};

#[derive(Component)]
struct SettingControl(UiSetting);

#[derive(Component)]
struct SettingNotice;

#[derive(Component)]
struct SettingsBack;

#[derive(Component)]
struct SettingsSurface;

#[derive(Component)]
struct SettingsRoot;

#[derive(Component)]
struct PauseOverlay;

#[derive(Component)]
struct PauseHint;

#[derive(Component)]
struct PauseNotice;

#[derive(Component)]
struct ResumeControl;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Splash), spawn_splash)
        .add_systems(OnExit(Screen::Splash), despawn_screen(Screen::Splash))
        .add_systems(OnEnter(Screen::Loading), spawn_loading)
        .add_systems(OnExit(Screen::Loading), despawn_screen(Screen::Loading))
        .add_systems(OnEnter(Screen::Settings), spawn_settings)
        .add_systems(
            Update,
            (
                (refresh_settings, apply_settings_layout)
                    .chain()
                    .in_set(UiSystems::Render),
                handle_settings_controls,
            )
                .run_if(in_state(Screen::Settings)),
        )
        .add_systems(OnExit(Screen::Settings), despawn_screen(Screen::Settings));
    app.add_systems(OnEnter(hex_core::Pause(true)), spawn_pause)
        .add_systems(
            Update,
            (
                refresh_pause.in_set(UiSystems::Render),
                handle_pause_controls.in_set(UiSystems::EmitIntents),
            )
                .run_if(in_state(hex_core::Pause(true))),
        )
        .add_systems(OnExit(hex_core::Pause(true)), despawn_pause);
}

fn spawn_pause(mut commands: Commands, assets: Res<UiAssets>, view: Res<PauseView>) {
    commands
        .spawn((crate::overlay_root("Pause Menu"), PauseOverlay))
        .with_children(|root| {
            root.spawn(screen_title(&assets, "Paused"));
            root.spawn((PauseHint, blurb(&assets, view.hint.clone())));
            root.spawn((
                PauseNotice,
                blurb(&assets, view.notice.clone().unwrap_or_default()),
            ));
            root.spawn((button("Resume"), ResumeControl))
                .with_children(|resume| {
                    resume.spawn(label(&assets, "Resume"));
                    resume.spawn(fine(&assets, "ESC"));
                });
        });
}

fn handle_pause_controls(
    controls: Query<&Interaction, (Changed<Interaction>, With<ResumeControl>)>,
    mut intents: MessageWriter<UiIntent>,
) {
    for interaction in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Gameplay(GameplayAction::Pause));
        }
    }
}

fn refresh_pause(
    view: Res<PauseView>,
    mut hints: Query<&mut Text, (With<PauseHint>, Without<PauseNotice>)>,
    mut notices: Query<&mut Text, (With<PauseNotice>, Without<PauseHint>)>,
) {
    if !view.is_changed() {
        return;
    }
    for mut hint in &mut hints {
        hint.0.clone_from(&view.hint);
    }
    for mut notice in &mut notices {
        notice.0 = view.notice.clone().unwrap_or_default();
    }
}

fn despawn_pause(mut commands: Commands, overlays: Query<Entity, With<PauseOverlay>>) {
    for entity in &overlays {
        commands.entity(entity).despawn();
    }
}

fn spawn_splash(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn(screen_root(Screen::Splash, "Splash Screen"))
        .with_children(|root| {
            root.spawn(display(&assets, "Hex Game"));
            root.spawn(blurb(&assets, "A deterministic elemental tactics game"));
        });
}

fn spawn_loading(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn(screen_root(Screen::Loading, "Loading Screen"))
        .with_children(|root| {
            root.spawn(display(&assets, "Preparing the battlefield"));
            root.spawn(blurb(
                &assets,
                "Loading and validating content before play begins…",
            ));
        });
}

fn spawn_settings(mut commands: Commands, assets: Res<UiAssets>, view: Res<UiSettingsView>) {
    commands
        .spawn((
            screen_root(Screen::Settings, "Settings Screen"),
            SettingsRoot,
            bevy::ui_widgets::ScrollArea,
            ScrollPosition::default(),
        ))
        .with_children(|root| {
            root.spawn(screen_title(&assets, "Settings"));
            root.spawn(blurb(
                &assets,
                "Display, readable UI scale, presentation, and volume.",
            ));
            root.spawn((
                button("Back"),
                SettingsBack,
                crate::UiVisibilityRequirement::Immediate,
            ))
            .with_child(label(&assets, "Back to title"));
            root.spawn((
                panel(),
                SettingsSurface,
                bevy::ui_widgets::ScrollArea,
                ScrollPosition::default(),
            ))
            .insert(Node {
                width: Val::Px(700.0),
                max_width: Val::Percent(94.0),
                max_height: Val::Percent(78.0),
                padding: UiRect::all(Val::Px(18.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                overflow: Overflow::scroll_y(),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            })
            .with_children(|surface| {
                spawn_settings_rows(surface, &assets, &view);
            });
            root.spawn((
                SettingNotice,
                blurb(&assets, view.notice.clone().unwrap_or_default()),
            ));
        });
}

fn apply_settings_layout(
    metrics: Res<ResolvedUiMetrics>,
    added: Query<(), Added<SettingsSurface>>,
    mut roots: Query<&mut Node, (With<SettingsRoot>, Without<SettingsSurface>)>,
    mut surfaces: Query<&mut Node, (With<SettingsSurface>, Without<SettingsRoot>)>,
    mut controls: Query<
        &mut Node,
        (
            With<SettingControl>,
            Without<SettingsRoot>,
            Without<SettingsSurface>,
            Without<SettingsBack>,
        ),
    >,
    mut backs: Query<
        &mut Node,
        (
            With<SettingsBack>,
            Without<SettingControl>,
            Without<SettingsRoot>,
            Without<SettingsSurface>,
        ),
    >,
) {
    if !metrics.is_changed() && added.is_empty() {
        return;
    }
    let compact = metrics.viewport == UiViewportClass::Compact;
    for mut node in &mut roots {
        node.overflow = if compact {
            Overflow::scroll_y()
        } else {
            Overflow::clip_y()
        };
    }
    for mut node in &mut surfaces {
        node.flex_shrink = if compact { 0.0 } else { 1.0 };
        node.max_height = if compact {
            Val::Auto
        } else {
            Val::Percent(78.0)
        };
        node.overflow = if compact {
            Overflow::visible()
        } else {
            Overflow::scroll_y()
        };
    }
    for mut node in &mut controls {
        node.width = Val::Percent(100.0);
        node.max_width = Val::Px(440.0);
        node.min_width = Val::Px(0.0);
        node.min_height = Val::Px(64.0 * metrics.content_scale.max(1.0));
        node.flex_shrink = 0.0;
    }
    for mut node in &mut backs {
        node.position_type = PositionType::Absolute;
        node.top = Val::Px(12.0);
        node.right = Val::Px(12.0);
        node.width = Val::Px(240.0);
        node.max_width = Val::Percent(40.0);
        node.min_width = Val::Px(0.0);
    }
}

fn refresh_settings(
    view: Res<UiSettingsView>,
    assets: Res<UiAssets>,
    mut commands: Commands,
    surfaces: Query<Entity, With<SettingsSurface>>,
    controls: Query<(&SettingControl, &Children)>,
    mut labels: Query<&mut Text, Without<SettingNotice>>,
    mut notices: Query<&mut Text, (With<SettingNotice>, Without<SettingControl>)>,
) {
    if !view.is_changed() {
        return;
    }
    if controls.iter().count() != view.rows.len() {
        for surface in &surfaces {
            commands.entity(surface).despawn_related::<Children>();
            commands
                .entity(surface)
                .with_children(|surface| spawn_settings_rows(surface, &assets, &view));
        }
    }
    for (control, children) in &controls {
        let Some(row) = view.rows.iter().find(|row| row.setting == control.0) else {
            continue;
        };
        if let Some(child) = children.first() {
            if let Ok(mut text) = labels.get_mut(*child) {
                text.0 = format!("{} · {}", row.label, row.value);
            }
        }
    }
    for mut notice in &mut notices {
        notice.0 = view.notice.clone().unwrap_or_default();
    }
}

fn spawn_settings_rows(
    surface: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    view: &UiSettingsView,
) {
    surface.spawn(heading(assets, "Display and audio"));
    for row in &view.rows {
        surface
            .spawn((
                button(format!("Setting {:?}", row.setting)),
                SettingControl(row.setting),
            ))
            .with_child(label(assets, format!("{} · {}", row.label, row.value)));
    }
}

fn handle_settings_controls(
    controls: Query<(&Interaction, &SettingControl), Changed<Interaction>>,
    back: Query<&Interaction, (Changed<Interaction>, With<SettingsBack>)>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::AdjustSetting(control.0));
        }
    }
    if back
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        intents.write(UiIntent::Back);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_entities_are_owned_by_their_screen() {
        let mut world = World::new();
        let entity = world
            .spawn(screen_root(Screen::Loading, "Loading Screen"))
            .id();
        assert_eq!(
            world
                .entity(entity)
                .get::<crate::DespawnOnExit>()
                .map(|tag| tag.0),
            Some(Screen::Loading)
        );
    }
}
