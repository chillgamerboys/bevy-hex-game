# EOS feasibility foundation

This behavior-neutral foundation precedes the
[host-owned Campaign wave](waves/host-owned-campaign/manifest.md) and the later Universal
Online wave. It fixes the shared seams that can be reviewed without an EOS product
deployment while keeping Direct/LAN and offline play unchanged.

## Locked architecture

- `hex_multiplayer` remains transport-neutral and owns no EOS handle, callback, product
  credential, Steam id, gameplay authority, or world authority.
- `hex_eos_ffi` is the sole workspace crate allowed to contain `unsafe`. It exposes owned
  safe values, accepts only an explicit absolute runtime path, and never searches the
  working directory or platform library path.
- `hex_online` owns safe EOS lifecycle and Aeronet I/O integration. Its backend is injected
  explicitly; installing `OnlinePlugin` without one loads no runtime, opens no socket, and
  emits a typed `Disabled` refusal.
- The pinned declaration baseline is official EOS SDK 1.19.1. An upgrade changes the pin,
  protected ABI evidence, release checksum, and this record together.
- EOS supplies the sole universal Internet lobby/P2P path. Steam supplies a Connect
  credential and native invitation/rich-presence entry into the same EOS lobby. No Steam
  gameplay transport or mirrored Steam lobby is introduced.
- The official SDK, redistributable runtimes, product/sandbox/deployment values, client
  credentials, encryption keys, and Steam configuration are protected release inputs and
  are never committed.

## Public foundation

The shared crate publishes transport selection, identity lifecycle, redacted online
principal/lobby/join-code types, seatless online requests, typed progress/refusal/events,
transport-bound reconnect vocabulary, bounded snapshot transfer headers/chunks/progress,
and `HostCampaignCheckpointV2`. Join codes carry 80 random bits, render as four groups of
four Crockford Base32 symbols only through an explicit sharing method, and place only a
SHA-256 digest in searchable metadata.

Snapshot-transfer metadata fixes a 32 KiB chunk, eight-chunk in-flight window, separate
compressed/uncompressed 64 MiB bounds, canonical-uncompressed SHA-256, session and transfer
identity, and authority baseline. Compression, acknowledgement, retry, transactional
assembly/import, and Aeronet channel behavior belong to the Universal Online O2 lane.

## Evidence and protected gate

Ordinary CI proves serialization/redaction/bounds, default-off composition, dynamic-loader
path/version refusal, strict Clippy/docs, dependency policy, and shipping compilation with
and without the `online` feature. It never substitutes a mock for a live-service claim.

Before Universal Online dispatch, protected CI and a native development deployment must:

1. mount the checksum-pinned official 1.19.1 headers and target runtime;
2. run `python3 tools/check_eos_sdk_abi.py --include-dir <absolute-sdk-include>` on
   each native target, compiling every declared C function/struct against those headers
   and comparing API-version, size, alignment, callback convention, and architecture
   expectations;
3. load the staged runtime on Windows x86_64, Linux x86_64, macOS x86_64, and macOS arm64;
4. prove Device ID login, Steam-ticket Connect login, lobby create/search/join, and a
   two-process P2P packet exchange; and
5. record product, sandbox, deployment, artifact checksum, target, and exact candidate SHA
   without printing credentials, product-user ids, lobby ids, raw join codes, or tickets.

The current checkout has no protected SDK artifact or EOS/Steam deployment configuration,
so those live claims remain explicitly **blocked external evidence**, not waived and not
reported as passing. Campaign implementation may consume the stable transport-neutral
checkpoint contract, but no Universal Online lane may claim service feasibility until this
gate is recorded.

The selected SDK baseline is corroborated by Epic's official
[May 2026 EOS SDK update](https://onlineservices.epicgames.com/news/online-services-and-fortnite-are-now-available-on-windows-on-snapdragon),
and the product-level service choice follows Epic's official
[multiplayer services overview](https://onlineservices.epicgames.com/multiplayer).
