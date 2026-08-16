//! Pure client-hosted multiplayer screen and local-overlay navigation.

use bevy_ecs::prelude::Resource;
use hex_core::PlayerSeat;

/// Coarse player-facing route inside the Multiplayer screen.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MultiplayerRoute {
    /// Direct Host / Direct Join entry.
    #[default]
    Home,
    /// Three-slot Campaign browser and Direct/LAN host endpoint.
    HostCampaign,
    /// Continuous mDNS/DNS-SD browser for explicitly open lobbies on this LAN.
    BrowseLan,
    /// Endpoint help and Sandbox configuration handoff.
    HostDirect,
    /// Bounded connection-code entry.
    JoinDirect,
    /// A direct socket exists and custom admission is pending.
    Connecting,
    /// Six-seat assignment/readiness lobby.
    Lobby,
    /// Frozen peers are generating and verifying the exact world.
    Loading,
    /// A previously admitted client is reconnecting/catching up.
    Reconnecting,
    /// The previous session ended with a typed reason.
    Ended,
}

/// Local process role within a multiplayer session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiplayerRole {
    /// Listen host and simulation authority.
    Host,
    /// Remote replica.
    Client,
}

/// Stable reason displayed after returning from a failed or ended session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiplayerEndReason {
    /// A direct client connection or listen-host endpoint failed.
    ConnectionFailed,
    /// Listen host transport ended; host migration is unsupported.
    HostDisconnected,
    /// Host explicitly closed the session.
    HostClosed,
    /// Host removed this client.
    Kicked,
    /// Protocol or security validation failed.
    ProtocolViolation,
    /// Protocol, build, or shipped-content identity is incompatible.
    Incompatible,
    /// The host no longer accepts new players.
    LobbyClosed,
    /// Every available human seat or party assignment is claimed.
    LobbyFull,
    /// The invite or reconnect credential was rejected.
    InvalidCredential,
    /// Exact world verification failed.
    MapMismatch,
    /// The encounter/session ended normally.
    SessionEnded,
}

/// Result of Back/Escape while the Multiplayer screen is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiplayerBackResult {
    /// A child setup/end route returned to Multiplayer home.
    Home,
    /// A live or connecting session must be left before returning home.
    LeaveSession,
    /// Multiplayer home returns to the Main Menu.
    MainMenu,
}

/// Renderer-free local navigation layered over canonical network session state.
#[derive(Resource, Debug, Clone, PartialEq, Eq, Default)]
pub struct MultiplayerModel {
    /// Current Multiplayer screen route.
    pub route: MultiplayerRoute,
    /// Local process role after Host/Join begins.
    pub role: Option<MultiplayerRole>,
    /// Host-derived human seat after admission; host is always seat zero.
    pub local_seat: Option<PlayerSeat>,
    /// Typed reason retained on the ended route.
    pub ended_reason: Option<MultiplayerEndReason>,
    /// Remote-client Escape overlay; it never implies global `Pause`.
    pub local_menu_open: bool,
    /// Monotonic immutable-view invalidation token.
    pub revision: u64,
}

impl MultiplayerModel {
    /// Enters a fresh Multiplayer home without retaining a previous session identity.
    pub fn enter_home(&mut self) {
        self.set_session_route(MultiplayerRoute::Home, None, None);
        self.ended_reason = None;
    }

    /// Opens Host Direct endpoint/setup help.
    pub fn show_host_direct(&mut self) {
        self.set_session_route(MultiplayerRoute::HostDirect, None, None);
    }

    /// Opens the host-owned Campaign slot browser.
    pub fn show_host_campaign(&mut self) {
        self.set_session_route(MultiplayerRoute::HostCampaign, None, None);
    }

    /// Opens same-network lobby discovery without assuming a session role.
    pub fn show_lan_browser(&mut self) {
        self.set_session_route(MultiplayerRoute::BrowseLan, None, None);
    }

    /// Opens Join Direct connection-code entry.
    pub fn show_join_direct(&mut self) {
        self.set_session_route(MultiplayerRoute::JoinDirect, None, None);
    }

    /// Records an explicitly started host or client connection attempt.
    pub fn connecting(&mut self, role: MultiplayerRole) {
        let seat = (role == MultiplayerRole::Host).then_some(PlayerSeat::HOST);
        self.set_session_route(MultiplayerRoute::Connecting, Some(role), seat);
    }

    /// Records successful custom admission and opens the assignment lobby.
    pub fn admitted(&mut self, role: MultiplayerRole, seat: PlayerSeat) -> bool {
        if !seat.is_human()
            || (role == MultiplayerRole::Host && seat != PlayerSeat::HOST)
            || (role == MultiplayerRole::Client && seat == PlayerSeat::HOST)
        {
            return false;
        }
        self.set_session_route(MultiplayerRoute::Lobby, Some(role), Some(seat));
        true
    }

    /// Projects the canonical host lobby phase into the corresponding local route.
    pub fn show_lobby(&mut self) {
        self.set_route(MultiplayerRoute::Lobby);
    }

    /// Projects map generation/fingerprint verification.
    pub fn show_loading(&mut self) {
        self.set_route(MultiplayerRoute::Loading);
    }

