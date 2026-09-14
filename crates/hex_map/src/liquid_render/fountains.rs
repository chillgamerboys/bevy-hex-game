//! Fountain appearance on the existing water geometry, never an extra surface.

use super::*;

/// Passive, stack-safe water ownership retained only while building render batches.
#[derive(Default)]
pub(crate) struct FountainWater {
    generation: u64,
    names: Vec<String>,
    cells: BTreeMap<TilePos, usize>,
}

impl FountainWater {
    #[cfg(feature = "arena-prototype")]
    pub(crate) fn from_view(generation: u64, view: &hex_core::arena::ArenaTerrainView) -> Self {
        let mut result = Self {
            generation,
            ..default()
        };
        if let Some(sites) = &view.expedition {
            for (index, (name, volume)) in sites.fountains.iter().enumerate() {
                result.names.push(name.clone());
                result
                    .cells
                    .extend(volume.cells.iter().map(|cell| (*cell, index)));
            }
        }
        result
    }

    pub(super) fn group(&self, surface: LiquidSurface) -> Option<usize> {
        if surface.role == FillMaterialRole::Water {
            self.cells.get(&surface.position).copied()
        } else {
            None
        }
    }

    pub(super) fn attach(
        &self,
        commands: &mut Commands,
        entity: Entity,
        group: Option<usize>,
        style: MaterialStyle,
        ordinary: &[MaterialSet],
        charged: Option<&MaterialSet>,
    ) {
        let Some((name, charged)) = group.and_then(|index| self.names.get(index)).zip(charged)
        else {
            return;
        };
        commands.entity(entity).insert(FountainMaterial {
            generation: self.generation,
            name: name.clone(),
            ordinary: material_handle(ordinary, FillMaterialRole::Water, style),
            charged: charged.handle(style),
        });
    }
}

/// Both handles share the exact mesh and liquid animation. All pools reuse the
/// same charged pair; a pool's current state only selects its handle.
#[derive(Component)]
#[cfg_attr(
    not(feature = "arena-prototype"),
    expect(
        dead_code,
        reason = "Only standalone arena rendering consumes fountain bindings."
    )
)]
pub(crate) struct FountainMaterial {
    generation: u64,
    name: String,
    ordinary: Handle<LiquidMaterial>,
    charged: Handle<LiquidMaterial>,
}

#[cfg(feature = "arena-prototype")]
pub(crate) fn sync_fountain_materials(
    state: Res<hex_core::arena::ArenaFountainVisuals>,
    mut pools: Query<(&FountainMaterial, &mut MeshMaterial3d<LiquidMaterial>)>,
) {
    for (pool, mut material) in &mut pools {
        let charged = state.generation == pool.generation && state.charged.contains(&pool.name);
        let desired = if charged {
            &pool.charged
        } else {
            &pool.ordinary
        };
        if material.0 != *desired {
            material.0 = desired.clone();
        }
    }
}

pub(super) fn charged_materials(
    foam: Color,
    phase_seconds: f32,
    water_style: WaterSurfaceStyle,
    materials: &mut Assets<LiquidMaterial>,
) -> MaterialSet {
    // A restrained turquoise treatment of the accepted foam palette. The
    // extension supplies emission; StandardMaterial.emissive is overwritten by
    // the liquid shader. Keep flow, highlights, alpha and depth settings intact.
    let foam_linear = foam.to_linear();
    let color = Color::linear_rgb(
        foam_linear.red * 0.12,
        foam_linear.green * 0.58,
        foam_linear.blue * 0.42,
    );
    let set = MaterialSet::create(
        FillMaterialRole::Water,
        color,
        foam,
        phase_seconds,
        water_style,
        materials,
    );
    for handle in [&set.surface, &set.fall] {
        if let Some(mut material) = materials.get_mut(handle) {
            // This pulse closes at the same 400-second phase wrap as ordinary
            // water and lava. It changes appearance, never fountain eligibility.
            material.extension.params.emission = Vec4::new(0.32, 0.06, 0.05, 0.0);
        }
    }
    set
}

#[cfg(all(test, feature = "arena-prototype"))]
mod tests {
    use bevy::ecs::world::CommandQueue;
    use hex_core::arena::{
        ArenaExpeditionSites, ArenaFountainVisuals, ArenaFountainVolume, ArenaTerrainView,
    };

    use super::*;

    fn sites(generation: u64) -> FountainWater {
        let view = ArenaTerrainView {
            expedition: Some(ArenaExpeditionSites {
                fountains: BTreeMap::from([
                    (
                        "first".to_owned(),
                        ArenaFountainVolume {
                            cells: (2..=4)
                                .map(|level| TilePos::new(HexCoord::ORIGIN, level))
                                .collect(),
                        },
                    ),
                    (
                        "second".to_owned(),
                        ArenaFountainVolume {
                            cells: BTreeSet::from([TilePos::new(HexCoord::from_axial(1, 0), 1)]),
                        },
                    ),
                ]),
                ..default()
            }),
            ..default()
        };
        FountainWater::from_view(generation, &view)
    }

