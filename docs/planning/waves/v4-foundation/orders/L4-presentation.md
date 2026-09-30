# L4 — Resident terrain presentation

Owner: world. Worktree: `../hex-v4-presentation`; branch: `feat/v4-presentation`.

Consume validated V4 chunk products without installing the V3 whole-world plugin.
Reuse the existing bounded terrain mesh generation and exact logical run contract.
Publish and retire individual chunk roots and assets; share material assets. Convert
exact i64 world coordinates to bounded render-local integer coordinates before float
conversion. Support origin rebasing, revision checks, edits, teardown and re-entry.

The adapter must not become terrain authority or load packages itself. The application
coordinator feeds resident products and controls bounded publication. Support source
material colors and flags without inheriting the frozen V3 substance-name registry.
Retain all stacked runs and exact headroom, including static object occupancy.

Allowed files are the new `hex_map/src/v4/` module, necessary extraction in `grid.rs`,
module export in `lib.rs`, and map Cargo dependency wiring. Avoid changing V3 behavior.
Commit independently; source tests before promotion, and coordinate the expensive Cargo
lane. Root owns the actual app, its motion consumers, and combined windowless captures.
