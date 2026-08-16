# Same-network Sandbox multiplayer with LAN discovery

This is the canonical procedure for testing Hex multiplayer when the host and guest
computers are on the same local network. The game advertises an explicitly open Sandbox
lobby with mDNS/DNS-SD and joins it through the existing encrypted Direct transport. No
player needs to find an IP address, copy a `HEX1` code, configure DNS, or forward a router
port.

LAN discovery is a shipped convenience around Direct multiplayer, not a new gameplay
transport or authorization system. It works only on the current local multicast link.
For testers on different Internet connections, use the temporary
[Tailscale procedure](remote-multiplayer-testing.md); the eventual player-facing Internet
path remains EOS-backed Play Online.

## Before starting

1. Put both computers on the same trusted home or test network. A guest Wi-Fi network,
   client-isolated access point, separate VLAN, or some corporate networks may prevent
   devices from discovering or reaching one another.
2. Run the exact same Hex build and shipped content on both computers. For source builds,
   check out the same commit and launch with `cargo dev`.
3. Allow Hex local-network and incoming-network access if macOS, Windows, or a firewall
   prompts. Discovery uses standard mDNS multicast on UDP `5353`; the game session uses
   the selected Direct UDP port, `7777` by default.
4. Close any earlier Hex host still using the chosen Direct port.

## Host a Sandbox

1. Start Hex and choose **Multiplayer → Host LAN Sandbox**.
2. Configure the shipped Sandbox, choose the map and rosters, and complete deployment.
3. Wait in the six-seat assignment lobby until the LAN panel says **Discoverable now**.
4. Assign at least one party character to every connected guest. Do not launch until all
   guests have joined and marked themselves ready.

Advertisement begins only after deployment, when the assignment lobby is genuinely open.
It stops as soon as the host launches. Returning to the lobby makes the same still-open
session discoverable again.

## Join the host

1. Start the same build of Hex and choose **Multiplayer → Find LAN Games**.
2. Grant local-network access if the operating system asks.
3. Select the compatible Sandbox card. Sessions from a different build or shipped-content
   revision remain visible but cannot be joined.
4. In the lobby, wait for the host's assignment and select **Ready**.

The selected record supplies a resolved LAN address, the host's pinned certificate, and
the current invitation to the ordinary Direct connection. Exact protocol, build, content,
capacity, lobby phase, seat ownership, and generated-world checks still run on the host.

## Security boundary

mDNS/DNS-SD is unauthenticated local-link metadata. To make an open lobby joinable without
copying a private code, its current ephemeral invite is visible to devices on that LAN.
Treat **Host LAN Sandbox** like opening a table to everyone on a trusted room network:
anyone on the link may request a seat until Launch closes admission. The invitation is
rotated after admission, expires with the session certificate, and is never written to
normal logs or exposed to the UI adapter.

Use **Host Direct** and a private `HEX1` code when discovery is inappropriate. Neither LAN
mode weakens transport encryption, certificate pinning, host-derived seats, or command
authorization.

## Acceptance route

Use this minimum exact-head route:

1. The guest finds the host without entering an address or code.
2. Both processes show the same six-seat lobby; the host assigns the guest and launch is
   refused until the guest is ready.
3. Launch the default Flat Arena Sandbox and verify independent cameras, identical
   disclosed enemies, host-owned simulation, movement, and one combat decision.
4. Disconnect and restart the guest, then use the reserved-seat reconnect action and
   verify the authoritative world is restored without a duplicate command.
5. Return to the lobby and verify it becomes discoverable again, then close the host and
   verify the guest receives the typed host-closure reason.

Record the candidate SHA, both operating systems, network type, assigned seats, reconnect
result, host-loss result, and named human motion/input/presentation verdict. Never record
the underlying invite or reconnect credential.

## Diagnose a missing lobby

| Observation | Next check |
|---|---|
| The host is still configuring or deploying | Finish deployment; discovery intentionally starts only in the assignment lobby |
| The host says “Starting LAN discovery…” indefinitely or reports failure | Grant local-network/firewall access, use **Retry LAN Advertisement**, and check whether another security tool blocks UDP multicast |
| No lobby appears on the guest | Select **Refresh LAN Games**, grant local-network access, and confirm both devices use the same non-guest Wi-Fi/Ethernet network |
| A lobby is visible but disabled | Install the exact same build and shipped content; do not bypass the compatibility check |
| Discovery works but joining times out | Permit inbound UDP `7777` (or the host-selected port), disable access-point client isolation, and confirm the host is still pre-launch |
| Devices are on different VLANs, subnets with multicast filtering, or Internet connections | mDNS does not provide routed discovery; use Direct with a reachable route or the documented Tailscale test procedure |
| The host returned to Main Menu or quit | Create a new LAN session; its certificate and invitation are intentionally ephemeral |

Discovery chooses a private IPv4 address where available and ignores unspecified,
multicast, and unscoped IPv6 link-local addresses. IPv4 loopback is the last-resort
choice so two native processes on one computer can exercise the route; another computer
normally resolves the host's LAN interface instead. If a machine advertises several
usable interfaces and the selected route is not reachable from the guest, disable the
unrelated VPN/interface for this test or use Host Direct with the intended address.
