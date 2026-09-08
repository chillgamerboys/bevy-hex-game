//! Gameplay-owned battle setup and observer projections; world selection stays separate.

use bevy_ecs::prelude::Resource;
use hex_core::arena::ArenaMap;
use serde::{Deserialize, Serialize};

use crate::{Species, TeamId};

/// Initial hard bound for one observer battle, checked before actors are spawned.
pub const MAX_BATTLE_ACTORS: usize = 24;

/// Whether the run has a controlled human body or only monster teams.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArenaControl {
    /// Preserve the selected map's ordinary human encounter rules.
    #[default]
    Player,
    /// Observe autonomous opposing monster teams without a dummy human.
    Spectator,
}

/// Shared roster recipes for the start screen, launcher and calibration harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BattlePreset {
    /// One accepted Shadow opponent.
    Shadow,
    /// One low flying Dragon.
    Dragon,
    /// Five melee Goblins.
    Goblins,
    /// One Shaman and three Goblins in the same support party.
    ShamanParty,
    /// One seven-hex stone Golem.
    Golem,
    /// One player-sized Goblin for creature-control comparisons.
    Goblin,
    /// One weak flying Wisp.
    Wisp,
    /// Two flying Wisps.
    Wisps2,
    /// Four flying Wisps.
    Wisps4,
    /// Eight flying Wisps.
    Wisps8,
    /// Twelve flying Wisps, the largest two-team comparison under the actor cap.
    Wisps12,
}

impl BattlePreset {
    /// Initial selectable recipes, in stable presentation order.
    pub const ALL: [Self; 11] = [
        Self::Shadow,
        Self::Dragon,
        Self::Goblins,
        Self::ShamanParty,
        Self::Golem,
        Self::Goblin,
        Self::Wisp,
        Self::Wisps2,
        Self::Wisps4,
        Self::Wisps8,
        Self::Wisps12,
    ];

    /// Frozen count sweep for the Wisp crossover, with no assumed equivalence.
    pub const WISP_SWARMS: [Self; 5] = [
        Self::Wisp,
        Self::Wisps2,
        Self::Wisps4,
        Self::Wisps8,
        Self::Wisps12,
    ];
    /// One provisional Wisp menu recipe; explicit/observer selection uses ALL.
    pub const PLAYER: [Self; 6] = [
        Self::Shadow,
        Self::Dragon,
        Self::Goblins,
        Self::ShamanParty,
        Self::Golem,
        Self::Wisps4,
    ];

    /// Frozen original-group comparison corpus; later creatures are calibrated separately.
    pub const ORIGINAL: [Self; 4] = [Self::Shadow, Self::Dragon, Self::Goblins, Self::ShamanParty];

    /// Concise presentation label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Shadow => "Shadow",
            Self::Dragon => "Dragon",
            Self::Goblins => "5 Goblins",
            Self::ShamanParty => "Shaman + 3 Goblins",
            Self::Golem => "Golem",
            Self::Goblin => "Goblin",
            Self::Wisp => "Wisp",
            Self::Wisps2 => "2 Wisps",
            Self::Wisps4 => "4 Wisps",
            Self::Wisps8 => "8 Wisps",
            Self::Wisps12 => "12 Wisps",
        }
    }

    /// Stable command-line spelling, independent of display labels.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Shadow => "shadow",
            Self::Dragon => "dragon",
            Self::Goblins => "goblins",
            Self::ShamanParty => "shaman-party",
            Self::Golem => "golem",
            Self::Goblin => "goblin",
            Self::Wisp => "wisp",
            Self::Wisps2 => "wisps-2",
            Self::Wisps4 => "wisps-4",
            Self::Wisps8 => "wisps-8",
            Self::Wisps12 => "wisps-12",
        }
    }

    /// Parse one exact supported command-line recipe.
    #[must_use]
    pub fn from_slug(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.slug() == value)
    }

    /// Species belonging to the recipe's single party.
    #[must_use]
    pub fn members(self) -> Vec<Species> {
        match self {
            Self::Shadow => vec![Species::Shadow],
            Self::Dragon => vec![Species::Dragon],
            Self::Goblins => vec![Species::Goblin; 5],
            Self::ShamanParty => vec![
                Species::Shaman,
                Species::Goblin,
                Species::Goblin,
                Species::Goblin,
            ],
            Self::Golem => vec![Species::Golem],
            Self::Goblin => vec![Species::Goblin],
            Self::Wisp => vec![Species::Wisp],
            Self::Wisps2 => vec![Species::Wisp; 2],
            Self::Wisps4 => vec![Species::Wisp; 4],
            Self::Wisps8 => vec![Species::Wisp; 8],
            Self::Wisps12 => vec![Species::Wisp; 12],
        }
    }
}