    fn surfaces() -> Vec<LiquidSurface> {
        [(0, 4), (1, 1), (2, 1), (0, -1)]
            .into_iter()
            .map(|(q, level)| LiquidSurface {
                position: TilePos::new(HexCoord::from_axial(q, 0), level),
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Still,
                downstream: None,
            })
            .chain([LiquidSurface {
                position: TilePos::new(HexCoord::from_axial(3, 0), 0),
                role: FillMaterialRole::Lava,
                flow: LiquidFlowState::Still,
                downstream: None,
            }])
            .collect()
    }

    #[test]
    fn fountain_partition_retains_exact_caps_curtains_and_uvs_without_boundary_walls() {
        let surfaces = surfaces();
        let pools = sites(7);
        let neutral = FountainWater::default();
        let mut gathered = Vec::new();
        for (key, batch) in batch_liquid_caps(&surfaces, &pools) {
            for surface in &batch {
                assert_eq!(key.fountain, pools.group(*surface));
            }
            gathered.extend(batch);
        }
        gathered.sort_by_key(|surface| surface.position);
        let mut expected = surfaces.clone();
        expected.sort_by_key(|surface| surface.position);
        assert_eq!(gathered, expected, "each exact surface occurs once");
        let cap_vertices = |facts: &FountainWater| {
            let mut vertices = Vec::new();
            for batch in batch_liquid_caps(&surfaces, facts).values() {
                let mesh = cap_batch_geometry(batch, 0.35).expect("cap geometry");
                for ((position, normal), uv) in
                    mesh.positions.iter().zip(&mesh.normals).zip(&mesh.uvs)
                {
                    let [x, y, z] = *position;
                    let [nx, ny, nz] = *normal;
                    let [u, v] = *uv;
                    vertices.push([x, y, z, nx, ny, nz, u, v].map(f32::to_bits));
                }
            }
            vertices.sort();
            vertices
        };
        assert_eq!(
            cap_vertices(&pools),
            cap_vertices(&neutral),
            "grouping preserves every position, normal, UV and duplicate count"
        );
        let raw_neutral = curtain_strips(&surfaces, &neutral).expect("ordinary sides");
        let raw_pools = curtain_strips(&surfaces, &pools).expect("pool sides");
        let neutral_strips = raw_neutral
            .values()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        let pool_strips = raw_pools
            .values()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            pool_strips, neutral_strips,
            "no added group-boundary curtain"
        );
        assert_eq!(
            pool_strips.len(),
            1,
            "only the real three-level step has a side"
        );
        for (key, strips) in raw_pools {
            assert_eq!(key.fountain, Some(0));
            assert_eq!(
                curtain_geometry(&strips, 0.35).expect("pool geometry"),
                curtain_geometry(&neutral_strips.iter().copied().collect::<Vec<_>>(), 0.35)
                    .expect("neutral geometry")
            );
        }
        assert_eq!(pools.group(*surfaces.last().expect("lava")), None);
        assert_eq!(
            pools.group(*surfaces.get(3).expect("lower water")),
            None,
            "another water surface at the same coordinate is not fountain water"
        );
    }

    #[test]
    fn fountain_material_retains_animation_alpha_and_depth_for_both_surface_styles() {
        let mut materials = Assets::<LiquidMaterial>::default();
        let foam = Color::srgb(0.93, 0.99, 1.0);
        let ordinary = MaterialSet::create(
            FillMaterialRole::Water,
            Color::srgb(0.02, 0.24, 0.76),
            foam,
            18.5,
            WaterSurfaceStyle::Translucent,
            &mut materials,
        );
        let charged = charged_materials(foam, 18.5, WaterSurfaceStyle::Translucent, &mut materials);
        for style in [MaterialStyle::Surface, MaterialStyle::Fall] {
            let before = materials.get(&ordinary.handle(style)).expect("ordinary");
            let after = materials.get(&charged.handle(style)).expect("charged");
            assert_eq!(before.base.alpha_mode, after.base.alpha_mode);
            assert_eq!(
                before.base.base_color.alpha().to_bits(),
                after.base.base_color.alpha().to_bits()
            );
            assert_eq!(
                before.base.depth_bias.to_bits(),
                after.base.depth_bias.to_bits()
            );
            assert_eq!(before.base.cull_mode, after.base.cull_mode);
            assert_eq!(
                before.base.perceptual_roughness.to_bits(),
                after.base.perceptual_roughness.to_bits()
            );
            assert_eq!(
                before.base.reflectance.to_bits(),
                after.base.reflectance.to_bits()
            );
            assert_eq!(
                before.extension.params.flow_phase_scale,
                after.extension.params.flow_phase_scale
            );
            assert_eq!(
                before.extension.params.modulation,
                after.extension.params.modulation
            );
            assert_eq!(
                before.extension.params.foam_color,
                after.extension.params.foam_color
            );
            assert_ne!(before.base.base_color, after.base.base_color);
            assert_eq!(before.extension.params.emission, Vec4::ZERO);
            assert!(after.extension.params.emission.x > 0.0);
            assert!(after.extension.params.emission.x + after.extension.params.emission.y < 0.5);
        }
    }

    fn spawn(app: &mut App, pools: &FountainWater) -> Vec<Entity> {
        let table = super::super::tests::liquid_table();
        let water = table.id("water").expect("water");
        let lava = table.id("lava").expect("lava");
        let mut map = VoxelMap::new();
        for surface in surfaces() {
            map.set(
                surface.position,
                if surface.role == FillMaterialRole::Water {
                    water
                } else {
                    lava
                },
            );
        }
        let mut queue = CommandQueue::default();
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = app
            .world_mut()
            .remove_resource::<Assets<LiquidMaterial>>()
            .expect("materials");
        let entities = spawn_presentations(
            &mut Commands::new(&mut queue, app.world()),
            &mut meshes,
            &mut materials,
            &map,
            &table,
            0.35,
            0.0,
            WaterSurfaceStyle::Translucent,
            None,
            pools,
        )
        .expect("liquids");
        queue.apply(app.world_mut());
        app.insert_resource(materials);
        entities
    }

    #[test]
    fn fountain_live_handles_follow_charge_consumption_stale_generation_and_reset() {
        let mut app = App::new();
        app.init_resource::<ArenaFountainVisuals>()
            .init_resource::<Assets<LiquidMaterial>>()
            .init_resource::<Time>()
            .insert_resource(LiquidVisualTime::frozen_at(21.0).expect("finite phase"))
            .add_systems(
                Update,
                (advance_liquid_visual_time, sync_fountain_materials).chain(),
            );
        let old_entities = spawn(&mut app, &sites(7));
        let assert_charged = |app: &mut App, expected: &BTreeSet<&str>| {
            let mut query = app
                .world_mut()
                .query::<(&FountainMaterial, &MeshMaterial3d<LiquidMaterial>)>();
            let mut found = BTreeSet::new();
            let mut bindings = 0;
            for (pool, material) in query.iter(app.world()) {
                bindings += 1;
                found.insert(pool.name.as_str());
                assert_eq!(
                    material.0,
                    if expected.contains(pool.name.as_str()) {
                        pool.charged.clone()
                    } else {
                        pool.ordinary.clone()
                    }
                );
            }
            assert_eq!(bindings, 3, "two caps and one existing side curtain");
            assert_eq!(found, BTreeSet::from(["first", "second"]));
        };
        app.update();
        assert_charged(&mut app, &BTreeSet::new());
        app.insert_resource(ArenaFountainVisuals {
            generation: 7,
            charged: BTreeSet::from([
                "first".to_owned(),
                "second".to_owned(),
                "unknown".to_owned(),
            ]),
        });
        app.update();
        assert_charged(&mut app, &BTreeSet::from(["first", "second"]));
        app.world_mut()
            .resource_mut::<ArenaFountainVisuals>()
            .charged
            .remove("first");
        app.update();
        assert_charged(&mut app, &BTreeSet::from(["second"]));
        app.world_mut()
            .resource_mut::<ArenaFountainVisuals>()
            .generation = 8;
        app.update();
        assert_charged(&mut app, &BTreeSet::new());
        for entity in old_entities {
            app.world_mut().despawn(entity);
        }
        spawn(&mut app, &sites(8));
        app.world_mut()
            .resource_mut::<ArenaFountainVisuals>()
            .charged
            .insert("first".to_owned());
        app.update();
        assert_charged(&mut app, &BTreeSet::from(["first", "second"]));
        assert_eq!(
            app.world()
                .resource::<LiquidMaterialHandles>()
                .handles
                .len(),
            6,
            "water, lava and one shared charged pair stay phase-registered after reset"
        );
        for handle in &app.world().resource::<LiquidMaterialHandles>().handles {
            let material = app
                .world()
                .resource::<Assets<LiquidMaterial>>()
                .get(handle)
                .expect("registered material");
            assert_eq!(
                material.extension.params.flow_phase_scale.z.to_bits(),
                21.0_f32.to_bits(),
                "inactive and active handles share the ordinary visual phase"
            );
        }
    }
}
