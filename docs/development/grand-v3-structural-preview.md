# Grand V3 structural preview

The `grand_v3_structural_preview` example compiles Grand V3 directly, exports the
public exact world snapshot, and writes structural-review data without starting a
Bevy app, opening a window, or launching the game. It is intended for fast shape
comparison before a rendered capture, not as a replacement for visual review.

Use the development-profile author loop while changing the compiler:

```sh
python3 tools/review.py structural author
```

Use the release, nonincremental checkpoint only when recording evidence:

```sh
python3 tools/review.py structural checkpoint
```

The compatibility script defaults to that checkpoint shape. An explicit first
argument selects the author loop instead:

```sh
tools/run_grand_v3_structural_review.sh
tools/run_grand_v3_structural_review.sh author
```

Choose a seed and destination below the structural-review root with:

```sh
python3 tools/review.py structural author \
  --seed 1592598566 \
  --output target/grand-v3-structural-preview/review
```

The default destination is content-addressed as
`target/grand-v3-structural-preview/<author-or-checkpoint>/<full-head>/<clean-or-dirty-diff>/seed-<seed>/`.
Custom destinations must remain below `target/grand-v3-structural-preview/` so the
wrapper can invalidate only review-owned files safely.

Both modes are strict by default. A diagnostic run may explicitly add
`--allow-structural-draft`, but `review-run.json` then labels the output
`structural-draft` and unapprovable. An inherited draft variable is scrubbed; the
flag is the only supported way to weaken admission. Author-mode output and dirty
worktree output are also exact, reproducible diagnostics rather than approval
evidence. Only a complete, strict, clean checkpoint is marked approvable.
Files marked `assume-unchanged` or `skip-worktree` in the Git index are recorded in
the receipt and conservatively make the worktree dirty, even when ordinary
`git status` output is empty.

Before checking disk space or starting Cargo, the wrapper locks the destination,
installs both incomplete markers and an incomplete operational receipt, and removes
stale owned evidence. It then requires at least 20 GiB free on the output filesystem.
If the threshold is missed, the old COMPLETE evidence remains invalidated; remove or
relocate reproducible inactive build/review artifacts and retry. Do not lower the
policy. Author mode uses the ordinary development profile with incremental
compilation. Checkpoint mode retains the release profile with incremental compilation
disabled.

A completed run writes these deterministic structural artifacts:

- `manifest.txt`: schema, seed, semantic fingerprint, bounds, and sample counts.
- `height-map.csv`: exact axial terrain, material, liquid, biome, and blocker data.
- `terrain-height-map.pgm` and `material-height-map.pgm`: portable grayscale maps
  for a quick image-editor view. Pixel zero means no sample; other pixels equal
  the integer level plus one. The PGM is an axial `q`/`r` rectangle, so it is
  intentionally skewed rather than screen-space hex rendering.
- `cross-sections.csv`: front and side profiles for both six-cell peak chains,
  six canonical Massif radial profiles, longitudinal and transverse Frozen Woods
  profiles, a lower-entry-to-summit Crystal shell profile, and the Crystal-to-Frozen
  exit profile. A bent peak-chain profile follows all six authored coarse peak cells;
  it is not a straight chord that can skip crowns or saddles.
- `profiles.svg`: a quick-look contact sheet of every cross-section plus the exact
  waterfall centerline. Each panel reports its own vertical range so plateau caps,
  cylindrical bases, disconnected saddles, and cascade steps remain legible; use the
  CSV files for exact numeric acceptance.
- `waterfall-centerline.csv`: the exact directed liquid path, local cross axis,
  contiguous wet width, and contiguous gorge-floor width at or below the water.
- `waterfall-gorge-width.csv`: every exact sample in each 25-column cross row.

It also writes operational evidence that is deliberately separate from the
deterministic structural manifest:

- `review-run.json`: strict/draft and author/checkpoint state, HEAD, dirty state,
  diff and full-workspace hashes, tokenized commands, exact binary hash, selected
  and executed counts, separate build/run durations, exact Cargo/rustc verbose
  identities, active rustup toolchain when available, artifact and log hashes,
  free-space preflight, and final freshness disposition.
- `review-build.log` and `review-execution.log`: complete Cargo and preview output
  for the recorded attempt.
- `review-verification-build.log`: the final same-command Cargo check for an
  approvable checkpoint. Every compiler artifact, including the exact preview
  example, must report fresh; any rebuild fails the no-op gate.