/// One allegiance with one or more independent support parties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamRoster {
    /// Stable allegiance, not inferred from actor IDs or roster order.
    pub team: TeamId,
    /// Each inner vector is one party; Shaman support stays inside that party.
    pub parties: Vec<Vec<Species>>,
}

impl TeamRoster {
    /// Build one party using a supported recipe.
    #[must_use]
    pub fn from_preset(team: TeamId, preset: BattlePreset) -> Self {
        Self {
            team,
            parties: vec![preset.members()],
        }
    }

    /// Total bodies that this team will require at reset.
    #[must_use]
    pub fn actor_count(&self) -> usize {
        self.parties.iter().map(Vec::len).sum()
    }
}

/// Configuration consumed only when the ordinary arena reset generation changes.
#[derive(Debug, Clone, PartialEq, Eq, Resource, Serialize, Deserialize)]
pub struct ArenaBattleSetup {
    /// Ordinary human encounter or autonomous observer match.
    pub control: ArenaControl,
    /// Optional Fort player opponent recipe, consumed only on reset. None keeps the world recipe.
    #[serde(default)]
    pub player_recipe: Option<BattlePreset>,
    /// Exactly two distinct teams are admitted in the initial spectator mode.
    pub rosters: Vec<TeamRoster>,
    /// Replay seed for monster decisions; does not alter the accepted Duel default seed.
    pub seed: u64,
    /// Optional simulation-tick bound; reaching it is a timeout, never an elimination draw.
    pub tick_limit: Option<u64>,
}

impl Default for ArenaBattleSetup {
    fn default() -> Self {
        Self {
            control: ArenaControl::Player,
            player_recipe: None,
            rosters: vec![
                TeamRoster::from_preset(1, BattlePreset::Shadow),
                TeamRoster::from_preset(2, BattlePreset::Dragon),
            ],
            seed: 1,
            tick_limit: Some(14_400),
        }
    }
}

impl ArenaBattleSetup {
    /// Construct a two-minute observer match using two original recipes.
    #[must_use]
    pub fn spectator(left: BattlePreset, right: BattlePreset, seed: u64) -> Self {
        Self {
            control: ArenaControl::Spectator,
            rosters: vec![
                TeamRoster::from_preset(1, left),
                TeamRoster::from_preset(2, right),
            ],
            seed,
            ..Self::default()
        }
    }

