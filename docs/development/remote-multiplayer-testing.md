# Remote multiplayer testing with Tailscale

This is the canonical temporary procedure for testing Hex's Direct multiplayer across
different Internet connections before Play Online has a live EOS lobby and relay path.
It lets a remote group exercise the real listen host, `HEX1` admission, protocol,
authority, disclosure, reconnect, and gameplay behavior without requiring a publicly
routable address in the shared game connection code or configuring a router.

If both computers are on the same physical network, use the simpler
[automatic LAN discovery procedure](lan-multiplayer-testing.md) instead. mDNS is a
same-link mechanism and is deliberately not part of this remote Tailscale route.

Tailscale is **external test infrastructure**. It is not linked into, launched by, or
distributed with Hex, and it is not the intended player-facing multiplayer service.
The shipping direction remains EOS-backed Play Online with Steam-native invitations;
Direct/LAN remains the advanced fallback.

## What each layer owns

| Layer | Responsibility |
|---|---|
| Hex Direct multiplayer | Encrypted WebTransport, pinned session certificate, private invite token, exact build/content admission, authoritative simulation, seat ownership, and reconnect credentials |
| Tailscale | A private route between testers, including NAT traversal and an encrypted relay fallback when a direct route is unavailable |
| Future EOS and Steam adapters | Player-facing lobby discovery, short codes, Internet traversal/relay, platform identity, and native invitations |

The game still opens its ordinary Direct UDP endpoint. Tailscale supplies a reachable
virtual address for that endpoint; it does not replace or bypass any game protocol or
authorization check.

## Cost and account boundary

