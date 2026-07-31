use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, button, despawn_screen, display, heading, label, panel, screen_root, screen_title,
    PauseView, UiAssets, UiIntent, UiSetting, UiSettingsView,
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
struct PauseOverlay;

#[derive(Component)]
struct PauseHint;

#[derive(Component)]
struct PauseNotice;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Splash), spawn_splash)
        .add_systems(OnExit(Screen::Splash), despawn_screen(Screen::Splash))
        .add_systems(OnEnter(Screen::Loading), spawn_loading)
        .add_systems(OnExit(Screen::Loading), despawn_screen(Screen::Loading))
        .add_systems(OnEnter(Screen::Settings), spawn_settings)
        .add_systems(
            Update,
            (refresh_settings, handle_settings_controls).run_if(in_state(Screen::Settings)),
        )
        .add_systems(OnExit(Screen::Settings), despawn_screen(Screen::Settings));
    app.add_systems(OnEnter(hex_core::Pause(true)), spawn_pause)
        .add_systems(
            Update,
            refresh_pause.run_if(in_state(hex_core::Pause(true))),
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
        });
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
        .spawn(screen_root(Screen::Settings, "Settings Screen"))
        .with_children(|root| {
            root.spawn(screen_title(&assets, "Settings"));
            root.spawn(blurb(
                &assets,
                "Display, readable UI scale, presentation, and volume.",
            ));
            root.spawn((panel(), SettingsSurface))
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
    surface
        .spawn((button("Back"), SettingsBack))
        .with_child(label(assets, "Back to title"));
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
