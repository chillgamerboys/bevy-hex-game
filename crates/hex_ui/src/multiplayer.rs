//! Direct Connect, six-seat lobby, and remote-client local-menu presentation.

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::ScrollArea;
use hex_core::{PlayerSeat, Screen};
use hex_gameplay_model::{MultiplayerRole, MultiplayerRoute};

use crate::{
    blurb, body_text_role, button, despawn_screen, fine, heading, label, overlay_root, panel,
    responsive_control_role, screen_root, screen_title, DespawnOnExit, MultiplayerIntent,
    MultiplayerSeatConnectionView, MultiplayerTextField, MultiplayerView, SensitiveText, UiAssets,
    UiIntent, UiSystems, UiVisibilityRequirement, ACCENT_EDGE, LABEL,
};

const CONNECTION_CODE_CHAR_LIMIT: usize = 2_048;
const HOST_CHAR_LIMIT: usize = 253;
const PORT_CHAR_LIMIT: usize = 5;

#[derive(Component)]
struct MultiplayerSurface;

#[derive(Component)]
struct MultiplayerLocalMenu;

#[derive(Component, Clone)]
struct MultiplayerControl(MultiplayerIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Multiplayer), spawn_screen)
        .add_systems(
            Update,
            (
                refresh_screen.in_set(UiSystems::Render),
                emit_controls,
                emit_text_changes,
            )
                .run_if(in_state(Screen::Multiplayer)),
        )
        .add_systems(
            OnExit(Screen::Multiplayer),
            despawn_screen(Screen::Multiplayer),
        )
        .add_systems(OnEnter(Screen::Gameplay), spawn_local_menu)
        .add_systems(
            Update,
            (refresh_local_menu.in_set(UiSystems::Render), emit_controls)
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn spawn_screen(mut commands: Commands, assets: Res<UiAssets>, view: Res<MultiplayerView>) {
    commands
        .spawn((
            screen_root(Screen::Multiplayer, "Multiplayer"),
            MultiplayerSurface,
            ScrollArea,
            ScrollPosition::default(),
        ))
        .insert(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            padding: UiRect::all(Val::Px(28.0)),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::FlexStart,
            row_gap: Val::Px(16.0),
            overflow: Overflow::scroll_y(),
            ..default()
        })
        .with_children(|root| render_screen(root, &assets, &view));
}

fn refresh_screen(
    mut commands: Commands,
    view: Res<MultiplayerView>,
    assets: Res<UiAssets>,
    roots: Query<Entity, With<MultiplayerSurface>>,
) {
    if !view.is_changed() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn_related::<Children>();
        commands
            .entity(root)
            .with_children(|root| render_screen(root, &assets, &view));
    }
}

fn render_screen(root: &mut ChildSpawnerCommands, assets: &UiAssets, view: &MultiplayerView) {
    root.spawn((
        screen_title(assets, route_title(view.route)),
        crate::UiTextMustFit,
    ));
    if let Some(notice) = &view.notice {
        root.spawn((
            Name::new("Multiplayer Notice"),
            blurb(assets, notice.clone()),
        ));
    }

    match view.route {
        MultiplayerRoute::Home => render_home(root, assets),
        MultiplayerRoute::HostDirect => render_host_direct(root, assets, view),
        MultiplayerRoute::JoinDirect => render_join_direct(root, assets, view),
        MultiplayerRoute::Connecting => render_waiting(
            root,
            assets,
            view.role,
            "Connecting",
            "Opening the encrypted, certificate-pinned direct session and waiting for admission.",
        ),
        MultiplayerRoute::Lobby => render_lobby(root, assets, view),
        MultiplayerRoute::Loading => render_waiting(
            root,
            assets,
            view.role,
            "Verifying World",
            "Every peer is generating the frozen shipped map and comparing the complete public-world fingerprint.",
        ),
        MultiplayerRoute::Reconnecting => render_waiting(
            root,
            assets,
            view.role,
            "Reconnecting",
            "Your reserved seat is being reclaimed at a safe authority boundary. Local camera and selection will be restored locally.",
        ),
        MultiplayerRoute::Ended => render_ended(root, assets, view),
    }
}

fn route_title(route: MultiplayerRoute) -> &'static str {
    match route {
        MultiplayerRoute::Home => "Hex / Multiplayer",
        MultiplayerRoute::HostDirect => "Hex / Host Direct",
        MultiplayerRoute::JoinDirect => "Hex / Join Direct",
        MultiplayerRoute::Connecting => "Hex / Connecting",
        MultiplayerRoute::Lobby => "Hex / Session Lobby",
        MultiplayerRoute::Loading => "Hex / World Verification",
        MultiplayerRoute::Reconnecting => "Hex / Reconnecting",
        MultiplayerRoute::Ended => "Hex / Session Ended",
    }
}

