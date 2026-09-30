# Hex Game temporary dependency patch

This directory contains the published `aeronet_transport` 0.21.0 crate, with the
single dependency change recorded in `hex-ringbuf-0.5.2.patch`. No upstream Rust
source file is modified. This note, the patch file, and the two upstream license
files are additions to the published archive.

## Provenance

- Upstream repository: https://github.com/aecsocket/aeronet
- Published package: https://crates.io/crates/aeronet_transport/0.21.0
- Exact archive: https://static.crates.io/crates/aeronet_transport/aeronet_transport-0.21.0.crate
- Archive SHA-256: `08794e7ff02cf169ccc643d94327d2bea38f60d92136acd201b8705630d9e15e`
- The archive checksum was verified against the workspace Cargo.lock before unpacking.
- Upstream VCS commit recorded in the archive: `cbbaff12df74f9f611619a89f47c75a2f01a5e32`
- Original published Cargo.toml.orig and .cargo_vcs_info.json are preserved.

The crate declares `MIT OR Apache-2.0` but omits license texts from its published
archive. The following texts were added verbatim from that exact upstream revision:

- `LICENSE-APACHE`: https://raw.githubusercontent.com/aecsocket/aeronet/cbbaff12df74f9f611619a89f47c75a2f01a5e32/LICENSE-APACHE
  SHA-256: `a6cba85bc92e0cff7a450b1d873c0eaa2e9fc96bf472df0247a26bec77bf3ff9`
- `LICENSE-MIT`: https://raw.githubusercontent.com/aecsocket/aeronet/cbbaff12df74f9f611619a89f47c75a2f01a5e32/LICENSE-MIT
  SHA-256: `508a77d2e7b51d98adeed32648ad124b7b30241a8e70b2e72c99f92d8e5874d1`

## Minimal change

RUSTSEC-2026-0293 affects ringbuf before 0.5.2:
https://rustsec.org/advisories/RUSTSEC-2026-0293.html

Aeronet Transport 0.21.0 requires ringbuf ^0.4.8, which cannot resolve the fixed
0.5.2. Updating Cargo.lock alone cannot remove the vulnerable version. This patch
changes only `[dependencies.ringbuf].version` from `0.4.8` to `0.5.2`.
`default-features = false`, `alloc`, and `portable-atomic` remain unchanged.
No Aeronet protocol, sampling, or networking implementation changes.

The application remains on Bevy 0.19 and Aeronet 0.21.0. The APIs used by this
crate were inspected in ringbuf 0.5.2, but integration compilation and multiplayer
tests still determine compatibility. Preparation is not a successful test run.

## Integration and removal

Exclude `vendor/aeronet_transport` from workspace membership. Add a
`[patch.crates-io]` entry selecting this path for `aeronet_transport`, update the
workspace lockfile, and rerun the dependency advisory and multiplayer checks.
Do not add an advisory ignore or change unrelated dependencies for this patch.

Remove this directory, its workspace exclusion, and its crates.io patch when a
published Aeronet Transport release compatible with the application's accepted
Bevy/Aeronet line requires fixed ringbuf >=0.5.2. Update Cargo.lock, verify the
vulnerable ringbuf version is absent, and rerun the same checks before removal.
