//! Direct Connect, six-seat lobby, and remote-client local-menu presentation.

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::ScrollArea;
use hex_core::{PlayerSeat, Screen};
use hex_gameplay_model::{MultiplayerRole, MultiplayerRoute};

use crate::{
    blurb, body_text_role, button, despawn_screen, fine, fluid_button, heading, label,
    overlay_root, panel, responsive_control_role, screen_root, screen_title,
    CampaignSlotStatusView, DespawnOnExit, MultiplayerCampaignSaveStatusView, MultiplayerIntent,
    MultiplayerSeatConnectionView, MultiplayerTextField, MultiplayerView, SensitiveText, UiAssets,
    UiIntent, UiSystems, UiVisibilityRequirement, ACCENT_EDGE, LABEL, READ_ONLY_HUD,
};

const CONNECTION_CODE_CHAR_LIMIT: usize = 2_048;
const HOST_CHAR_LIMIT: usize = 253;
const PORT_CHAR_LIMIT: usize = 5;

#[derive(Component)]
struct MultiplayerSurface;

#[derive(Component)]
struct MultiplayerLocalMenu;

#[derive(Component)]
struct MultiplayerCampaignStatus;

#[derive(Component, Clone)]
struct MultiplayerControl(MultiplayerIntent);

#[derive(Clone, Copy, PartialEq, Eq)]
struct MultiplayerPageIdentity {
    route: MultiplayerRoute,
    campaign_refusal: bool,
    role: Option<MultiplayerRole>,
}

#[derive(Default)]
struct MultiplayerPageScrollReset {
    page: Option<MultiplayerPageIdentity>,
    frames_remaining: u8,
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Multiplayer), spawn_screen)
        .add_systems(
            Update,
            (
                refresh_screen.in_set(UiSystems::Render),
                emit_controls.in_set(UiSystems::EmitIntents),
                emit_text_changes.in_set(UiSystems::EmitIntents),
            )
                .run_if(in_state(Screen::Multiplayer)),
        )
        .add_systems(
            OnExit(Screen::Multiplayer),
            despawn_screen(Screen::Multiplayer),
        )
        .add_systems(
            Last,
            finish_page_change_scroll_reset.run_if(in_state(Screen::Multiplayer)),
        )
        .add_systems(OnEnter(Screen::Gameplay), spawn_local_menu)
        .add_systems(
            Update,
            (
                refresh_local_menu.in_set(UiSystems::Render),
                refresh_campaign_save_status.in_set(UiSystems::Render),
                emit_controls.in_set(UiSystems::EmitIntents),
            )
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn spawn_screen(
    mut commands: Commands,
    assets: Res<UiAssets>,
    view: Res<MultiplayerView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
) {
    let view = effective_view(&view, review.as_deref());
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
        .with_children(|root| render_screen(root, &assets, view));
}

fn refresh_screen(
    mut commands: Commands,
    view: Res<MultiplayerView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    assets: Res<UiAssets>,
    mut roots: Query<(Entity, &mut ScrollPosition), With<MultiplayerSurface>>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    mut focus: ResMut<InputFocus>,
    mut focus_refreshes: ResMut<crate::focus::FocusRefreshRequests>,
    mut previous_route: Local<Option<MultiplayerRoute>>,
) {
    if !view.is_changed() && review.as_ref().is_none_or(|review| !review.is_changed()) {
        return;
    }
    let view = effective_view(&view, review.as_deref());
    let route_changed = *previous_route != Some(view.route);
    for (root, mut scroll_position) in &mut roots {
        if route_changed {
            crate::focus::begin_route_refresh(
                root,
                &mut focus,
                &parents,
                &names,
                &mut focus_refreshes,
            );
            *scroll_position = ScrollPosition::default();
        }
        commands.entity(root).despawn_related::<Children>();
        commands
            .entity(root)
            .with_children(|root| render_screen(root, &assets, view));
    }
    *previous_route = Some(view.route);
}

/// Applies the page reset after focus restoration and Bevy's scroll-into-view pass.
///
/// Route activation and a newly surfaced Campaign refusal are both page changes.
/// Running this in `Last` across the replacement-layout frames prevents the
/// control that initiated a route transition from carrying its deferred
/// scroll-into-view adjustment into the replacement page.
fn finish_page_change_scroll_reset(
    view: Res<MultiplayerView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    mut roots: Query<&mut ScrollPosition, With<MultiplayerSurface>>,
    mut reset: Local<MultiplayerPageScrollReset>,
) {
    let view = effective_view(&view, review.as_deref());
    let page = MultiplayerPageIdentity {
        route: view.route,
        campaign_refusal: view.campaign_host.refusal.is_some(),
        role: view.role,
    };
    if reset.page != Some(page) {
        reset.page = Some(page);
        reset.frames_remaining = 4;
    }
    if reset.frames_remaining > 0 {
        for mut scroll_position in &mut roots {
            *scroll_position = ScrollPosition::default();
        }
        reset.frames_remaining -= 1;
    }
}

fn effective_view<'a>(
    live: &'a MultiplayerView,
    review: Option<&'a crate::review::UiReviewPresentation>,
) -> &'a MultiplayerView {
    review
        .and_then(|review| review.multiplayer.as_ref())
        .unwrap_or(live)
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
        MultiplayerRoute::HostCampaign => render_host_campaign(root, assets, view),
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
            if view.campaign_session {
                "Importing the host's complete Campaign baseline, then verifying the exact public-world fingerprint before activation."
            } else {
                "Every peer is generating the frozen shipped map and comparing the complete public-world fingerprint."
            },
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
        MultiplayerRoute::HostCampaign => "Hex / Host Campaign",
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
            actions.spawn(heading(assets, "Host a Campaign"));
            action_button(
                actions,
                assets,
                "Host Campaign",
                MultiplayerIntent::OpenHostCampaign,
                true,
            );
            actions.spawn(heading(assets, "Advanced · Direct/LAN"));
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
        "Direct/LAN remains available without an online service. Internet play currently requires a reachable forwarded UDP port.",
    ));
}

