//! Gameplay facts are authored exact supporting voxels, including cave floors.
use super::*;
#[derive(Clone, Copy)]
pub(super) struct Pad {
    pub(super) x: f64,
    pub(super) z: f64,
    pub(super) radius: f64,
    pub(super) level: i32,
    pub(super) material: &'static str,
}
pub(super) const PADS: &[Pad] = &[
    Pad {
        x: 275.,
        z: -490.,
        radius: 12.,
        level: 699,
        material: "moss",
    },
    Pad {
        x: -400.,
        z: -565.,
        radius: 12.,
        level: 840,
        material: "stone",
    },
    Pad {
        x: -179.,
        z: -518.,
        radius: 12.,
        level: 800,
        material: "slate",
    },
    Pad {
        x: -1170.,
        z: 455.,
        radius: 12.,
        level: 580,
        material: "basalt",
    },
    Pad {
        x: -1205.,
        z: 430.,
        radius: 13.,
        level: 575,
        material: "basalt",
    },
    Pad {
        x: -60.,
        z: 225.,
        radius: 16.,
        level: 500,
        material: "moss",
    },
];
const ENCOUNTERS: &[(&str, f64, f64, bool)] = &[
    ("grand_goblin_01", -80., 350., false),
    ("grand_goblin_02", -230., 250., false),
    ("grand_goblin_03", 100., 270., false),
    ("grand_goblin_04", -160., 160., false),
    ("grand_goblin_05", 80., 125., false),
    ("grand_goblin_06", 0., 180., false),
    ("grand_shaman_01", -60., 225., false),
    ("grand_dragon_01", -1205., 430., false),
    ("grand_golem_01", -280., -348., true),
    ("grand_wisp_01", -400., -530., true),
    ("grand_worm_01", -315., -510., false),
    ("grand_worm_02", -190., -640., false),
    ("grand_worm_03", -110., -510., false),
    ("grand_shadow_tunnel", -105., -295., true),
];
#[derive(Serialize)]
/// Exact manifest-bound shrine, encounter, and fountain companion.
pub struct GrandSites {
    pub(super) version: u32,
    pub(super) world_id: String,
    pub(super) manifest_fingerprint: u64,
    pub(super) encounters: Vec<Encounter>,
    pub(super) route_nodes: Vec<Node>,
    pub(super) routes: Vec<Route>,
    pub(super) fountains: Vec<Fountain>,
}
#[derive(Serialize)]
pub struct Encounter {
    pub(super) id: String,
    pub(super) preferred: VoxelPosition,
    pub(super) surfaces: Vec<VoxelPosition>,
    pub(super) rally_entry: Option<String>,
}
#[derive(Serialize)]
pub struct Node {
    pub(super) id: String,
    pub(super) position: VoxelPosition,
}
#[derive(Serialize)]
pub struct Route {
    pub(super) id: String,
    pub(super) from: String,
    pub(super) to: String,
    pub(super) clearance_levels: u32,
    pub(super) supports: Vec<VoxelPosition>,
    pub(super) ribbon: Vec<VoxelPosition>,
}
#[derive(Serialize)]
pub struct Fountain {
    pub(super) id: String,
    pub(super) cells: Vec<VoxelPosition>,
}
impl GrandCompiler {
    pub(super) fn support(&self, x: f64, z: f64, inside: bool) -> VoxelPosition {
        let p = nearest_hex(x, z);
        VoxelPosition {
            column: p,
            level: if inside {
                self.cavity(p)
                    .map_or(self.surface(p).level, |(floor, _)| floor)
            } else {
                self.surface(p).level
            },
        }
    }
    #[expect(
        clippy::expect_used,
        reason = "The canonical measured bay must contain a dry starting beach; compilation must fail if authoring breaks that required site."
    )]
    fn beach_spawn(&self) -> VoxelPosition {
        let mut best = None;
        for r in 180..=380 {
            for q in -440..=-220 {
                let p = WorldHex::new(q, r);
                let depth = grid_value(&self.coast, p, 0);
                if !(4..=6).contains(&depth) {
                    continue;
                }
                let [x, z] = world_xz(p);
                let s = self.surface(p);
                if s.water.is_some() || s.level < 400 || s.level > 420 {
                    continue;
                }
                let distance = (x + 300.).powi(2) + (z - 450.).powi(2);
                if best.is_none_or(|(old, _)| distance < old) {
                    best = Some((
                        distance,
                        VoxelPosition {
                            column: p,
                            level: s.level,
                        },
                    ));
                }
            }
        }
        best.expect("authored bay has a dry shallow starting beach")
            .1
    }
    pub(super) fn make_anchors(&self) -> Vec<WorldAnchor> {
        let mut anchors = vec![];
        for (id, x, z, inside, gameplay) in [
            ("party_start", -300., 450., false, true),
            ("shrine_water", 275., -490., false, true),
            ("shrine_air", -400., -565., false, true),
            ("shrine_earth", -179., -518., false, true),
            ("shrine_plant", -60., 125., true, true),
            ("shrine_fire", -1170., 455., false, true),
            ("garden", 275., -490., false, false),
            ("mountain_lake", 330., -470., false, false),
            ("waterfall", 345., -380., false, false),
            ("library_entrance", 305., -348., true, true),
            ("library_hall", -370., -348., true, true),
            ("library_upper", -400., -530., true, true),
            ("shadow_entrance", -105., -151., true, true),
            ("shadow_tunnel", -105., -295., true, true),
            ("shadow_exit", -105., -618., true, true),
            ("crystal_ascent", -201., -524., false, false),
            ("frozen_woods", -215., -604., false, false),
            ("world_tree", -60., 125., false, false),
            ("root_temple_entrance", -60., 187.5, true, false),
            ("root_temple_approach", -60., 225., true, false),
            ("goblin_fort", -60., 225., false, false),
            ("forest", -100., 230., false, false),
            ("valley_lake", 405., -115., false, false),
            ("bay", -301., 465., false, false),
            ("volcano", -1180., 450., false, false),
            ("sailing_start", -301., 520., false, false),
            ("volcano_landing", -1095., 465., false, false),
        ] {
            anchors.push(WorldAnchor {
                id: format!("grand/anchor/{id}"),
                region_id: "grand".into(),
                position: if id == "party_start" {
                    self.beach_spawn()
                } else if matches!(id, "sailing_start" | "volcano_landing") {
                    VoxelPosition {
                        column: nearest_hex(x, z),
                        level: SEA_TOP - 1,
                    }
                } else {
                    self.support(x, z, inside)
                },
                role: if gameplay {
                    AnchorRole::Gameplay
                } else {
                    AnchorRole::Observation
                },
            });
        }
        anchors.sort_by(|a, b| a.id.cmp(&b.id));
        anchors
    }
    /// Manifest-bound sites, independent of which source chunks are currently resident.
    pub fn sites(&self, manifest_fingerprint: u64) -> Result<GrandSites, ContractError> {
        let mut encounters = vec![];
        for &(id, x, z, inside) in ENCOUNTERS {
            let preferred = self.support(x, z, inside);
            let mut surfaces = vec![];
            for q in -10_i64..=10 {
                for r in -10_i64..=10 {
                    if q.abs().max(r.abs()).max((q + r).abs()) > 10 {
                        continue;
                    }
                    let p = WorldHex::new(preferred.column.q + q, preferred.column.r + r);
                    let [xx, zz] = world_xz(p);
                    if inside && self.cavity(p).is_none() {
                        continue;
                    }
                    let pos = self.support(xx, zz, inside);
                    if self.clear_support(pos, 16) {
                        surfaces.push(pos);
                    }
                }
            }
            surfaces.sort();
            surfaces.dedup();
            if surfaces.is_empty() || !surfaces.contains(&preferred) {
                return Err(ContractError::new(
                    id,
                    "encounter lacks exact supported headroom",
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
            .filter(|a| a.role == AnchorRole::Gameplay)
            .map(|a| Node {
                id: a
                    .id
                    .rsplit_once('/')
                    .map_or(a.id.as_str(), |(_, name)| name)
                    .into(),
                position: a.position,
            })
            .collect();
        let center = nearest_hex(276., -474.);
        let mut cells = vec![];
        for q in -3_i64..=3 {
            for r in -3_i64..=3 {
                if q.abs().max(r.abs()).max((q + r).abs()) > 3 {
                    continue;
                }
                let p = WorldHex::new(center.q + q, center.r + r);
                let (_, liquid) = self.column(p);
                if let Some(liquid) = liquid {
                    for level in liquid.bottom..liquid.top {
                        cells.push(VoxelPosition { column: p, level });
                    }
                }
            }
        }
        cells.sort();
        let sites = GrandSites {
            version: 1,
            world_id: self.source.id.clone(),
            manifest_fingerprint,
            encounters,
            route_nodes,
            routes: vec![],
            fountains: vec![Fountain {
                id: "garden_fountain".into(),
                cells,
            }],
        };
        for node in &sites.route_nodes {
            if !self.clear_support(node.position, 8) {
                return Err(ContractError::new(
                    &node.id,
                    "gameplay anchor lacks support or headroom",
                ));
            }
        }
        Ok(sites)
    }
    pub(super) fn clear_support(&self, p: VoxelPosition, clearance: i32) -> bool {
        let (c, _) = self.column(p.column);
        let solid = |level| {
            c.runs
                .iter()
                .any(|r| r.bottom <= level && r.top > level && r.material != "water")
        };
        solid(p.level)
            && (p.level + 1..=p.level + clearance).all(|l| !solid(l))
            && self
                .influences
                .get(&p.column.chunk())
                .is_none_or(|objects| {
                    objects.iter().all(|o| {
                        o.occupancy
                            .iter()
                            .filter(|c| c.position == p.column)
                            .all(|c| {
                                c.runs
                                    .iter()
                                    .all(|r| r.top <= p.level + 1 || r.bottom > p.level + clearance)
                            })
                    })
                })
    }
}

/// Exact deployment disks reserve canopy and trunk clearance before any tree is placed.
pub(super) fn reserved_encounter(x: f64, z: f64) -> bool {
    ENCOUNTERS
        .iter()
        .any(|(_, a, b, _)| (x - a).hypot(z - b) < 35.)
}