    /// Projects reconnect/catch-up without changing the retained role/seat.
    pub fn show_reconnecting(&mut self) {
        self.set_route(MultiplayerRoute::Reconnecting);
    }

    /// Ends the current session and clears local authority/menu state.
    pub fn end(&mut self, reason: MultiplayerEndReason) {
        self.role = None;
        self.local_seat = None;
        self.local_menu_open = false;
        self.route = MultiplayerRoute::Ended;
        self.ended_reason = Some(reason);
        self.bump();
    }

    /// Applies Back/Escape for screen-local routes without performing external effects.
    pub fn back(&mut self) -> MultiplayerBackResult {
        match self.route {
            MultiplayerRoute::Home => MultiplayerBackResult::MainMenu,
            MultiplayerRoute::HostCampaign
            | MultiplayerRoute::BrowseLan
            | MultiplayerRoute::HostDirect
            | MultiplayerRoute::JoinDirect
            | MultiplayerRoute::Ended => {
                self.enter_home();
                MultiplayerBackResult::Home
            }
            MultiplayerRoute::Connecting
            | MultiplayerRoute::Lobby
            | MultiplayerRoute::Loading
            | MultiplayerRoute::Reconnecting => MultiplayerBackResult::LeaveSession,
        }
    }

    /// Toggles the remote client's non-pausing gameplay menu. Hosts use global pause and
    /// therefore cannot open this local-only surface.
    pub fn toggle_client_menu(&mut self) -> bool {
        if self.role != Some(MultiplayerRole::Client) {
            return false;
        }
        self.local_menu_open = !self.local_menu_open;
        self.bump();
        true
    }

    fn set_session_route(
        &mut self,
        route: MultiplayerRoute,
        role: Option<MultiplayerRole>,
        local_seat: Option<PlayerSeat>,
    ) {
        self.route = route;
        self.role = role;
        self.local_seat = local_seat;
        self.local_menu_open = false;
        self.ended_reason = None;
        self.bump();
    }

    fn set_route(&mut self, route: MultiplayerRoute) {
        if self.route != route {
            self.route = route;
            self.bump();
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_matrix_distinguishes_setup_live_and_root_routes() {
        for route in [
            MultiplayerRoute::HostCampaign,
            MultiplayerRoute::BrowseLan,
            MultiplayerRoute::HostDirect,
            MultiplayerRoute::JoinDirect,
            MultiplayerRoute::Ended,
        ] {
            let mut model = MultiplayerModel {
                route,
                ..Default::default()
            };
            assert_eq!(model.back(), MultiplayerBackResult::Home);
            assert_eq!(model.route, MultiplayerRoute::Home);
        }
        for route in [
            MultiplayerRoute::Connecting,
            MultiplayerRoute::Lobby,
            MultiplayerRoute::Loading,
            MultiplayerRoute::Reconnecting,
        ] {
            let mut model = MultiplayerModel {
                route,
                role: Some(MultiplayerRole::Client),
                local_seat: Some(PlayerSeat(1)),
                ..Default::default()
            };
            assert_eq!(model.back(), MultiplayerBackResult::LeaveSession);
            assert_eq!(model.route, route, "external leave must settle first");
        }
        let mut root = MultiplayerModel::default();
        assert_eq!(root.back(), MultiplayerBackResult::MainMenu);
    }

    #[test]
    fn admission_rejects_non_human_and_nonzero_host_seats() {
        let mut model = MultiplayerModel::default();
        assert!(!model.admitted(MultiplayerRole::Client, PlayerSeat::AI));
        assert!(!model.admitted(MultiplayerRole::Client, PlayerSeat::HOST));
        assert!(!model.admitted(MultiplayerRole::Host, PlayerSeat(1)));
        assert!(model.admitted(MultiplayerRole::Host, PlayerSeat::HOST));
        assert_eq!(model.route, MultiplayerRoute::Lobby);
    }

    #[test]
    fn only_remote_clients_can_open_the_non_pausing_menu() {
        let mut host = MultiplayerModel::default();
        host.connecting(MultiplayerRole::Host);
        assert!(!host.toggle_client_menu());
        assert!(!host.local_menu_open);

        let mut client = MultiplayerModel::default();
        client.connecting(MultiplayerRole::Client);
        assert!(client.admitted(MultiplayerRole::Client, PlayerSeat(2)));
        assert!(client.toggle_client_menu());
        assert!(client.local_menu_open);
        assert!(client.toggle_client_menu());
        assert!(!client.local_menu_open);
    }

    #[test]
    fn typed_end_clears_session_identity_but_retains_reason() {
        let mut model = MultiplayerModel::default();
        model.connecting(MultiplayerRole::Client);
        assert!(model.admitted(MultiplayerRole::Client, PlayerSeat(3)));
        assert!(model.toggle_client_menu());

        model.end(MultiplayerEndReason::HostDisconnected);

        assert_eq!(model.route, MultiplayerRoute::Ended);
        assert_eq!(model.role, None);
        assert_eq!(model.local_seat, None);
        assert!(!model.local_menu_open);
        assert_eq!(
            model.ended_reason,
            Some(MultiplayerEndReason::HostDisconnected)
        );
    }
}