The wrapper builds through Cargo JSON, requires exactly one regular structural-preview
executable, hashes it, and then runs that exact path with an explicit repository
`BEVY_ASSET_ROOT`. Inherited review, Bevy, WGPU, compiler-wrapper, Rust flags, and
Cargo profile overrides are removed before build and execution. `CARGO_TARGET_DIR`
is forced to the repository `target/`; wrapper and rustflag variables are explicitly
disabled. This is not a hermetic Cargo installation: repository and inherited
`CARGO_HOME` configuration for registries, sources, networking, linkers, and other
settings can still apply. The exact command and resulting executable hash remain in
the receipt; use a controlled Cargo home when those remaining settings matter.
Cargo and rustc identity commands run under this same sanitized environment after
the destination is marked incomplete. Missing or incomplete Cargo/rustc identity is
a fail-closed error; rustup identity is recorded when rustup is available.

The waterfall crown and base review anchors deliberately sit on safe camera footing,
usually on a bank rather than in liquid. The preview resolves each to the nearest
point of the authoritative directed liquid chain (within three hexes) and rejects a
chain that never approaches either anchor. It does not require a water voxel and a
standable review anchor to share one coordinate.

All rows have stable names and coordinate ordering. Repeating the command against
the same code, assets, and seed produces byte-identical output. The CSV and PGM
projection is implemented in `hex_map::structural_preview`, where focused unit
tests cover peak/profile selection, radial ordering, directed-waterfall tracing,
gorge-width measurement, and canonical serialization.

Publication is fail-closed across both Cargo and generation. A persistent
`.review.lock` serializes one destination and is held by child processes too, so an
orphan cannot race a replacement wrapper. `REVIEW_INCOMPLETE.txt` belongs only to the
wrapper; the Rust publisher owns `INCOMPLETE.txt` and must not remove the wrapper marker.
The Rust marker must be absent immediately after successful child exit, while the
wrapper marker remains until all checks finish. SIGTERM, SIGHUP, SIGINT, timeouts, and
other interruptions terminate the child process group before recording failure.
INT/TERM/HUP handlers only set a cancellation flag; interruption is raised at controlled
phase boundaries so receipt fsync and failure cleanup cannot be interrupted midway.

The wrapper rejects known `Path not found` diagnostics, parses the structural schema
and seed as exact key/value records, verifies every artifact hash, and checks the
exact executable plus HEAD/diff/workspace identity repeatedly. A strict clean
checkpoint then repeats the identical Cargo build into its own log. That build must
be a no-op, resolve the same path and hash, and leave source provenance unchanged.
Output ancestry, directory identity, and lock identity are revalidated while the
lock is held at each publication boundary. The complete receipt is atomically
replaced and fsynced before the wrapper marker is durably removed. All required
artifacts and phase logs are opened without following a final symlink, hashed through
one descriptor, and fsynced first; pre/post verification artifact hashes must match.
A build or generator failure, low-disk preflight, cancellation, source edit, binary change,
non-no-op verification, missing file, malformed manifest, or failed hash leaves the
directory explicitly incomplete.

Treat a directory as current only when both `REVIEW_INCOMPLETE.txt` and
`INCOMPLETE.txt` are absent, `manifest.txt` exists, `.review.lock` is not currently
held by a running invocation, `review-run.json` says `COMPLETE`, and its artifact and
required-log hashes match. An approvable checkpoint must additionally record a
performed no-op verification with an unchanged binary. A complete capture set remains
`unapprovable` when the operational manifest names author mode, structural draft, or
a dirty worktree.

Argument, asset-root, and initial Git-provenance checks happen before the output lock.
Failure there is intentionally not an output attempt and does not invalidate an
explicit pre-existing destination; no Cargo or generator has started. Once the lock
is acquired, the destination is invalidated before disk and toolchain preflights.

This wrapper is local workflow stabilization, not a hostile-filesystem sandbox.
Phase-boundary ancestry and inode checks reject ordinary directory/symlink swaps, and
required files use no-follow opens, but a malicious process with write access can
still race path-based failure publication or binary launch between checks. Run review
work in a trusted checkout and do not approve a destination whose ancestry changed.

This lane proves integer world consequences available through Snapshot V1. It
does not prove materials look good, terrain reads clearly in perspective, or a
camera route is unobstructed; retain the normal rendered review for those claims.
