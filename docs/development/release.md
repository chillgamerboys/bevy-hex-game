# Release artifact scaffold

Wave 5 produces unsigned pre-alpha archives. Every platform uses the
`hex-game-<platform>` artifact name and contains a `hex-game` executable (or
`hex-game.exe`), assets, the current placeholder `app-icon.png`, README, and any
platform symbol companions emitted by Rust. Release builds retain line-table debug
information. Symbols stay in the artifact; nothing uploads crash data.

The application identity is `com.chillgamerboys.hex-game`, with the player-facing name
**Hex Game**. The icon is deliberately the existing hex UI mark until product art
replaces it.

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