    /// Validate the bounded actor/party contract before resolving physical spawn positions.
    pub fn validate_for(&self, map: ArenaMap) -> Result<(), BattleSetupError> {
        if self.control == ArenaControl::Player {
            if self.player_recipe.is_some() && map != ArenaMap::Fort {
                return Err(BattleSetupError::PlayerRecipeMap);
            }
            if self
                .player_recipe
                .is_some_and(|recipe| BattlePreset::WISP_SWARMS.contains(&recipe))
            {
                return Err(BattleSetupError::CreatureNotReady);
            }
            return Ok(());
        }
        if self.player_recipe.is_some() {
            return Err(BattleSetupError::PlayerRecipeInSpectator);
        }
        if !matches!(map, ArenaMap::Duel | ArenaMap::Fort) {
            return Err(BattleSetupError::UnsupportedMap);
        }
        if self.rosters.len() != 2 {
            return Err(BattleSetupError::TeamCount);
        }
        if self
            .rosters
            .first()
            .zip(self.rosters.get(1))
            .is_some_and(|(a, b)| a.team == b.team)
        {
            return Err(BattleSetupError::DuplicateTeams);
        }
        if self
            .rosters
            .iter()
            .any(|team| team.parties.is_empty() || team.parties.iter().any(Vec::is_empty))
        {
            return Err(BattleSetupError::EmptyParty);
        }
        if self
            .rosters
            .iter()
            .flat_map(|team| team.parties.iter().flatten())
            .any(|species| *species == Species::Human)
        {
            return Err(BattleSetupError::HumanInRoster);
        }
        if self
            .rosters
            .iter()
            .map(TeamRoster::actor_count)
            .sum::<usize>()
            > MAX_BATTLE_ACTORS
        {
            return Err(BattleSetupError::TooManyActors);
        }
        if self
            .rosters
            .iter()
            .flat_map(|team| team.parties.iter().flatten())
            .any(|species| *species == Species::Wisp)
        {
            return Err(BattleSetupError::CreatureNotReady);
        }
        if self.tick_limit == Some(0) {
            return Err(BattleSetupError::ZeroTickLimit);
        }
        Ok(())
    }
}

/// Setup refusal; failure to place valid bodies is a separate runtime diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BattleSetupError {
    /// Wisp schema is visible while its full body, flight and shot authority is integrated.
    CreatureNotReady,
    /// Explicit player recipes are currently authored only for Fort.
    PlayerRecipeMap,
    /// Observer rosters cannot also request a player encounter recipe.
    PlayerRecipeInSpectator,
    /// This first version admits observer fights on Fort and the original Duel arena.
    UnsupportedMap,
    /// Exactly two teams are required.
    TeamCount,
    /// Both rosters use the same allegiance.
    DuplicateTeams,
    /// A team or one of its parties has no members.
    EmptyParty,
    /// Spectator mode cannot create a controlled-player species.
    HumanInRoster,
    /// Roster exceeds the bounded simulation capacity.
    TooManyActors,
    /// A finite tick limit must permit at least one tick.
    ZeroTickLimit,
}

impl std::fmt::Display for BattleSetupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::CreatureNotReady => "Wisp flight and attacks are still being integrated.",
            Self::PlayerRecipeMap => "Player opponent recipes support Fort only.",
            Self::PlayerRecipeInSpectator => {
                "Spectator battles use team rosters, not a player recipe."
            }
            Self::UnsupportedMap => "Spectator battles support Fort and Duel.",
            Self::TeamCount => "Choose exactly two monster teams.",
            Self::DuplicateTeams => "The two teams must have different allegiances.",
            Self::EmptyParty => "Every team and party needs at least one creature.",
            Self::HumanInRoster => "Spectator rosters can contain only monsters.",
            Self::TooManyActors => "A spectator battle supports at most 24 creatures.",
            Self::ZeroTickLimit => "The battle time limit must allow at least one tick.",
        })
    }
}

impl std::error::Error for BattleSetupError {}

/// Observer-only terminal result; the legacy human/Duel result keeps its meaning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BattleResult {
    /// The sole team with surviving actors after the full tick resolves.
    TeamWinner(TeamId),
    /// Both teams were eliminated in the same completed tick.
    Draw,
    /// The configured simulation time elapsed with more than one team alive.
    Timeout,
    /// Configuration or physical deployment could not produce a valid match.
    InvalidSetup(String),
}

/// Compact aggregate for the omniscient observer HUD and local calibration receipts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BattleTeamSummary {
    /// Stable allegiance.
    pub team: TeamId,
    /// Number of actors admitted at match start.
    pub initial: usize,
    /// Number of actors still alive.
    pub living: usize,
    /// Remaining living actor HP.
    pub hp: f32,
    /// Sum of the admitted maximum HP for the whole team.
    pub max_hp: f32,
}

