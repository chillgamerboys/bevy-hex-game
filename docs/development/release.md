# Releases

Releases produce unsigned pre-alpha archives. Every platform uses the
`hex-game-<platform>` artifact name and contains a `hex-game` executable (or
`hex-game.exe`), assets, the current placeholder `app-icon.png`, README, and any
platform symbol companions emitted by Rust. Release builds retain line-table debug
information. Symbols stay in the artifact; nothing uploads crash data.

The application identity is `com.chillgamerboys.hex-game`, with the player-facing name
**Hex Game**. The icon is deliberately the existing hex UI mark until product art
replaces it.

## Procedure

The release chain is deliberately split so no tag can bypass the tested integration
branch or the human visual gate:

1. Land feature and documentation PRs on `dev`. Confirm its required CI, scripted
   visual walks, and affected manual walkthroughs.
2. Run the release workflow's dry run from current `dev` to compute the next semantic
   version and inspect the generated changelog.
3. Land the release bump as an ordinary PR to `dev`. Only this PR changes the
   workspace version in `Cargo.toml`, refreshes `Cargo.lock`, and moves `Unreleased`
   notes into the dated version section.
4. Have a human play and inspect that exact `dev`, then promote `dev` to `main` with a
   merge commit. `dev` is permanent and is never deleted.
5. Tag the promoted `main` merge commit with the matching `vX.Y.Z`, create the GitHub
   release from the changelog section, and push the tag. Tags are immutable.
6. Verify that the tag workflow succeeds and that the release contains all four
   archives: Linux x86-64, Windows x86-64, macOS Apple Silicon, and macOS Intel. Open
   at least the primary-platform archive and confirm the executable, assets, README,
   icon, and any symbol companion are present.

The repository's release and promotion skills are the executable procedure; this page
records the invariants an operator should verify. An apparent infrastructure timeout
is retried once after confirming no compiler, test, packaging, or application error
preceded it. A second identical hard timeout requires an explicit maintainer waiver in
the release notes; it is never silently called a pass.

## Reserved integrations

No job reads these names yet. They document the future credential boundary so enabling
one integration is an isolated reviewed change rather than an ad-hoc secret hunt.

| Future lane | Reserved configuration |
|---|---|
| Apple signing and notarization | `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` |
| Steam depot upload | `STEAM_APP_ID`, `STEAM_DEPOT_ID`, `STEAM_USERNAME`, `STEAM_CONFIG_VDF` |
| Opt-in crash reporting | `SENTRY_DSN`, `SENTRY_AUTH_TOKEN`, `SENTRY_ORG`, `SENTRY_PROJECT` |

Adding credentials alone must never activate a service. A later productization PR must
add the job or client, consent and failure behavior, environment protection, and a
human release test. Codesigning, notarization, Steam upload, telemetry, and crash
reporting are not Wave 5 gates.