fn render_host_campaign(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    view: &MultiplayerView,
) {
    root.spawn(blurb(
        assets,
        "Choose a host-owned Campaign slot. Empty slots begin a new Campaign; occupied slots restore the complete checkpoint before opening a fresh assignment lobby.",
    ));
    root.spawn((Name::new("Campaign Direct Endpoint"), panel()))
        .insert(action_panel_node(760.0))
        .with_children(|endpoint| {
            endpoint.spawn(heading(assets, "Direct/LAN endpoint"));
            endpoint.spawn(fine(
                assets,
                "Resuming never restores old seats or credentials. Every session gets a fresh invite and fresh party assignments.",
            ));
            text_field(
                endpoint,
                assets,
                "Advertised Host",
                &view.advertised_host,
                MultiplayerTextField::AdvertisedHost,
                HOST_CHAR_LIMIT,
            );
            text_field(
                endpoint,
                assets,
                "UDP Port",
                &view.advertised_port,
                MultiplayerTextField::AdvertisedPort,
                PORT_CHAR_LIMIT,
            );
        });

    if let Some(refusal) = &view.campaign_host.refusal {
        root.spawn((Name::new("Campaign Host Refusal"), panel()))
            .insert(action_panel_node(760.0))
            .with_children(|panel| {
                panel.spawn(heading(assets, "Campaign could not be hosted"));
                panel.spawn(blurb(assets, refusal.clone()));
            });
    }

    root.spawn((
        Name::new("Multiplayer Campaign Slots"),
        Node {
            width: Val::Percent(100.0),
            max_width: Val::Px(1_320.0),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Stretch,
            column_gap: Val::Px(18.0),
            row_gap: Val::Px(18.0),
            ..default()
        },
    ))
    .with_children(|cards| {
        for slot in &view.campaign_slots {
            cards
                .spawn((
                    Name::new(format!("Multiplayer Campaign Slot {}", slot.slot.number())),
                    panel(),
                ))
                .insert(Node {
                    width: Val::Px(390.0),
                    min_width: Val::Px(280.0),
                    min_height: Val::Px(300.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
                    row_gap: Val::Px(10.0),
                    ..crate::panel_node()
                })
                .with_children(|card| {
                    card.spawn(heading(
                        assets,
                        format!("Campaign Slot {}", slot.slot.number()),
                    ));
                    match &slot.status {
                        CampaignSlotStatusView::Empty => {
                            card.spawn(blurb(assets, "Empty Campaign slot"));
                            scrollable_card_action_button(
                                card,
                                assets,
                                &format!("Host New Campaign Slot {}", slot.slot.number()),
                                MultiplayerIntent::HostCampaign(slot.slot),
                                !view.campaign_host.preparing,
                            );
                        }
                        CampaignSlotStatusView::Available { party, active_time } => {
                            card.spawn(label(assets, format!("Active gameplay · {active_time}")));
                            for member in party {
                                card.spawn(label(assets, member.name.clone()));
                                card.spawn(fine(assets, member.lattice.clone()));
                            }
                            scrollable_card_action_button(
                                card,
                                assets,
                                &format!("Resume Campaign Slot {}", slot.slot.number()),
                                MultiplayerIntent::HostCampaign(slot.slot),
                                !view.campaign_host.preparing,
                            );
                        }
                        CampaignSlotStatusView::Invalid { reason } => {
                            card.spawn(blurb(assets, "Campaign unavailable"));
                            card.spawn(fine(assets, reason.clone()));
                        }
                    }
                    if view.campaign_host.preparing && view.campaign_host.slot == Some(slot.slot) {
                        card.spawn(fine(
                            assets,
                            "Restoring and validating the complete checkpoint…",
                        ));
                    }
                });
        }
    });

    root.spawn((Name::new("Campaign Host Actions"), panel()))
        .insert(action_panel_node(760.0))
        .with_children(|actions| {
            action_button(actions, assets, "Back", MultiplayerIntent::Back, true);
        });
    root.spawn(fine(
        assets,
        network_forwarding_copy(Some(&view.advertised_port)),
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
                render_connection_code(form, assets, "Direct Connection Code", code);
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
    if view.campaign_session {
        root.spawn((Name::new("Fresh Campaign Assignment"), panel()))
            .insert(action_panel_node(760.0))
            .with_children(|campaign| {
                campaign.spawn(heading(assets, "Fresh Campaign assignment"));
                campaign.spawn(fine(
                    assets,
                    "The host restored the Campaign checkpoint. Seats, readiness, reconnect credentials, cameras, and selections are new for this session.",
                ));
            });
    }
    if let Some(summary) = &view.launch_summary {
        root.spawn((
            Name::new("Lobby Launch Summary"),
            blurb(assets, summary.clone()),
        ));
    }
    if host {
        if let Some(code) = &view.share_code {
            root.spawn((Name::new("Lobby Direct Invite"), panel()))
                .insert(action_panel_node(680.0))
                .with_children(|invite| {
                    render_connection_code(invite, assets, "Lobby Direct Connection Code", code);
                });
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
                                scrollable_card_action_button(
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
                    scrollable_card_action_button(
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
                    scrollable_card_action_button(
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
                scrollable_action_button(
                    actions,
                    assets,
                    "Launch",
                    MultiplayerIntent::Launch,
                    view.can_launch,
                );
                if let Some(blocker) = &view.launch_blocker {
                    actions.spawn((Name::new("Launch Blocker"), fine(assets, blocker.clone())));
                }
                scrollable_action_button(
                    actions,
                    assets,
                    "Close Session",
                    MultiplayerIntent::CloseSession,
                    true,
                );
            } else {
                scrollable_action_button(
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

fn render_connection_code(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    code_name: &'static str,
    code: &SensitiveText,
) {
    parent.spawn(heading(assets, "Direct connection code"));
    parent.spawn((
        Name::new(code_name),
        AccessibleLabel::new("Direct connection code, share only with invited players"),
        fine(assets, code.expose().to_owned()),
    ));
    action_button(
        parent,
        assets,
        "Copy Connection Code",
        MultiplayerIntent::CopyConnectionCode,
        true,
    );
    parent.spawn(fine(
        assets,
        "The code contains a one-time private invite. Share it only with players joining this lobby.",
    ));
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

fn scrollable_action_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: &str,
    intent: MultiplayerIntent,
    enabled: bool,
) {
    let mut control = parent.spawn((
        button(text.to_owned()),
        MultiplayerControl(intent),
        UiVisibilityRequirement::Scrollable,
    ));
    if !enabled {
        control.insert(InteractionDisabled);
    }
    control.with_child(label(assets, text.to_owned()));
}

fn scrollable_card_action_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: &str,
    intent: MultiplayerIntent,
    enabled: bool,
) {
    let mut control = parent.spawn((
        fluid_button(text.to_owned()),
        MultiplayerControl(intent),
        UiVisibilityRequirement::Scrollable,
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
    commands.spawn((
        Name::new("Campaign Save Status"),
        MultiplayerCampaignStatus,
        DespawnOnExit(Screen::Gameplay),
        READ_ONLY_HUD,
        Visibility::Hidden,
        GlobalZIndex(9),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(28.0),
            left: Val::Percent(20.0),
            width: Val::Percent(60.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
    ));
}

fn refresh_campaign_save_status(
    mut commands: Commands,
    view: Res<MultiplayerView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    assets: Res<UiAssets>,
    roots: Query<Entity, With<MultiplayerCampaignStatus>>,
) {
    if !view.is_changed() && review.as_ref().is_none_or(|review| !review.is_changed()) {
        return;
    }
    let view = effective_view(&view, review.as_deref());
    let status = (view.role == Some(MultiplayerRole::Client))
        .then_some(view.campaign_save_status.as_ref())
        .flatten();
    for root in &roots {
        commands.entity(root).despawn_related::<Children>();
        commands.entity(root).insert(if status.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        let Some(status) = status else {
            continue;
        };
        commands.entity(root).with_children(|status_root| {
            status_root
                .spawn((Name::new("Campaign Save Status Panel"), panel()))
                .insert(action_panel_node(620.0))
                .with_children(|status_panel| {
                    status_panel.spawn(heading(&assets, "Campaign Save"));
                    status_panel.spawn(blurb(&assets, campaign_save_status_copy(status)));
                });
        });
    }
}

fn campaign_save_status_copy(status: &MultiplayerCampaignSaveStatusView) -> String {
    match status {
        MultiplayerCampaignSaveStatusView::Saving => {
            "The host is saving the complete Campaign checkpoint…".to_owned()
        }
        MultiplayerCampaignSaveStatusView::Saved => {
            "The host saved the Campaign checkpoint.".to_owned()
        }
        MultiplayerCampaignSaveStatusView::Refused { reason } => {
            format!("The host could not save the Campaign: {reason}")
        }
    }
}

fn refresh_local_menu(
    mut commands: Commands,
    view: Res<MultiplayerView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    assets: Res<UiAssets>,
    roots: Query<Entity, With<MultiplayerLocalMenu>>,
) {
    if !view.is_changed() && review.as_ref().is_none_or(|review| !review.is_changed()) {
        return;
    }
    let view = effective_view(&view, review.as_deref());
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

    #[cfg(feature = "test-support")]
    #[derive(Resource, Default)]
    struct MultiplayerIntentLog(Vec<MultiplayerIntent>);

    #[cfg(feature = "test-support")]
    fn record_multiplayer_intents(
        mut intents: MessageReader<UiIntent>,
        mut log: ResMut<MultiplayerIntentLog>,
    ) {
        for intent in intents.read() {
            if let UiIntent::Multiplayer(intent) = intent {
                log.0.push(intent.clone());
            }
        }
    }

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
            MultiplayerRoute::HostCampaign,
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

    #[cfg(feature = "test-support")]
    #[test]
    fn campaign_browser_renders_three_slots_and_only_legal_host_actions() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
        let mut view = MultiplayerView {
            route: MultiplayerRoute::HostCampaign,
            ..Default::default()
        };
        view.campaign_slots[1].status = CampaignSlotStatusView::Available {
            party: Vec::new(),
            active_time: "42m".to_owned(),
        };
        view.campaign_slots[2].status = CampaignSlotStatusView::Invalid {
            reason: "Incompatible shipped content".to_owned(),
        };
        app.world_mut().insert_resource(view);
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Multiplayer);
        for _ in 0..8 {
            app.update();
        }

        let slots = app
            .world_mut()
            .query::<&Name>()
            .iter(app.world())
            .filter(|name| name.as_str().starts_with("Multiplayer Campaign Slot "))
            .count();
        assert_eq!(slots, 3);
        let actions = app
            .world_mut()
            .query::<&MultiplayerControl>()
            .iter(app.world())
            .filter(|control| matches!(&control.0, MultiplayerIntent::HostCampaign(_)))
            .count();
        assert_eq!(actions, 2, "invalid slots must expose no host action");
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn page_changes_reset_the_shared_multiplayer_scroll_position() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
        app.world_mut().insert_resource(MultiplayerView {
            route: MultiplayerRoute::HostCampaign,
            ..Default::default()
        });
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Multiplayer);
        for _ in 0..8 {
            app.update();
        }

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<MultiplayerSurface>>()
            .single(app.world())
            .expect("the Multiplayer screen has one scrolling root");
        let previous_control = app
            .world_mut()
            .query::<(Entity, &MultiplayerControl)>()
            .iter(app.world())
            .find_map(|(entity, control)| {
                matches!(&control.0, MultiplayerIntent::HostCampaign(_)).then_some(entity)
            })
            .expect("the Campaign browser exposes a host action");
        app.insert_resource(InputFocus::from_entity(previous_control));
        app.world_mut()
            .entity_mut(root)
            .insert(ScrollPosition(Vec2::new(0.0, 640.0)));
        app.world_mut().resource_mut::<MultiplayerView>().route = MultiplayerRoute::HostDirect;
        for _ in 0..3 {
            app.update();
        }

        assert_eq!(
            app.world()
                .get::<ScrollPosition>(root)
                .expect("the route root remains scrollable")
                .0,
            Vec2::ZERO
        );

        app.world_mut().resource_mut::<MultiplayerView>().route = MultiplayerRoute::HostCampaign;
        for _ in 0..3 {
            app.update();
        }
        app.world_mut()
            .entity_mut(root)
            .insert(ScrollPosition(Vec2::new(0.0, 640.0)));
        app.world_mut()
            .resource_mut::<MultiplayerView>()
            .campaign_host
            .refusal = Some("Incompatible shipped content".to_owned());
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(
            app.world()
                .get::<ScrollPosition>(root)
                .expect("the Campaign route remains scrollable")
                .0,
            Vec2::ZERO
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn client_campaign_save_status_is_visible_and_never_blocks_gameplay_input() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
        app.world_mut().insert_resource(MultiplayerView {
            role: Some(MultiplayerRole::Client),
            campaign_session: true,
            campaign_save_status: Some(MultiplayerCampaignSaveStatusView::Saving),
            ..Default::default()
        });
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..8 {
            app.update();
        }

        let (visibility, pickable) = app
            .world_mut()
            .query_filtered::<(&Visibility, &Pickable), With<MultiplayerCampaignStatus>>()
            .iter(app.world())
            .next()
            .expect("the client save status is a visible read-only projection");
        assert_eq!(*visibility, Visibility::Inherited);
        assert_eq!(*pickable, Pickable::IGNORE);
        assert!(app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .any(|text| text.0.contains("host is saving the complete Campaign")));
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
    fn host_connection_code_exposes_one_immediate_copy_action_only_when_shareable() {
        for route in [MultiplayerRoute::HostDirect, MultiplayerRoute::Lobby] {
            let mut app = App::new();
            app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
            app.world_mut().insert_resource(MultiplayerView {
                route,
                role: Some(MultiplayerRole::Host),
                local_seat: Some(PlayerSeat::HOST),
                share_code: Some(SensitiveText::new("HEX1.private-review-code")),
                ..Default::default()
            });
            app.world_mut()
                .resource_mut::<NextState<Screen>>()
                .set(Screen::Multiplayer);
            for _ in 0..8 {
                app.update();
            }

            let snapshot = crate::test_support::ui_tree_snapshot(app.world_mut());
            let copy = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Copy Connection Code" && node.focusable)
                .expect("a shareable host code must expose a copy action");
            assert_eq!(copy.keyboard_reachable, Some(true));
            assert_eq!(
                copy.visibility_requirement,
                Some(UiVisibilityRequirement::Immediate)
            );

            let controls = app
                .world_mut()
                .query::<(&Name, &MultiplayerControl)>()
                .iter(app.world())
                .filter(|(name, control)| {
                    name.as_str() == "Copy Connection Code"
                        && control.0 == MultiplayerIntent::CopyConnectionCode
                })
                .count();
            assert_eq!(controls, 1);
        }

        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
        app.world_mut().insert_resource(MultiplayerView {
            route: MultiplayerRoute::HostDirect,
            role: Some(MultiplayerRole::Host),
            share_code: None,
            ..Default::default()
        });
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Multiplayer);
        for _ in 0..8 {
            app.update();
        }
        assert!(app
            .world_mut()
            .query::<&Name>()
            .iter(app.world())
            .all(|name| name.as_str() != "Copy Connection Code"));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn host_connection_code_copy_accepts_a_real_pointer_press_after_gameplay_handoff() {
        use bevy::{
            input::{mouse::MouseButtonInput, ButtonState},
            window::PrimaryWindow,
        };

        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720))
            .init_resource::<MultiplayerIntentLog>()
            .add_systems(
                Update,
                record_multiplayer_intents.after(UiSystems::EmitIntents),
            );
        app.world_mut().insert_resource(MultiplayerView {
            route: MultiplayerRoute::Lobby,
            role: Some(MultiplayerRole::Host),
            local_seat: Some(PlayerSeat::HOST),
            share_code: Some(SensitiveText::new("HEX1.private-review-code")),
            ..Default::default()
        });
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..8 {
            app.update();
        }
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Multiplayer);
        for _ in 0..8 {
            app.update();
        }

        let (copy, center) = app
            .world_mut()
            .query::<(
                Entity,
                &Name,
                &MultiplayerControl,
                &ComputedNode,
                &bevy::ui::UiGlobalTransform,
            )>()
            .iter(app.world())
            .find_map(|(entity, name, control, computed, transform)| {
                (name.as_str() == "Copy Connection Code"
                    && control.0 == MultiplayerIntent::CopyConnectionCode)
                    .then_some((
                        entity,
                        transform.affine().translation * computed.inverse_scale_factor,
                    ))
            })
            .expect("the host lobby exposes its copy action");

        let window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .expect("headless UI supplies one primary window");
        app.world_mut()
            .entity_mut(window)
            .get_mut::<Window>()
            .expect("primary window remains live")
            .set_physical_cursor_position(Some(center.as_dvec2()));
        app.world_mut().write_message(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
            window,
        });

        app.update();

        assert_eq!(
            app.world().get::<Interaction>(copy),
            Some(&Interaction::Pressed),
            "Bevy's pointer hit-testing must press the exact lobby control"
        );
        assert_eq!(
            app.world().resource::<MultiplayerIntentLog>().0,
            [MultiplayerIntent::CopyConnectionCode]
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

    #[cfg(feature = "test-support")]
    #[test]
    fn local_ready_control_stays_inside_its_seat_across_the_required_matrix() {
        let matrix = [
            (UVec2::new(1280, 720), crate::UiScaleMode::Auto),
            (UVec2::new(1920, 1080), crate::UiScaleMode::Auto),
            (UVec2::new(3840, 2160), crate::UiScaleMode::Auto),
            (UVec2::new(1280, 720), crate::UiScaleMode::Percent200),
            (UVec2::new(1920, 1080), crate::UiScaleMode::Percent200),
            (UVec2::new(3840, 2160), crate::UiScaleMode::Percent200),
        ];

        for (size, mode) in matrix {
            for (ready, connection) in [
                (false, MultiplayerSeatConnectionView::Connected),
                (true, MultiplayerSeatConnectionView::ReclaimPending),
            ] {
                let mut app = App::new();
                app.add_plugins(crate::test_support::HeadlessUiPlugin::new(size.x, size.y))
                    .insert_resource(crate::UiScalePreference(mode));
                let mut view = MultiplayerView {
                    route: MultiplayerRoute::Lobby,
                    role: Some(MultiplayerRole::Client),
                    local_seat: Some(PlayerSeat(1)),
                    ..Default::default()
                };
                let guest = view
                    .seats
                    .get_mut(1)
                    .expect("six-seat fixture has seat two");
                guest.local = true;
                guest.ready = ready;
                guest.connection = connection;
                app.world_mut().insert_resource(view);
                app.world_mut()
                    .resource_mut::<NextState<Screen>>()
                    .set(Screen::Multiplayer);
                for _ in 0..8 {
                    app.update();
                }

                let snapshot = crate::test_support::ui_tree_snapshot(app.world_mut());
                let card = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == "Lobby Seat 2")
                    .expect("local seat card must render");
                let action_name = if ready { "Not Ready" } else { "Ready" };
                let action = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == action_name && node.focusable)
                    .expect("local readiness action must render");
                assert_eq!(action.parent_name.as_deref(), Some("Lobby Seat 2"));

                let card_bounds = Rect::from_center_size(card.center, card.size);
                let action_bounds = Rect::from_center_size(action.center, action.size);
                let epsilon = 0.51;
                assert!(
                    action_bounds.min.x + epsilon >= card_bounds.min.x
                        && action_bounds.max.x <= card_bounds.max.x + epsilon
                        && action_bounds.min.y + epsilon >= card_bounds.min.y
                        && action_bounds.max.y <= card_bounds.max.y + epsilon,
                    "{action_name} escaped its seat at {size:?} {mode:?}: action={action_bounds:?}, card={card_bounds:?}"
                );
                assert_eq!(action.keyboard_reachable, Some(true));
                assert_eq!(action.meets_minimum_target, Some(true));
            }
        }
    }
}
