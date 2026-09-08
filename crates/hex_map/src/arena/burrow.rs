//! Atomic arena-only conversion. Movement authority remains with gameplay.

use hex_core::arena::{
    ArenaBurrowMaterials, ArenaBurrowOutcome, ArenaBurrowRejection, ArenaBurrowRequest,
    ArenaBurrowResult,
};

use super::*;

#[derive(SystemParam)]
pub(super) struct Channels<'w> {
    pub requests: ResMut<'w, Messages<ArenaBurrowRequest>>,
    pub outcomes: ResMut<'w, Messages<ArenaBurrowOutcome>>,
    pub policy: Res<'w, ArenaBurrowMaterials>,
}

pub(super) fn materials(substances: &SubstanceTable) -> ArenaBurrowMaterials {
    ArenaBurrowMaterials {
        eligible: (0..substances.len())
            .filter_map(|index| u16::try_from(index).ok().map(SubstanceId))
            .filter(|id| {
                substances.is_solid(*id)
                    && substances.is_diggable(*id)
                    && substances.toughness(*id).is_some()
            })
            .collect(),
    }
}

pub(super) struct Resolver<'a> {
    pub generation: u64,
    pub geometry: ArenaVoxelGeometry,
    pub original: &'a ArenaTerrainView,
    pub substances: &'a SubstanceTable,
    pub policy: &'a ArenaBurrowMaterials,
    pub dirt: SubstanceId,
}

impl Resolver<'_> {
    pub(super) fn resolve(
        &self,
        request: ArenaBurrowRequest,
        sequences: &mut BTreeMap<u8, u64>,
        map: &mut VoxelMap,
        damage: &mut TerrainDamageState,
        damaged: &mut DamagedVoxels,
    ) -> ArenaBurrowOutcome {
        let result = self.admit(&request, sequences, map).map_or_else(
            |(position, reason)| ArenaBurrowResult::Rejected { position, reason },
            |()| {
                damage
                    .convert_to_dirt(&request.volume, map, self.substances, self.dirt, damaged)
                    .map_or(
                        ArenaBurrowResult::Rejected {
                            position: None,
                            reason: ArenaBurrowRejection::InvalidDestination,
                        },
                        |changed| ArenaBurrowResult::Accepted { changed },
                    )
            },
        );
        ArenaBurrowOutcome {
            generation: request.generation,
            actor: request.actor,
            sequence: request.sequence,
            result,
        }
    }

    fn admit(
        &self,
        request: &ArenaBurrowRequest,
        sequences: &mut BTreeMap<u8, u64>,
        map: &VoxelMap,
    ) -> Result<(), (Option<TilePos>, ArenaBurrowRejection)> {
        use ArenaBurrowRejection as Reject;
        if request.generation != self.generation {
            return Err((None, Reject::StaleGeneration));
        }
        if sequences
            .get(&request.actor)
            .is_some_and(|sequence| *sequence >= request.sequence)
        {
            return Err((None, Reject::ReusedSequence));
        }
        // Rejections consume their sequence; stale generations cannot poison a
        // new generation's bounded per-actor high-water mark.
        sequences.insert(request.actor, request.sequence);
        if let Some(reason) = request.structural_rejection() {
            return Err((None, reason));
        }
        if !self.policy.eligible.contains(&self.dirt)
            || !self.substances.is_solid(self.dirt)
            || !self.substances.is_diggable(self.dirt)
            || self.substances.toughness(self.dirt).is_none()
        {
            return Err((None, Reject::InvalidDestination));
        }
        for &pos in &request.volume {
            // Requests can contain arbitrary integer identities. Widen before
            // deriving cube Z so malformed axial extremes reject, never overflow.
            let radius = u64::from(pos.coord.x().unsigned_abs())
                .max(u64::from(pos.coord.y().unsigned_abs()))
                .max((i64::from(pos.coord.x()) + i64::from(pos.coord.y())).unsigned_abs());
            let reason = if radius > u64::from(self.geometry.radius)
                || !(self.geometry.min_level..=self.geometry.max_level).contains(&pos.level)
            {
                Some(Reject::OutsideWorld)
            } else if self
                .original
                .edit_protected
                .get(&pos.coord)
                .is_some_and(|intervals| {
                    intervals
                        .iter()
                        .any(|(low, high)| (*low..=*high).contains(&pos.level))
                })
            {
                Some(Reject::Protected)
            } else if self.original.liquids.iter().any(|span| {
                span.bottom.coord == pos.coord
                    && (span.bottom.level..=span.top_level).contains(&pos.level)
            }) {
                Some(Reject::Liquid)
            } else if self.original.static_spans.iter().any(|span| {
                span.bottom.coord == pos.coord
                    && (span.bottom.level..=span.top_level).contains(&pos.level)
            }) {
                // All authored occupancy counts, even when every ordinary mask
                // is false. Protected/static air is never a burrow corridor.
                Some(Reject::StaticObject)
            } else {
                let material = map.get(pos);
                (!material.is_air() && !self.policy.eligible.contains(&material))
                    .then_some(Reject::IneligibleMaterial)
            };
            if let Some(reason) = reason {
                return Err((Some(pos), reason));
            }
        }
        Ok(())
    }
}