fn render_home(root: &mut ChildSpawnerCommands, assets: &UiAssets) {
    root.spawn(blurb(
        assets,
        "Create or join a server-authoritative, client-hosted session for up to six human players.",
    ));
    root.spawn((Name::new("Multiplayer Home Actions"), panel()))
        .insert(action_panel_node(520.0))
        .with_children(|actions| {
            action_button(
                actions,
                assets,
                "Host Direct",
                MultiplayerIntent::OpenHostDirect,
                true,
            );
            action_button(
                actions,
                assets,
                "Join Direct",
                MultiplayerIntent::OpenJoinDirect,
                true,
            );
            action_button(actions, assets, "Back", MultiplayerIntent::Back, true);
        });
    root.spawn(fine(
        assets,
        "Steam invites and relay traversal will use this same game protocol in a later milestone.",
    ));
}

fn render_host_direct(root: &mut ChildSpawnerCommands, assets: &UiAssets, view: &MultiplayerView) {
    root.spawn(blurb(
        assets,
        "Hosting binds an encrypted WebTransport listener on the selected UDP port. Internet guests normally need that UDP port forwarded to this computer.",
    ));
    root.spawn((Name::new("Direct Host Setup"), panel()))
        .insert(action_panel_node(680.0))
        .with_children(|form| {
            form.spawn(heading(assets, "Advertised endpoint"));
            form.spawn(fine(
                assets,
                "Use a reachable public hostname/IP for Internet play, or a LAN address for local play.",
            ));
            text_field(
                form,
                assets,
                "Advertised Host",
                &view.advertised_host,
                MultiplayerTextField::AdvertisedHost,
                HOST_CHAR_LIMIT,
            );
            text_field(
                form,
                assets,
                "UDP Port",
                &view.advertised_port,
                MultiplayerTextField::AdvertisedPort,
                PORT_CHAR_LIMIT,
            );
            action_button(
                form,
                assets,
                "Configure Shipped Sandbox",
                MultiplayerIntent::ConfigureSandbox,
                true,
            );
            if let Some(summary) = &view.launch_summary {
                form.spawn((Name::new("Frozen Session Summary"), label(assets, summary.clone())));
            }
            if let Some(code) = &view.share_code {
                form.spawn(heading(assets, "Direct connection code"));
                form.spawn((
                    Name::new("Direct Connection Code"),
                    AccessibleLabel::new("Direct connection code, share only with invited players"),
                    fine(assets, code.expose().to_owned()),
                ));
                form.spawn(fine(
                    assets,
                    "The code contains a one-time private invite. Share it only with players joining this lobby.",
                ));
            }
            action_button(form, assets, "Back", MultiplayerIntent::Back, true);
        });
    render_network_limits(root, assets, Some(&view.advertised_port));
}

fn render_join_direct(root: &mut ChildSpawnerCommands, assets: &UiAssets, view: &MultiplayerView) {
    root.spawn(blurb(
        assets,
        "Paste the complete private HEX1 connection code supplied by the host. The code pins the host certificate before admission.",
    ));
    root.spawn((Name::new("Direct Join Setup"), panel()))
        .insert(action_panel_node(760.0))
        .with_children(|form| {
            text_field(
                form,
                assets,
                "HEX1 Connection Code",
                view.join_code.expose(),
                MultiplayerTextField::JoinCode,
                CONNECTION_CODE_CHAR_LIMIT,
            );
            action_button(
                form,
                assets,
                "Join Session",
                MultiplayerIntent::JoinDirect,
                !view.join_code.is_empty(),
            );
            if view.reconnect_available {
                action_button(
                    form,
                    assets,
                    "Reconnect Reserved Seat",
                    MultiplayerIntent::ReconnectDirect,
                    true,
                );
                form.spawn(fine(
                    assets,
                    "Reconnect uses this app's private rotating credential and its persisted pinned host endpoint. The old invite code is not required.",
                ));
            }
            action_button(form, assets, "Back", MultiplayerIntent::Back, true);
        });
    render_network_limits(root, assets, None);
}

fn render_network_limits(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    advertised_port: Option<&str>,
) {
    root.spawn((Name::new("Direct Connect Limitations"), panel()))
        .insert(action_panel_node(760.0))
        .with_children(|help| {
            help.spawn(heading(assets, "Direct Internet requirements"));
            help.spawn(fine(assets, network_forwarding_copy(advertised_port)));
            help.spawn(fine(
                assets,
                "This build has no UPnP, public join-code service, STUN/TURN, or non-Steam relay. Steam relay traversal comes later; LAN and manually forwarded Internet connections work without Steam.",
            ));
        });
}

