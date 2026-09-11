//! World-published damage facts and explicitly temporary terrain forecasts.

use std::collections::{BTreeMap, BTreeSet};

use bevy_math::Vec3;

use super::{ArenaSolidSpan, ArenaTerrainView, ArenaVoxelGeometry};
use crate::{
    ElementId, HexCoord, SubstanceId, TerrainDamageKind, TerrainImpactDisposition,
    TerrainVoxelHealth, TerrainVoxelOutcome, TilePos,
};

/// Accepted material properties needed for exact damage forecasts.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArenaDamageMaterial {
    /// Authored maximum voxel HP; absence means indestructible.
    pub toughness: Option<u8>,
    /// Whether ordinary damage may alter this material.
    pub diggable: bool,
    /// Admitted elemental attacks; missing entries resist.
    pub elements: BTreeSet<ElementId>,
    /// Whether physical contact may damage this material.
    pub physical: bool,
}

impl ArenaDamageMaterial {
    /// Admission before exact-position protection is checked.
    #[must_use]
    pub fn admits(&self, kind: TerrainDamageKind) -> bool {
        self.diggable
            && self.toughness.is_some()
            && match kind {
                TerrainDamageKind::Elemental(element) => self.elements.contains(&element),
                TerrainDamageKind::Physical => self.physical,
            }
    }
}

/// Read-only world publication; no presence or visibility grant is implied.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArenaDamageView {
    /// Changes for HP-only mutations as well as material changes and reset.
    pub revision: u64,
    /// Accepted Fire element used by the arena's ordinary Fireballs.
    pub fire: Option<ElementId>,
    /// Complete accepted damage policy keyed by material.
    pub materials: BTreeMap<SubstanceId, ArenaDamageMaterial>,
    /// Sparse partial health; absence means the material's current maximum.
    pub partial: BTreeMap<TilePos, TerrainVoxelHealth>,
}

impl ArenaDamageView {
    /// Exact remaining health according to the accepted material and sparse ledger.
    #[must_use]
    pub fn health(&self, substance: SubstanceId, pos: TilePos) -> Option<TerrainVoxelHealth> {
        let maximum = self.materials.get(&substance)?.toughness?;
        let remaining = self.partial.get(&pos).map_or(maximum, |hp| hp.remaining);
        TerrainVoxelHealth::new(remaining.min(maximum), maximum)
    }
}

/// Pure per-voxel damage resolution shared by world authority and previews.
///
/// The world supplies material/health facts and its complete admission decision.
/// This function changes no state. An unadmitted or zero-power hit resists.
#[must_use]
pub fn resolve_terrain_voxel_damage(
    pos: TilePos,
    substance: Option<SubstanceId>,
    health: Option<TerrainVoxelHealth>,
    admitted: bool,
    power: u8,
) -> TerrainVoxelOutcome {
    let mut result = TerrainVoxelOutcome {
        pos,
        disposition: TerrainImpactDisposition::Resisted,
        before: substance,
        after: substance,
        health_before: health,
        health_after: health,
    };
    if substance.is_none_or(SubstanceId::is_air) {
        result.disposition = TerrainImpactDisposition::NoMaterial;
        result.before = None;
        result.after = None;
        result.health_before = None;
        result.health_after = None;
    } else if admitted && power > 0 {
        if let Some(health) = health.filter(|health| health.is_valid()) {
            if power >= health.remaining {
                result.disposition = TerrainImpactDisposition::Destroyed;
                result.after = None;
                result.health_after = None;
            } else {
                result.disposition = TerrainImpactDisposition::Damaged;
                result.health_after = Some(TerrainVoxelHealth {
                    remaining: health.remaining - power,
                    maximum: health.maximum,
                });
            }
        }
    }
    result
}

