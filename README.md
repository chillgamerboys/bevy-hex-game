# Hex

> **A deterministic tactical RPG where every character, spell, and wound is a
> pattern of hexagons.**

![The original Hex logo: a white hexagon over a person standing on a beach](readme_assets/game_logo.jpg)

*The original 2022 logo, kept as a fond artifact of where the project started.*

**Hex** is the working title for a party-based magic game built around one shape.
A party of up to six characters explores an isometric world in real time, then
shifts into deliberate turns when a fight or timed action begins. Travel and combat
happen on the same map: a three-dimensional landscape of hexagonal prisms where
elevation, routes, sight, and terrain all matter.

The repetition is the point. The world is made of hexes, but so are characters,
elements, and spells. Instead of collecting independent abilities and watching a
health bar fall, players arrange a connected magical structure and then fight to
keep its important relationships intact.

<!--
Regenerate readme_assets/procedural-hills.png with:
HEX_WALK_SCRIPT=walks/gameplay.ron \
HEX_WALK_OUT=.context/readme-captures/gameplay \
cargo run --release -p hex_game --features visual-walk
cp .context/readme-captures/gameplay/11-hills.png \
  readme_assets/procedural-hills.png
-->
![The current Procedural Hills scenario: an elevated hex-prism landscape divided by a river and bridge, with player and enemy pieces visible](readme_assets/procedural-hills.png)

*The current Procedural Hills build. The same terrain supports exploration,
positioning, and combat.*

## Magic is geometry

Every character and enemy is defined by a **lattice**: a finite grid of hexagonal
cells containing elemental gems, spells, and fusions. Gems hold mana. A spell can
draw power only from the cells directly beside it, and a fusion combines adjacent
basic elements into a more complex ingredient.

That turns character building into a packing problem. An Ember spell needs one
neighboring fire gem; Fireball needs a complete ring of six. Sharing gems lets a
small lattice hold more spells, but those spells compete for the same mana and fail
together when a shared cell breaks. Spreading out costs more space but creates
redundancy. There is no universally best layout: efficiency and resilience are the
same geometric choice viewed from opposite sides.

![Two Fireball lattices: one castable with all six adjacent fire gems, and one offline after a required gem is disabled](readme_assets/lattice-adjacency.svg)

*Binary spells work only while every required neighbor is available.*

## Damage changes the character

There is no hit-point bar. Damage **disables lattice cells**.

Losing an expendable gem may be survivable. Losing a shared gem can silence several
spells. Breaking a fusion can disconnect everything downstream, while disabling a
gem that funds an enchantment also breaks the enchantment. The defender normally
chooses which cells to surrender, so taking damage is a sequence of tactical
decisions about what the character can still afford to be.

This also makes a character's build their body. Tight, powerful lattices are brittle;
roomy lattices endure. Variable-power rituals remain useful after binary spells have
lost a required link, giving a battered character meaningful options instead of
merely smaller numbers.

## Knowledge replaces dice

Combat resolution is deterministic: no to-hit rolls and no damage variance.
Uncertainty comes from incomplete information instead. Enemy lattices and intentions
begin hidden, while sight and Light-based divination reveal what is worth attacking
or defending.

The game does not protect a player from the consequences of positioning. Area
effects can touch allies, enemies, and their caster. Magic can also persistently
reshape terrain: a spell describes the energy and volume it applies, while the world
decides how dirt, stone, water, or another material responds. Winning should come
from understanding the board, not rerolling it.

## Where the build stands

Hex is an early playable skeleton, not the complete game described above. The current
build has deterministic procedural terrain, stacked-surface movement and path
preview, exploration and combat tempos, validated element and spell content, and a
pure lattice rules engine with an interactive demonstration.

The central connection is still missing: characters in the world do not yet carry
their lattices. In-world casting, spell effects, and lattice-based damage are not
wired into combat, so an attack currently produces an animation and log entry rather
than disabling cells. The exact, regularly updated boundary is recorded in the
[project status](docs/planning/status.md).

<!--
Regenerate readme_assets/lattice-demo-disabled-gem.png with:
HEX_WALK_SCRIPT=walks/menus.ron \
HEX_WALK_OUT=.context/readme-captures/lattice \
cargo run --release -p hex_game --features visual-walk
cp .context/readme-captures/lattice/06-demo-shield-broken.png \
  readme_assets/lattice-demo-disabled-gem.png
-->
![The interactive lattice demo after a metal gem has been disabled, leaving Metal Shield blocked and showing the broken enchantment in the event log](readme_assets/lattice-demo-disabled-gem.png)

*The interactive rules demo: disabling one funding gem has broken Metal Shield.*

## Read more

- [The full game design](docs/design/game.md) owns the intended rules and the
  questions that are deliberately still open.
- [Current status](docs/planning/status.md) separates what is built from what is
  provisional or planned.
- [Visual language](docs/design/visual-language.md) describes the art vocabulary
  growing around the hex-prism world.

## Build or contribute

Hex is written in Rust with [Bevy](https://bevy.org/) 0.19. Start with the
[setup guide](docs/development/setup.md), read [CONTRIBUTING.md](CONTRIBUTING.md)
before changing code, or use the [documentation index](docs/README.md) to find the
design, system, and development reference for a specific area.