fn network_forwarding_copy(advertised_port: Option<&str>) -> String {
    let instruction = advertised_port.map_or_else(
        || "Forward the host-selected UDP port to the host computer.".to_owned(),
        |port| format!("Forward UDP {port} to the host computer."),
    );
    format!(
        "{instruction} Carrier-grade NAT (CGNAT) or restrictive networks may make direct hosting impossible."
    )
}

fn render_waiting(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    role: Option<MultiplayerRole>,
    heading_text: &str,
    detail: &str,
) {
    let (action_label, action) = waiting_action(role);
    root.spawn((Name::new(heading_text.to_owned()), panel()))
        .insert(action_panel_node(680.0))
        .with_children(|status| {
            status.spawn(heading(assets, heading_text));
            status.spawn(blurb(assets, detail));
            action_button(status, assets, action_label, action, true);
        });
}

fn waiting_action(role: Option<MultiplayerRole>) -> (&'static str, MultiplayerIntent) {
    if role == Some(MultiplayerRole::Host) {
        ("Close Session", MultiplayerIntent::CloseSession)
    } else {
        ("Leave Session", MultiplayerIntent::LeaveSession)
    }
}

fn render_lobby(root: &mut ChildSpawnerCommands, assets: &UiAssets, view: &MultiplayerView) {
    let host = view.role == Some(MultiplayerRole::Host);
    if let Some(summary) = &view.launch_summary {
        root.spawn((
            Name::new("Lobby Launch Summary"),
            blurb(assets, summary.clone()),
        ));
    }
    if host {
        if let Some(code) = &view.share_code {
            root.spawn((
                Name::new("Lobby Direct Connection Code"),
                AccessibleLabel::new("Direct connection code, share only with invited players"),
                fine(assets, code.expose().to_owned()),
            ));
        }
    }

    root.spawn((
        Name::new("Six Seat Lobby"),
        Node {
            width: Val::Percent(100.0),
            max_width: Val::Px(1_420.0),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Stretch,
            column_gap: Val::Px(14.0),
            row_gap: Val::Px(14.0),
            ..default()
        },
    ))
    .with_children(|deck| {
        for seat in &view.seats {
            deck.spawn((
                Name::new(format!("Lobby Seat {}", seat.seat.0 + 1)),
                panel(),
            ))
            .insert(Node {
                width: Val::Px(430.0),
                min_height: Val::Px(250.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                row_gap: Val::Px(7.0),
                ..crate::panel_node()
            })
            .with_children(|card| {
                let mut title = format!("Seat {}", seat.seat.0 + 1);
                if seat.seat == PlayerSeat::HOST {
                    title.push_str(" · HOST");
                }
                if seat.local {
                    title.push_str(" · YOU");
                }
                card.spawn(heading(assets, title));
                card.spawn(label(
                    assets,
                    seat.player_label
                        .clone()
                        .unwrap_or_else(|| "Open seat".to_owned()),
                ));
                card.spawn(fine(assets, seat_connection_label(seat.connection)));
                if seat.assignments.is_empty() {
                    card.spawn(fine(assets, "No party member assigned"));
                }
                for assignment in &seat.assignments {
                    card.spawn(label(assets, assignment.label.clone()));
                    if host {
                        card.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            flex_wrap: FlexWrap::Wrap,
                            column_gap: Val::Px(5.0),
                            row_gap: Val::Px(5.0),
                            ..default()
                        })
                        .with_children(|moves| {
                            for destination in view.seats.iter().filter(|destination| {
                                destination.seat != seat.seat
                                    && !matches!(
                                        destination.connection,
                                        MultiplayerSeatConnectionView::Vacant
                                    )
                            }) {
                                action_button(
                                    moves,
                                    assets,
                                    &format!("Move to {}", destination.seat.0 + 1),
                                    MultiplayerIntent::AssignUnit {
                                        unit: assignment.unit,
                                        destination: destination.seat,
                                    },
                                    true,
                                );
                            }
                        });
                    }
                }
                if seat.seat == PlayerSeat::HOST {
                    card.spawn(fine(assets, "Host readiness is implicit"));
                } else {
                    card.spawn(fine(assets, if seat.ready { "READY" } else { "NOT READY" }));
                }
                if seat.local && !host && seat.seat != PlayerSeat::HOST {
                    action_button(
                        card,
                        assets,
                        if seat.ready { "Not Ready" } else { "Ready" },
                        MultiplayerIntent::SetReady(!seat.ready),
                        true,
                    );
                }
                if host
                    && seat.seat != PlayerSeat::HOST
                    && !matches!(seat.connection, MultiplayerSeatConnectionView::Vacant)
                {
                    action_button(
                        card,
                        assets,
                        "Kick",
                        MultiplayerIntent::Kick(seat.seat),
                        true,
                    );
                }
            });
        }
    });

    root.spawn((Name::new("Lobby Actions"), panel()))
        .insert(action_panel_node(680.0))
        .with_children(|actions| {
            if host {
                action_button(
                    actions,
                    assets,
                    "Launch",
                    MultiplayerIntent::Launch,
                    view.can_launch,
                );
                if let Some(blocker) = &view.launch_blocker {
                    actions.spawn((Name::new("Launch Blocker"), fine(assets, blocker.clone())));
                }
                action_button(
                    actions,
                    assets,
                    "Close Session",
                    MultiplayerIntent::CloseSession,
                    true,
                );
            } else {
                action_button(
                    actions,
                    assets,
                    "Leave Session",
                    MultiplayerIntent::LeaveSession,
                    true,
                );
            }
        });
}

