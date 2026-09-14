# [UI] Atlas legend overlaps the explorer HUD

Local intake; Linear requires reauthentication, so no issue was created.
The repair is within the authorized V4 integration work.

- Exact build: clean `453daa3ac9d4ba67861922f928e97171a43ad3db`, release.
- Route: seven-region explorer, atlas view, focus 0,0,40, two parties, radius 16.
- Output: windowless 1920×1080 capture, macOS 26.6.2.
- Evidence: `seven-region-atlas` in the task's `work/v4-release-captures-r3`.
- Reproduction: open the atlas with M after terrain publication settles.
- Observed: the normal explorer HUD covers the left portion of the atlas legend.
- Expected: the atlas legend is readable; the ordinary HUD returns when the atlas closes.
- Repair: hide the ordinary HUD layout while atlas visibility is active.
- Acceptance: repeat the atlas capture and inspect both legend and geography;
  interactive native toggle review remains pending.