impl ArenaTerrainView {
    /// Copies bounded columns for temporary planning, never authoritative mutation.
    ///
    /// A conservative axial window includes the requested horizontal radius and
    /// complete vertical columns. Requests are capped at 32 world units. Collision
    /// outside this copy is unknown: planners must keep every rollout inside it.
    #[must_use]
    pub fn local_damage_preview(
        &self,
        geometry: ArenaVoxelGeometry,
        center: Vec3,
        radius: f32,
    ) -> Self {
        let mut copy = Self {
            revision: self.revision,
            damage: ArenaDamageView {
                revision: self.damage.revision,
                fire: self.damage.fire,
                materials: self.damage.materials.clone(),
                partial: BTreeMap::new(),
            },
            spawns: self.spawns,
            selection: self.selection,
            full_rebuild: true,
            ..Self::default()
        };
        if !center.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return copy;
        }
        let middle = HexCoord::from_world(center);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "finite radius is capped at 32"
        )]
        let reach = radius.min(32.0).ceil() as i32 + 2;
        for q in middle.x().saturating_sub(reach)..=middle.x().saturating_add(reach) {
            for r in middle.y().saturating_sub(reach)..=middle.y().saturating_add(reach) {
                let coord = HexCoord::from_axial(q, r);
                if !geometry.contains_column(coord) {
                    continue;
                }
                let range = TilePos::new(coord, i32::MIN)..=TilePos::new(coord, i32::MAX);
                copy.voxels
                    .extend(self.voxels.range(range.clone()).map(|(p, s)| (*p, *s)));
                copy.damage
                    .partial
                    .extend(self.damage.partial.range(range).map(|(p, h)| (*p, *h)));
                if let Some(spans) = self.columns.get(&coord) {
                    copy.columns.insert(coord, spans.clone());
                }
                if let Some(protection) = self.edit_protected.get(&coord) {
                    copy.edit_protected.insert(coord, protection.clone());
                }
                copy.dirty_columns.insert(coord);
            }
        }
        copy.static_spans.extend(
            self.static_spans
                .iter()
                .filter(|span| copy.dirty_columns.contains(&span.bottom.coord))
                .copied(),
        );
        copy.liquids.extend(
            self.liquids
                .iter()
                .filter(|span| copy.dirty_columns.contains(&span.bottom.coord))
                .copied(),
        );
        copy
    }

    /// Applies projected damage only to a caller-owned temporary copy.
    ///
    /// The authoritative `ArenaTerrainView` must never call this method. Runtime
    /// terrain writes remain `TerrainImpact`/`TerrainEdit`. Duplicate input cells
    /// count once; results are canonical. Collision columns refresh only on removal.
    pub fn project_damage(
        &mut self,
        volume: &[TilePos],
        kind: TerrainDamageKind,
        power: u8,
    ) -> Vec<TerrainVoxelOutcome> {
        let mut changed = BTreeSet::new();
        let mut health_changed = false;
        let outcomes = volume
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|pos| {
                let substance = self.voxels.get(&pos).copied();
                let health = substance.and_then(|substance| self.damage.health(substance, pos));
                let protected = self
                    .edit_protected
                    .get(&pos.coord)
                    .is_some_and(|intervals| {
                        intervals
                            .iter()
                            .any(|(bottom, top)| (*bottom..=*top).contains(&pos.level))
                    });
                let admitted = !protected
                    && substance
                        .and_then(|substance| self.damage.materials.get(&substance))
                        .is_some_and(|material| material.admits(kind));
                let outcome = resolve_terrain_voxel_damage(pos, substance, health, admitted, power);
                match outcome.disposition {
                    TerrainImpactDisposition::Destroyed => {
                        self.voxels.remove(&pos);
                        self.damage.partial.remove(&pos);
                        changed.insert(pos.coord);
                        health_changed = true;
                    }
                    TerrainImpactDisposition::Damaged => {
                        if let Some(health) = outcome.health_after {
                            self.damage.partial.insert(pos, health);
                        }
                        health_changed = true;
                    }
                    TerrainImpactDisposition::NoMaterial | TerrainImpactDisposition::Resisted => {}
                }
                outcome
            })
            .collect();
        if health_changed {
            self.damage.revision = self.damage.revision.saturating_add(1);
        }
        if !changed.is_empty() {
            for coord in &changed {
                self.rebuild_preview_column(*coord);
            }
            self.revision = self.revision.saturating_add(1);
            self.dirty_columns = changed;
        }
        outcomes
    }

    fn rebuild_preview_column(&mut self, coord: HexCoord) {
        let mut spans: Vec<ArenaSolidSpan> = Vec::new();
        let range = TilePos::new(coord, i32::MIN)..=TilePos::new(coord, i32::MAX);
        for (pos, substance) in self.voxels.range(range) {
            if let Some(last) = spans.last_mut().filter(|span| {
                span.substance == *substance && span.top_level.checked_add(1) == Some(pos.level)
            }) {
                last.top_level = pos.level;
            } else {
                spans.push(ArenaSolidSpan {
                    bottom: *pos,
                    top_level: pos.level,
                    substance: *substance,
                });
            }
        }
        if spans.is_empty() {
            self.columns.remove(&coord);
        } else {
            self.columns.insert(coord, spans);
        }
    }
}
