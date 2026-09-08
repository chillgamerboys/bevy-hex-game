//! Coarse, discrete combat knowledge and exact read-only round accounting.

use bevy_math::Vec3;
use serde::Serialize;

use crate::{ArenaOutcome, ArenaSession, BotDebugSnapshot, Spell, STEP};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CombatCueKind {
    Release,
    Impact,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CombatCue {
    pub id: u64,
    pub tick: u64,
    pub owner: u8,
    pub team: crate::TeamId,
    pub position: Vec3,
    pub kind: CombatCueKind,
}

/// Actual spell releases and HP losses, independent of presentation or bot estimates.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct ActorCombatStats {
    /// Shield, Fireball, and Area Blast releases, including unsuccessful casts.
    pub casts: [u32; 3],
    /// HP removed from the opponent, capped by their remaining life at each impact.
    pub damage_dealt: f32,
    /// HP removed by all spells, including self-damage.
    pub damage_received: f32,
    /// Portion of received damage caused by this actor's own spells.
    pub self_damage: f32,
    /// Fireballs that reached an explosion; excludes shots still flying or expired.
    pub fireballs_resolved: u32,
    /// Resolved Fireballs whose falloff would remove at least 40% of maximum
    /// damage from the living opponent, before clamping to remaining HP.
    pub useful_fireballs: u32,
    /// First tick on which this actor damaged the opponent.
    pub first_damage_tick: Option<u64>,
}

/// Compact snapshot for local playtest logs and deterministic comparisons.
#[derive(Debug, Clone, Serialize)]
pub struct RoundSummary {
    /// Simulation duration, excluding menus and pauses.
    pub seconds: f64,
    /// Winning actor ID, if a single actor won.
    pub winner: Option<u8>,
    /// True for a terminal round, including simultaneous knockout.
    pub complete: bool,
    /// Human then bot combat accounting.
    pub actors: [ActorCombatStats; 2],
}

impl ArenaSession {
    /// Select the frozen pre-memory bot for paired tests with identical spell rules.
    #[cfg(any(test, feature = "test-support"))]
    pub fn use_baseline_bot(&mut self, enabled: bool) {
        self.cancel_charges();
        self.baseline_bot = enabled.then(crate::bot_baseline::Bot::default);
        self.bot_enabled = true;
    }

    /// Inspect remembered information and decisions without changing the opponent.
    #[must_use]
    pub fn bot_debug(&self) -> BotDebugSnapshot {
        self.bot.debug()
    }

    /// Read a compact live or terminal round summary; reset clears all counters.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "arena tick counts are short local rounds"
    )]
    pub fn round_summary(&self) -> RoundSummary {
        RoundSummary {
            seconds: self.tick as f64 * f64::from(STEP),
            winner: match self.outcome {
                Some(ArenaOutcome::Winner(id)) => Some(id),
                _ => None,
            },
            complete: self.outcome.is_some(),
            actors: self.combat_stats,
        }
    }

    pub(super) fn combat_cue(&mut self, owner: u8, point: Vec3, kind: CombatCueKind) {
        let Some(team) = self
            .actors
            .iter()
            .find(|actor| actor.id == owner)
            .map(|actor| actor.team)
        else {
            return;
        };
        self.combat_cue_from(owner, team, point, kind);
    }

    pub(super) fn combat_cue_from(
        &mut self,
        owner: u8,
        team: crate::TeamId,
        point: Vec3,
        kind: CombatCueKind,
    ) {
        // Quantization belongs to the knowledge publication, not rendering. No cue
        // contains a hidden actor's live position after the discrete event.
        let position = (point / 2.0).round() * 2.0;
        self.combat_cues.push(CombatCue {
            id: self.next_cue,
            tick: self.tick,
            owner,
            team,
            position,
            kind,
        });
        self.next_cue = self.next_cue.wrapping_add(1);
        if self.combat_cues.len() > 64 {
            self.combat_cues.remove(0);
        }
    }

    pub(super) fn record_cast(&mut self, owner: u8, spell: Spell) {
        if let Some(actor) = self.actors.iter_mut().find(|a| a.id == owner) {
            actor.last_activity_tick = self.tick;
        }
        if self.encounter.initialized {
            let ability = match spell {
                Spell::Fireball => Some(0),
                Spell::Shield => Some(1),
                Spell::AreaBlast => None,
            };
            if let Some(index) = ability {
                if let Some(count) = self
                    .encounter
                    .ability_counts
                    .entry(owner)
                    .or_insert([0; 7])
                    .get_mut(index)
                {
                    *count += 1;
                }
            }
            if let Some(count) = self
                .encounter
                .stats
                .entry(owner)
                .or_default()
                .casts
                .get_mut(spell.index())
            {
                *count += 1;
            }
        }
        if let Some(count) = self
            .combat_stats
            .get_mut(usize::from(owner))
            .and_then(|stats| stats.casts.get_mut(spell.index()))
        {
            *count += 1;
        }
    }

    pub(super) fn record_damage(&mut self, owner: u8, victim: u8, amount: f32) {
        if amount > 0.0 {
            if let Some(actor) = self.actors.iter_mut().find(|a| a.id == victim) {
                actor.last_damage_tick = Some(self.tick);
                actor.last_activity_tick = self.tick;
            }
            if let Some(actor) = self.actors.iter_mut().find(|a| a.id == owner) {
                actor.last_activity_tick = self.tick;
            }
            self.wake_encounter_damage(owner, victim, amount);
        }
        if self.encounter.initialized {
            let stats = self.encounter.stats.entry(victim).or_default();
            stats.damage_received += amount;
            if owner == victim {
                stats.self_damage += amount;
            } else if amount > 0.0 {
                let stats = self.encounter.stats.entry(owner).or_default();
                stats.damage_dealt += amount;
                stats.first_damage_tick.get_or_insert(self.tick);
            }
        }
        if let Some(stats) = self.combat_stats.get_mut(usize::from(victim)) {
            stats.damage_received += amount;
            if owner == victim {
                stats.self_damage += amount;
            }
        }
        if owner != victim && amount > 0.0 {
            if let Some(stats) = self.combat_stats.get_mut(usize::from(owner)) {
                stats.damage_dealt += amount;
                stats.first_damage_tick.get_or_insert(self.tick);
            }
        }
    }

    pub(super) fn record_fireball_impact(&mut self, owner: u8, useful: bool) {
        if self.encounter.initialized {
            let stats = self.encounter.stats.entry(owner).or_default();
            stats.fireballs_resolved += 1;
            stats.useful_fireballs += u32::from(useful);
        }
        if let Some(stats) = self.combat_stats.get_mut(usize::from(owner)) {
            stats.fireballs_resolved += 1;
            stats.useful_fireballs += u32::from(useful);
        }
    }
}
