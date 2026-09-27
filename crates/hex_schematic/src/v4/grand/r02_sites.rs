//! Exact gameplay supports for the layered revision-02 geography.
use super::*;
use sites::{Encounter, Fountain, GrandSites, Node, Route};
const ENCOUNTERS: &[(&str, [f64; 2], Option<f64>)] = &[
    ("grand_goblin_01", [-35., -40.], None),
    ("grand_goblin_02", [-270., -160.], None),
    ("grand_goblin_03", [90., -220.], None),
    ("grand_goblin_04", [-335., -35.], None),
    ("grand_goblin_05", [-100., -260.], None),
    ("grand_goblin_06", [90., -55.], None),
    ("grand_shaman_01", [195., -45.], None),
    ("grand_golem_01", [-460., 500.], Some(135.)),
    ("grand_wisp_01", [-470., 485.], Some(300.)),
    ("grand_worm_01", [-205., 525.], Some(65.)),
    ("grand_worm_02", [-140., 585.], Some(65.)),
    ("grand_worm_03", [-170., 570.], Some(65.)),
    ("grand_shadow_tunnel", [-170., 340.], Some(18.)),
];
impl GrandCompiler {
    pub(super) fn r02_support(
        &self,
        point: [f64; 2],
        height: Option<f64>,
    ) -> Result<VoxelPosition, ContractError> {
        self.r02_support_column(
            self.geography.world_hex(point),
            height.map(|y| self.geography.top_level(y)),
        )
    }
    fn r02_support_column(
        &self,
        p: WorldHex,
        top: Option<i32>,
    ) -> Result<VoxelPosition, ContractError> {
        let (c, _) = self.column(p);
        let mut levels: Vec<_> = c
            .runs
            .iter()
            .filter(|r| r.material != "water")
            .map(|r| r.top - 1)
            .collect();
        if let Some(top) = top {
            levels.sort_by_key(|level| (level + 1 - top).abs());
        } else {
            levels.sort_by(|a, b| b.cmp(a));
        }
        levels
            .into_iter()
            .find(|&level| self.clear_support(VoxelPosition { column: p, level }, 8))
            .map(|level| VoxelPosition { column: p, level })
            .ok_or_else(|| ContractError::new("grand.site", format!("no clear support at {p:?}")))
    }
    pub(super) fn r02_anchors(&self) -> Result<Vec<WorldAnchor>, ContractError> {
        let d = self
            .geography
            .document
            .as_ref()
            .ok_or_else(|| ContractError::new("grand.site", "missing document"))?;
        let mut out = Vec::new();
        for (id, frame_id, gameplay) in [
            ("shrine_water", "shrine_water", true),
            ("shrine_air", "shrine_air", true),
            ("shrine_earth", "shrine_earth", true),
            ("shrine_plant", "shrine_plant", true),
            ("shrine_fire", "shrine_fire", true),
            ("garden", "garden", false),
            ("world_tree", "world_tree", false),
            ("library_hall", "library_lower", true),
            ("library_upper", "library_upper", true),
            ("library_entrance", "library_entrance", true),
            ("root_temple_entrance", "root_temple_entrance", false),
        ] {
            let frame = self.geography.frame(frame_id)?;
            let position = self.support_at(&frame, [0., 0.])?;
            out.push(WorldAnchor {
                id: format!("grand/anchor/{id}"),
                region_id: "grand".into(),
                position,
                role: if gameplay {
                    AnchorRole::Gameplay
                } else {
                    AnchorRole::Observation
                },
            });
        }
        for (id, point, height, gameplay) in [
            ("party_start", [-105., -495.], None, true),
            ("forest", [-100., -120.], None, false),
            ("goblin_fort", [-35., -40.], None, false),
            ("root_temple_approach", [195., -35.], Some(46.), false),
            ("shadow_entrance", [-170., 115.], Some(58.8), true),
            ("shadow_tunnel", [-170., 340.], Some(18.), true),
            ("shadow_exit", [-170., 615.], Some(65.), true),
            ("waterfall", [508., 442.], Some(114.), false),
            ("mountain_lake", [216., 650.], Some(206.), false),
            ("valley_lake", [202., 145.], None, false),
            ("bay", [-105., -520.], None, false),
            ("volcano", d.volcano.center, None, false),
        ] {
            out.push(WorldAnchor {
                id: format!("grand/anchor/{id}"),
                region_id: "grand".into(),
                position: self.r02_support(point, height)?,
                role: if gameplay {
                    AnchorRole::Gameplay
                } else {
                    AnchorRole::Observation
                },
            });
        }
        for (id, route_id, last) in [
            ("crystal_ascent", "crystal_ascent", false),
            ("frozen_woods", "crystal_ascent", true),
            ("volcano_landing", "volcano_ascent", false),
        ] {
            let route = self
                .layered
                .routes
                .iter()
                .find(|r| r.id == route_id)
                .ok_or_else(|| ContractError::new("grand.site", "missing required route"))?;
            let position = if last {
                route.supports.last()
            } else {
                route.supports.first()
            }
            .copied()
            .ok_or_else(|| ContractError::new("grand.site", "empty required route"))?;
            out.push(WorldAnchor {
                id: format!("grand/anchor/{id}"),
                region_id: "grand".into(),
                position,
                role: AnchorRole::Observation,
            });
        }
        // Starting boat position lies just offshore from the authored mainland beach.
        out.push(WorldAnchor {
            id: "grand/anchor/sailing_start".into(),
            region_id: "grand".into(),
            position: VoxelPosition {
                column: self.geography.world_hex(d.sailing_start),
                level: SEA_TOP - 1,
            },
            role: AnchorRole::Observation,
        });
        out.push(WorldAnchor {
            id: "grand/anchor/sailing_start_bay".into(),
            region_id: "grand".into(),
            position: VoxelPosition {
                column: self.geography.world_hex(d.sailing_start_bay),
                level: SEA_TOP - 1,
            },
            role: AnchorRole::Observation,
        });
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }
    pub(super) fn r02_sites(&self, manifest_fingerprint: u64) -> Result<GrandSites, ContractError> {
        let mut definitions = ENCOUNTERS.to_vec();
        let d = self
            .geography
            .document
            .as_ref()
            .ok_or_else(|| ContractError::new("grand.site", "missing geography"))?;
        definitions.push((
            "grand_dragon_01",
            [d.volcano.center[0] + 100., d.volcano.center[1] + 120.],
            None,
        ));
        let mut encounters = Vec::new();
        let mut failures = Vec::new();
        for (id, point, height) in definitions {
            let preferred = self.r02_support(point, height)?;
            let mut surfaces = Vec::new();
            for q in -10_i64..=10 {
                for r in -10_i64..=10 {
                    if q.abs().max(r.abs()).max((q + r).abs()) > 10 {
                        continue;
                    }
                    let p = WorldHex::new(preferred.column.q + q, preferred.column.r + r);
                    if let Ok(pos) =
                        self.r02_support_column(p, height.map(|h| self.geography.top_level(h)))
                    {
                        if self.clear_support(pos, 16) {
                            surfaces.push(pos);
                        }
                    }
                }
            }
            surfaces.sort();
            surfaces.dedup();
            if !surfaces.contains(&preferred) {
                failures.push(format!(
                    "{id}: preferred encounter support lacks exact body clearance"
                ));
            }
            encounters.push(Encounter {
                id: id.into(),
                preferred,
                surfaces,
                rally_entry: None,
            });
        }
        let route_nodes = self
            .anchors
            .iter()
            .map(|a| Node {
                id: a.id.rsplit('/').next().unwrap_or(&a.id).into(),
                position: a.position,
            })
            .collect();
        let mut routes = Vec::new();
        for r in &self.layered.routes {
            let blocked: Vec<_> = r
                .supports
                .iter()
                .filter(|support| !self.clear_support(**support, 8))
                .collect();
            if let Some(first) = blocked.first() {
                failures.push(format!(
                    "{}: {} blocked supports, first {first:?}; runs={:?}",
                    r.id,
                    blocked.len(),
                    self.column(first.column).0.runs
                ));
            }
            routes.push(Route {
                id: r.id.clone(),
                from: r.from.clone(),
                to: r.to.clone(),
                clearance_levels: 8,
                supports: r.supports.clone(),
                ribbon: r.ribbon.clone(),
            });
        }
        if !failures.is_empty() {
            return Err(ContractError::new("grand.sites", failures.join("\n")));
        }
        let frame = self.geography.frame("fountain")?;
        let center = frame.hex([0., 0.]);
        let mut cells = Vec::new();
        for q in -4_i64..=4 {
            for r in -4_i64..=4 {
                let p = WorldHex::new(center.q + q, center.r + r);
                if let Some(liquid) = self.column(p).1 {
                    if liquid.body_id == "grand/river" {
                        for level in liquid.bottom..liquid.top {
                            cells.push(VoxelPosition { column: p, level });
                        }
                    }
                }
            }
        }
        cells.sort();
        cells.dedup();
        Ok(GrandSites {
            version: 1,
            world_id: self.source.id.clone(),
            manifest_fingerprint,
            encounters,
            route_nodes,
            routes,
            fountains: vec![Fountain {
                id: "garden_fountain".into(),
                cells,
            }],
        })
    }
    pub(super) fn r02_reserved(&self, p: WorldHex, bottom: i32, top: i32) -> bool {
        if self
            .layered
            .reserved
            .get(&p)
            .is_some_and(|spans| spans.iter().any(|s| bottom < s[1] && top > s[0]))
        {
            return true;
        }
        let point = self.geography.model_xz(p);
        ENCOUNTERS.iter().any(|(_, center, height)| {
            if (point[0] - center[0]).hypot(point[1] - center[1]) > 22. {
                return false;
            }
            let floor = height.map_or_else(
                || self.surface(p).level + 1,
                |y| self.geography.top_level(y),
            );
            bottom < floor + 18 && top > floor
        })
    }
    pub(super) fn tree_reserved_interval(&self, p: WorldHex, bottom: i32, top: i32) -> bool {
        self.reserved_interval(p, bottom, top)
            || self.layered.columns.get(&p).is_some_and(|layers| {
                layers.iter().any(|l| {
                    l.layer == SupportLayer::RootTemple && bottom < l.ceiling && top > l.top
                })
            })
    }
}
