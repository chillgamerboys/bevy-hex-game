# Banked foundation map (a872a57)

- `crates/hex_core/src/arena.rs:224`: ArenaTerrainView is the passive authoritative world publication. Add optional expedition sites without coupling world to Species or private gameplay profiles.
- `crates/hex_map/src/arena/forest.rs:337`: current protected edit ranges span entire canopy columns. Replace only in agreement with V4 admission and explicit roots.
- `crates/hex_world_runtime/src/edits.rs:455`: full occupied-column/root guard rejects edits in free air. Exact object intervals alone are insufficient: also preserve every buttress ground contact and anchors. Conservative fallback for older packages.
- `crates/hex_schematic/src/v4/operators.rs:619`: bridge sweep rejects different endpoint heights in overlapping width disks. Graded arched bridge must use deterministic cross-sections.
- `crates/hex_arena/src/encounters.rs:1024`: separate_many currently all-pairs for up to four passes. Add a deterministic conservative spatial broadphase, preserving pair order and physical semantics.
- `crates/hex_arena/src/progression.rs:209`: roster species and hardcoded22/3 auto rewards must evolve to authored roles, defeat/availability/collection and once-only credit.
- `crates/hex_arena/src/encounters/abilities.rs`: Human passive healing map != Duel must exempt Forest only; support aura currently party-local. Troll needs forest scope, not all teams.
- World tree tool assumes flat level40 and symmetrical assets. New generation must evaluate final transformed occupied cells/support and route ribbons, not trunk-center/centerline proxies.

If current source disagrees with this map, report the discrepancy. Do not implement hard summit gates, delayed Shadow spawning, full-heal fountains or old all-Goblin damage rewards.