fn seat_connection_label(connection: MultiplayerSeatConnectionView) -> String {
    match connection {
        MultiplayerSeatConnectionView::Vacant => "VACANT".to_owned(),
        MultiplayerSeatConnectionView::Connected => "CONNECTED".to_owned(),
        MultiplayerSeatConnectionView::Reserved { seconds } => {
            format!("DISCONNECTED · RESERVED {seconds}s")
        }
        MultiplayerSeatConnectionView::Delegated => {
            "DISCONNECTED · TEMPORARY HOST DELEGATION".to_owned()
        }
        MultiplayerSeatConnectionView::ReclaimPending => {
            "RECONNECTED · WAITING FOR SAFE BOUNDARY".to_owned()
        }
    }
}

fn render_ended(root: &mut ChildSpawnerCommands, assets: &UiAssets, view: &MultiplayerView) {
    root.spawn((Name::new("Session Ended Panel"), panel()))
        .insert(action_panel_node(680.0))
        .with_children(|ended| {
            ended.spawn(heading(assets, "Session ended"));
            ended.spawn(blurb(
                assets,
                view.notice
                    .clone()
                    .unwrap_or_else(|| "The multiplayer session has ended.".to_owned()),
            ));
            action_button(
                ended,
                assets,
                "Multiplayer Home",
                MultiplayerIntent::Back,
                true,
            );
        });
}

fn action_panel_node(width: f32) -> Node {
    Node {
        width: Val::Px(width),
        max_width: Val::Percent(96.0),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Stretch,
        row_gap: Val::Px(10.0),
        ..crate::panel_node()
    }
}

fn action_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: &str,
    intent: MultiplayerIntent,
    enabled: bool,
) {
    let mut control = parent.spawn((
        button(text.to_owned()),
        MultiplayerControl(intent),
        UiVisibilityRequirement::Immediate,
    ));
    if !enabled {
        control.insert(InteractionDisabled);
    }
    control.with_child(label(assets, text.to_owned()));
}

