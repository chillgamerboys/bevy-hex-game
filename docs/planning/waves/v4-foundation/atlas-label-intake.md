# [UI] World atlas: feature labels obscure the geography

Local intake draft; no Linear issue was created. The connector requires reauthentication.
Implementation remains part of the already authorized V4 authoring-tool lane.

- Build: `1771610959cfc745965e5e47540c9fc357b422ea`, dirty working tree containing
  the new world tool and explorer. This is diagnostic evidence, not an approval pack.
- Compiler source SHA256: `8a2a219d625b6f005f4c694900a8b9eecdc388cf39655fda4ae0486376b84387`.
- Binary SHA256: `12fbb2fda3988f13ce26dbca4815dfddec8fe89682e71d928d5ecf4bf9278008`.
- Package fingerprint: `41bc190a1c9d087d`.
- Route/state: generated `one-region/review.html`, initial world-fit atlas.
- Phase: offline geographic authoring review.
- Viewport: 1280×720 logical pixels; devicePixelRatio 2, browser default page zoom.
- OS: macOS 26.6.2, build 25G83.

Reproduction: compile `assets/config/v4/rich-region.ron`, open the generated atlas,
and leave its default landmark display enabled. The 360 feature instances include
plants; their labels overlap across both mountainous and dune areas. Reproduced once
on the first load, with a CUA screenshot retained in the implementation task.

Expected: geography and named landmarks remain readable at world fit. Routine plants
must not be labeled as major landmarks, and visible text must be decluttered.

Impact: geographic review is obstructed. Turning landmarks off is a workaround;
the issue does not block compilation or exact map validation.

Acceptance:

- The initial 1280×720 atlas shows the entire region and legible selected landmarks.
- World-fit and region-focus labels do not overlap at the tested viewport.
- Landmark toggling, panning and zooming remain usable.
- Feature-instance counts remain accurate and distinct from landmark counts.

Status: repaired in the authoring tool. Fresh background-browser inspection at
1280×720 confirms six legible selected landmarks, the full region footprint, and
separate counts for 360 feature instances. Compiler source SHA256 for that preview:
`18327ba4bd4019323b006ff0fd021e41026b2b11e0fa46d0133bd627dbc17b59`;
binary SHA256: `43239170b6844a059b5746442b1fac114ab5a9c73002af26f06f47543d1e2526`.
This remains a diagnostic dirty-tree preview. Broader interaction and exact-candidate
visual review remain separate gates.
