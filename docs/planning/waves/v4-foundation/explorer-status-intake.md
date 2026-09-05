# [UI] V4 explorer retains its initial loading status

Local intake; no Linear issue was created because the connector requires
reauthentication. The repair is within the authorized V4 integration work.

- Exact build: clean `445799546a2b68cae5d1a104c314805316322fa8`, release.
- Route: windowless V4 explorer, one-region fixture, focus -72,20,40,
  radius 16, one party, orbit view, azimuth 35.
- Phase: initial exploration after publication settles.
- Viewport/output: 1920×1080, scale factor 1; macOS 26.6.2 (25G83).
- Evidence: `fixed-one-party-1` capture and typed receipt in the task's
  `work/v4-release-captures`, identifying 600 settled frames and zero loading work.
- Reproduction: start the named scene and wait without issuing a command.
- Observed: HUD retains “Loading nearby terrain...” after loading completes.
- Expected: initial guidance remains accurate before and after publication;
  the independent loading counter reports pending work.
- Impact: misleading status; movement remains available.
- Acceptance: repeat the settled capture and inspect the guidance and counter.

Repair: replace the sticky initial status with “Select terrain to plan a route”.
Final clean capture verification is recorded with the V4 acceptance results.