fn text_field(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    accessible: &'static str,
    value: &str,
    field: MultiplayerTextField,
    max_characters: usize,
) {
    parent.spawn((
        Name::new(accessible),
        AccessibleLabel::new(accessible),
        TabIndex(0),
        crate::DefaultImmediateControl,
        EditableText {
            max_characters: Some(max_characters),
            visible_width: Some(48.0),
            ..EditableText::new(value)
        },
        body_text_role(),
        responsive_control_role(),
        TextFont {
            font: assets.body.clone().into(),
            ..TextFont::from_font_size(18.0)
        },
        TextColor(LABEL),
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
        BorderColor::all(ACCENT_EDGE),
        Node {
            width: Val::Percent(100.0),
            min_height: Val::Px(44.0),
            flex_shrink: 0.0,
            padding: UiRect::all(Val::Px(9.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        field,
    ));
}

fn emit_controls(
    controls: Query<(&Interaction, &MultiplayerControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Multiplayer(control.0.clone()));
        }
    }
}

fn emit_text_changes(
    fields: Query<(&EditableText, &MultiplayerTextField), Changed<EditableText>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (value, field) in &fields {
        intents.write(UiIntent::Multiplayer(MultiplayerIntent::SetText(
            *field,
            SensitiveText::new(value.value().to_string()),
        )));
    }
}

fn spawn_local_menu(mut commands: Commands) {
    commands.spawn((
        overlay_root("Client Local Menu"),
        MultiplayerLocalMenu,
        DespawnOnExit(Screen::Gameplay),
        Visibility::Hidden,
    ));
}

fn refresh_local_menu(
    mut commands: Commands,
    view: Res<MultiplayerView>,
    assets: Res<UiAssets>,
    roots: Query<Entity, With<MultiplayerLocalMenu>>,
) {
    if !view.is_changed() {
        return;
    }
    let visible = view.role == Some(MultiplayerRole::Client) && view.local_menu_open;
    for root in &roots {
        commands.entity(root).despawn_related::<Children>();
        commands.entity(root).insert(if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !visible {
            continue;
        }
        commands.entity(root).with_children(|overlay| {
            overlay
                .spawn((Name::new("Client Local Menu Panel"), panel()))
                .insert(action_panel_node(460.0))
                .with_children(|menu| {
                    menu.spawn(heading(&assets, "Local Menu"));
                    menu.spawn(blurb(
                        &assets,
                        "The host simulation continues. This menu does not pause the encounter.",
                    ));
                    action_button(
                        menu,
                        &assets,
                        "Resume",
                        MultiplayerIntent::ResumeLocal,
                        true,
                    );
                    action_button(
                        menu,
                        &assets,
                        "Leave Session",
                        MultiplayerIntent::LeaveSession,
                        true,
                    );
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_copy_distinguishes_all_reservation_states() {
        assert_eq!(
            seat_connection_label(MultiplayerSeatConnectionView::Reserved { seconds: 17 }),
            "DISCONNECTED · RESERVED 17s"
        );
        assert!(
            seat_connection_label(MultiplayerSeatConnectionView::Delegated)
                .contains("HOST DELEGATION")
        );
        assert!(
            seat_connection_label(MultiplayerSeatConnectionView::ReclaimPending)
                .contains("SAFE BOUNDARY")
        );
    }

    #[test]
    fn route_titles_cover_the_complete_multiplayer_model() {
        for route in [
            MultiplayerRoute::Home,
            MultiplayerRoute::HostDirect,
            MultiplayerRoute::JoinDirect,
            MultiplayerRoute::Connecting,
            MultiplayerRoute::Lobby,
            MultiplayerRoute::Loading,
            MultiplayerRoute::Reconnecting,
            MultiplayerRoute::Ended,
        ] {
            assert!(!route_title(route).is_empty());
        }
    }

    #[test]
    fn waiting_states_never_offer_the_host_an_action_that_authority_refuses() {
        assert_eq!(
            waiting_action(Some(MultiplayerRole::Host)),
            ("Close Session", MultiplayerIntent::CloseSession)
        );
        assert_eq!(
            waiting_action(Some(MultiplayerRole::Client)),
            ("Leave Session", MultiplayerIntent::LeaveSession)
        );
    }

    #[test]
    fn direct_network_copy_names_the_numeric_or_host_selected_port() {
        assert_eq!(
            network_forwarding_copy(Some("7777")),
            "Forward UDP 7777 to the host computer. Carrier-grade NAT (CGNAT) or restrictive networks may make direct hosting impossible."
        );
        assert_eq!(
            network_forwarding_copy(None),
            "Forward the host-selected UDP port to the host computer. Carrier-grade NAT (CGNAT) or restrictive networks may make direct hosting impossible."
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn lobby_renders_six_stable_seats_and_client_only_local_menu_copy() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
        let mut view = MultiplayerView {
            route: MultiplayerRoute::Lobby,
            role: Some(MultiplayerRole::Client),
            local_seat: Some(PlayerSeat(1)),
            local_menu_open: true,
            ..Default::default()
        };
        let guest = view
            .seats
            .get_mut(1)
            .expect("six-seat fixture has seat two");
        guest.local = true;
        guest.connection = MultiplayerSeatConnectionView::Connected;
        app.world_mut().insert_resource(view);
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Multiplayer);
        for _ in 0..8 {
            app.update();
        }

        let seat_count = app
            .world_mut()
            .query::<&Name>()
            .iter(app.world())
            .filter(|name| name.as_str().starts_with("Lobby Seat "))
            .count();
        assert_eq!(seat_count, PlayerSeat::HUMAN_COUNT);

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..8 {
            app.update();
        }
        let menu = app
            .world_mut()
            .query::<(&Name, &Visibility)>()
            .iter(app.world())
            .find(|(name, _)| name.as_str() == "Client Local Menu")
            .map(|(_, visibility)| *visibility);
        assert_eq!(menu, Some(Visibility::Inherited));
    }
}