/// Read-only battle projection. Ordinary player HUDs must not display this information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BattleSummary {
    /// Replay seed actually consumed at reset.
    pub seed: u64,
    /// Completed battle simulation ticks.
    pub ticks: u64,
    /// Elapsed simulation seconds, independent of renderer timing.
    pub seconds: f32,
    /// Stable roster-order team summaries.
    pub teams: Vec<BattleTeamSummary>,
    /// None while the battle is active.
    pub result: Option<BattleResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_rosters_preserve_custom_teams_and_party_boundaries() {
        let setup = ArenaBattleSetup {
            rosters: vec![
                TeamRoster {
                    team: 7,
                    parties: vec![
                        vec![Species::Shaman, Species::Goblin],
                        vec![Species::Dragon],
                    ],
                },
                TeamRoster::from_preset(42, BattlePreset::Shadow),
            ],
            seed: u64::MAX,
            tick_limit: None,
            ..ArenaBattleSetup::spectator(BattlePreset::Dragon, BattlePreset::Shadow, 1)
        };
        let text = ron::to_string(&setup).expect("serialize battle setup");
        let decoded: ArenaBattleSetup = ron::from_str(&text).expect("deserialize battle setup");
        assert_eq!(decoded, setup);
        assert_eq!(decoded.validate_for(ArenaMap::Fort), Ok(()));
    }

    #[test]
    fn observer_admission_rejects_invalid_rosters_before_spawning() {
        let valid = ArenaBattleSetup::spectator(BattlePreset::Shadow, BattlePreset::Goblins, 3);
        assert_eq!(valid.validate_for(ArenaMap::Duel), Ok(()));
        assert_eq!(
            valid.validate_for(ArenaMap::SevenRegions),
            Err(BattleSetupError::UnsupportedMap)
        );
        let cases = [
            (Vec::new(), BattleSetupError::TeamCount),
            (
                vec![TeamRoster::from_preset(7, BattlePreset::Shadow); 2],
                BattleSetupError::DuplicateTeams,
            ),
            (
                vec![
                    TeamRoster {
                        team: 7,
                        parties: vec![vec![]],
                    },
                    TeamRoster::from_preset(8, BattlePreset::Dragon),
                ],
                BattleSetupError::EmptyParty,
            ),
            (
                vec![
                    TeamRoster {
                        team: 7,
                        parties: vec![vec![Species::Human]],
                    },
                    TeamRoster::from_preset(8, BattlePreset::Dragon),
                ],
                BattleSetupError::HumanInRoster,
            ),
            (
                vec![
                    TeamRoster {
                        team: 7,
                        parties: vec![vec![Species::Goblin; MAX_BATTLE_ACTORS]],
                    },
                    TeamRoster::from_preset(8, BattlePreset::Dragon),
                ],
                BattleSetupError::TooManyActors,
            ),
        ];
        for (rosters, expected) in cases {
            let setup = ArenaBattleSetup {
                rosters,
                ..valid.clone()
            };
            assert_eq!(setup.validate_for(ArenaMap::Fort), Err(expected));
        }
        let zero = ArenaBattleSetup {
            tick_limit: Some(0),
            ..valid
        };
        assert_eq!(
            zero.validate_for(ArenaMap::Fort),
            Err(BattleSetupError::ZeroTickLimit)
        );
        let player = ArenaBattleSetup {
            control: ArenaControl::Player,
            rosters: vec![],
            ..zero
        };
        assert_eq!(player.validate_for(ArenaMap::SevenRegions), Ok(()));
    }

    #[test]
    fn capacity_boundary_admits_twenty_four_and_preserves_preset_spelling() {
        let setup = ArenaBattleSetup {
            rosters: vec![
                TeamRoster {
                    team: 1,
                    parties: vec![vec![Species::Goblin; 12]],
                },
                TeamRoster {
                    team: 2,
                    parties: vec![vec![Species::Goblin; 12]],
                },
            ],
            ..ArenaBattleSetup::spectator(BattlePreset::Goblins, BattlePreset::Goblins, 1)
        };
        assert_eq!(setup.validate_for(ArenaMap::Fort), Ok(()));
        for preset in BattlePreset::ALL {
            assert_eq!(BattlePreset::from_slug(preset.slug()), Some(preset));
        }
        assert_eq!(BattlePreset::from_slug("unknown"), None);
    }
}