As checked on **2026-08-16**, Tailscale's Personal plan is free for up to six users
and unlimited personal user devices. Tailscale describes playing games with friends as
a Personal-plan use case. Six users exactly covers Hex's current listen host plus five
guest limit. Pricing and terms can change, so verify the
[current pricing](https://tailscale.com/pricing) before organizing a test.

The free Personal plan is not promised for commercial use. A company-run QA program or
commercial distribution must use an appropriate Tailscale plan or obtain confirmation
from Tailscale. This repository does not grant a right to redistribute, embed, or
commercially depend on Tailscale.

Every tester uses their own Tailscale account. Never share an account, authentication
cookie, or device key.

## Security rules

1. Prefer sharing only the host computer with each guest instead of adding guests to
   the host's whole tailnet. Tailscale documents this as
   [machine sharing](https://tailscale.com/kb/1084/sharing).
2. Use a single-use machine-share link for a bounded test when practical. Treat that
   link like a password until it is accepted.
3. If the tailnet has custom grants, limit guests to the shared host and the selected
   game UDP port. The default game port is `7777`.
4. Send the complete `HEX1...` connection code only in a private channel. It contains
   a session invitation credential even though normal game diagnostics redact it.
5. Never put a connection code, reconnect credential, Tailscale invitation, or account
   token in a terminal transcript, issue, PR, screenshot, test report, or game log.
6. Close the hosted game session and revoke the machine share after a one-off test.

Sharing the host computer does not make the game publicly reachable. It makes the host
reachable to the accepted Tailscale user, subject to the tailnet's access rules. Review
Tailscale's sharing and access-control documentation before using an existing work or
home tailnet that contains other sensitive services. Tailscale's documentation
currently labels machine sharing as beta and notes that peers may learn physical
endpoint addresses while negotiating a route. If machine sharing is unavailable, use
a tailnet containing only the test machines or configure restrictive grants before
inviting anyone to an existing tailnet.

## One-time tester setup

### Host

1. [Install Tailscale](https://tailscale.com/download) from its official package for
   the host operating system and sign in with a personal account.
2. In the Tailscale admin console, locate the computer that will run Hex.
3. Share that computer with the guest by email or a single-use link.
4. Wait for the guest to accept the share.
5. Copy the host computer's Tailscale IPv4 address. It normally begins with `100.`.
   The admin console and desktop client display it. Where the CLI is installed,
   `tailscale ip -4` prints it.

### Guest

1. [Install Tailscale](https://tailscale.com/download) and sign in with a separate
   account.
2. Accept the host's machine-share invitation.
3. Confirm that the shared host appears in the Tailscale client.
4. Where the CLI is installed, run `tailscale ping <host-tailscale-ip>` to verify the
   private route before diagnosing the game.

A Tailscale `100.x.y.z` address is expected here. Although this address block is also
used for carrier-grade NAT on the public Internet, Tailscale explicitly routes its
addresses between authorized members. Do not substitute the host's `127.0.0.1`, `.local`
hostname, ordinary LAN address, or public IP for this procedure.

## Prepare an exact game build

Every participant must run the same revision and shipped content. For a source-build
test, record the candidate before launching:

```sh
git rev-parse HEAD
```

Launch from the repository root through Cargo so the configured asset root is applied:

```sh
cargo run --release -p hex_game
```

Do not launch `target/debug/hex_game` or `target/release/hex_game` directly. Packaged
builds are acceptable when every tester has the same artifact.

An exact-head test report records the commit SHA and operating system for every
process. Do not treat matching version text as proof that two locally built trees are
identical.

## Host a Sandbox

Sandbox is the preferred first remote route because it proves transport and ordinary
multiplayer behavior without involving Campaign persistence.

1. Start Hex and choose **Multiplayer → Host Direct**.
2. Set **Advertised Host** to the host's Tailscale IPv4 address.
3. Leave the port at `7777` unless another process already owns it. If the port is
   changed, ensure the tailnet policy permits the replacement UDP port.
4. Choose **Configure Shipped Sandbox**.
5. Select the shipped map and rosters, then complete deployment. The current Direct
   flow freezes this setup before opening the assignment lobby.
6. In the lobby, choose **Copy Connection Code**.
7. Send the complete `HEX1...` code to the guest privately.

The connection code already contains the Tailscale address, UDP port, certificate
fingerprint, certificate expiry, and invite token. The guest does not enter the host
address separately.

## Join the Sandbox

1. Confirm Tailscale is connected and the shared host is visible.
2. Start the exact same Hex build.
3. Choose **Multiplayer → Join Direct**.
4. Paste the complete `HEX1...` code and choose **Join Session**.
5. Wait for build, protocol, shipped-content, and generated-world verification.
6. The host assigns at least one party member to every connected guest.
7. Each guest marks their seat ready; the host launches when the lobby permits it.

The macOS firewall may ask whether Hex or the launching terminal may accept incoming
traffic. The host must allow the game process to receive that traffic.

## Sandbox acceptance route

Record a separate result for each item rather than one undifferentiated “multiplayer
works” verdict:

- The guest reaches the lobby without a public IP or router port forward.
- Assignment changes clear affected readiness, and each connected seat owns at least
  one party member before launch.
- Both peers complete map verification and reach the same encounter.
- Host and guest cameras remain independent.
- A hostile that is currently disclosed to the shared player faction appears on both
  peers; withdrawal and re-observation agree.
- Each player can select and command only their assigned party members.
- Exploration movement, combat entry, active turns, casting, defender decisions,
  terrain mutation, and outcome presentation agree.
- The guest can close the process, restart the same build, choose
  **Reconnect Reserved Seat**, and recover the active session without duplicating a
  command.
- Host-only retry, return-to-lobby, global pause, and close controls remain host-only.
- Host shutdown returns the guest to Multiplayer with a typed reason.

Human observation is evidence for connection usability, native input, camera
independence, movement feel, and presentation. It does not replace the protocol,
authority, disclosure, or snapshot tests that establish exact game state.

## Campaign follow-up

A Sandbox PASS proves the remote Direct path but does not prove Campaign behavior. A
candidate that changes Campaign must also run its Campaign-specific route:

1. Host a new or compatible Campaign slot through **Host Campaign**.
2. Complete fresh seat assignment and launch.
3. Enter quiescent exploration, pause globally as host, and save.
4. Confirm the guest sees the non-blocking host-saving status and cannot save.
5. Close the session, restart the host, resume the saved slot into a fresh lobby, and
   verify restored world and party presentation.

Do not record a Campaign PASS from Sandbox evidence alone.

## Diagnose a failed connection

Work from the outside in:

| Observation | Next check |
|---|---|
| `tailscale ping` cannot reach the shared host | Confirm both clients are connected, the machine share was accepted, and tailnet grants permit the guest to reach the host |
| Tailscale reaches the host but Hex times out | Confirm the host is still in the lobby, the code advertises the Tailscale address, UDP port `7777` is permitted, and the host firewall accepted incoming traffic |
| The code advertises `127.0.0.1` or a LAN address | Close the session and create a new one after entering the Tailscale address; an already-issued code is immutable |
| Admission reports protocol, build, content, or map mismatch | Rebuild or redistribute one exact candidate; do not weaken compatibility checks |
| A second host cannot open the endpoint | Stop the previous host or select a distinct UDP port before creating the new session |
| A formerly valid code stops working | Confirm the original host session still exists. A restarted host creates a new session identity, certificate, and invitation code |
| The route works but latency is higher than expected | Where the CLI is installed, inspect `tailscale status`; a `relay`/DERP path is valid but typically slower than a `direct` path |
| The guest joins but authoritative state or presentation differs | Treat it as a game defect, not a VPN success. Preserve redacted logs and exact-head reproduction steps |

Tailscale tries a direct encrypted route and falls back to a relay when the network
prevents direct connectivity. Both are valid functional-test paths; record which one
was observed. See Tailscale's description of
[connection types](https://tailscale.com/docs/reference/connection-types).

For asset, renderer, and general launch failures, use the repository's
[troubleshooting guide](troubleshooting.md).

## Record the result

Use this minimum handoff without including secrets:

```text
Candidate SHA:
Host OS/build source:
Guest OS/build source:
Tailscale route: direct | relay | unknown
Scenario and seed:
Seats and assignments:
Sandbox route result:
Reconnect result:
Host-loss result:
Human movement/input/presentation verdict:
Campaign follow-up result, if applicable:
```

The exact Tailscale address may be omitted. The `HEX1` code and reconnect credential
must always be omitted.

## End the test

1. Use the host's **Close Session** action when possible so guests receive the typed
   closure.
2. Quit every game process.
3. Revoke the Tailscale machine share when ongoing access is unnecessary.
4. Remove temporary tailnet membership if membership was used instead of machine
   sharing.

Tailscale remains useful for private testing after EOS ships because it can exercise
the Direct fallback across real machines. It must never become evidence that the EOS
join-code, Steam-invite, or service-outage paths work; those require their own live
service matrix.
