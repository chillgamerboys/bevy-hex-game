//! Export the finalized authored instances, never another seeded forest placement.
use super::*;
use crate::v4::northern::forest::{ForestInstance, ForestOverview, ForestShape, forest_rotate};

pub(super) fn compile(objects: &[ObjectInstance]) -> Result<ForestOverview, ContractError> {
    let mut shapes: BTreeMap<String, ForestShape> = BTreeMap::new();
    let mut pending = Vec::new();
    let mut trees: Vec<_> = objects
        .iter()
        .filter(|o| o.id.starts_with("grand/tree/"))
        .collect();
    trees.sort_by(|a, b| a.id.cmp(&b.id));
    for object in trees {
        let base_level = object
            .grounding
            .iter()
            .flatten()
            .map(|p| p.level + 1)
            .max()
            .ok_or_else(|| {
                ContractError::new(&object.id, "forest source has no ground contacts")
            })?;
        let snowy = object
            .occupancy
            .iter()
            .flat_map(|c| &c.runs)
            .any(|r| r.material == "snow");
        let id = format!("{}{}", object.asset, if snowy { "/snow" } else { "" });
        let mut matched = None;
        for rotation in 0..6 {
            let (columns, roots) = local_source(object, base_level, rotation)?;
            if shapes
                .get(&id)
                .is_some_and(|shape| shape.columns != columns)
            {
                continue;
            }
            shapes.entry(id.clone()).or_insert(ForestShape {
                id: id.clone(),
                asset: object.asset.clone(),
                columns,
            });
            matched = Some((rotation, roots));
            break;
        }
        let (rotation, roots) = matched.ok_or_else(|| {
            ContractError::new(
                &object.id,
                "forest instances with one asset differ beyond rotation/root extension",
            )
        })?;
        pending.push((
            id,
            ForestInstance {
                id: object.id.clone(),
                shape: 0,
                origin: object.origin,
                rotation,
                base_level,
                roots,
                footprint: object
                    .occupancy
                    .iter()
                    .map(|c| c.position.chunk())
                    .chain(std::iter::once(object.origin.column.chunk()))
                    .collect(),
            },
        ));
    }
    let shapes: Vec<_> = shapes.into_values().collect();
    let mut instances = Vec::new();
    for (id, mut instance) in pending {
        instance.shape = shapes.iter().position(|s| s.id == id).ok_or_else(|| {
            ContractError::new("overview/forest", "canonical forest shape disappeared")
        })?;
        instances.push(instance);
    }
    let forest = ForestOverview {
        version: 1,
        shapes,
        instances,
    };
    forest.validate()?;
    for instance in &forest.instances {
        let source = objects
            .iter()
            .find(|o| o.id == instance.id)
            .ok_or_else(|| ContractError::new(&instance.id, "forest source disappeared"))?;
        let shape = forest
            .shapes
            .get(instance.shape)
            .ok_or_else(|| ContractError::new(&instance.id, "forest shape disappeared"))?;
        if !instance.matches_object(shape, source)? {
            return Err(ContractError::new(
                &instance.id,
                "distant forest does not exactly reconstruct authored source",
            ));
        }
    }
    Ok(forest)
}
fn local_source(
    object: &ObjectInstance,
    base: i32,
    rotation: u8,
) -> Result<(Vec<ColumnData>, Vec<ColumnData>), ContractError> {
    let mut columns = Vec::new();
    let mut roots = Vec::new();
    for column in &object.occupancy {
        let position = forest_rotate(
            WorldHex::new(
                column.position.q - object.origin.column.q,
                column.position.r - object.origin.column.r,
            ),
            (6 - rotation) % 6,
        )?;
        let mut runs = Vec::new();
        let mut extensions = Vec::new();
        for run in &column.runs {
            let bottom = run.bottom - base;
            let top = run.top - base;
            if bottom < 0 {
                if run.material != "timber" {
                    return Err(ContractError::new(
                        &object.id,
                        "foliage below the shared forest base",
                    ));
                }
                extensions.push(VoxelRun {
                    bottom,
                    top: top.min(0),
                    material: run.material.clone(),
                });
            }
            if top > 0 {
                runs.push(VoxelRun {
                    bottom: bottom.max(0),
                    top,
                    material: run.material.clone(),
                });
            }
        }
        if !runs.is_empty() {
            columns.push(ColumnData { position, runs });
        }
        if !extensions.is_empty() {
            roots.push(ColumnData {
                position,
                runs: extensions,
            });
        }
    }
    columns.sort_by_key(|c| c.position);
    roots.sort_by_key(|c| c.position);
    Ok((columns, roots))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v4::northern::forest::{
        MAX_FOREST_INSTANCES, MAX_FOREST_SHAPES, MAX_FOREST_SOURCE_RUNS, MAX_FOREST_VERTICES,
        forest_mesh,
    };
    use std::collections::BTreeSet;
    fn compiler() -> Result<GrandCompiler, Box<dyn std::error::Error>> {
        let source: GrandSpec = ron::from_str(include_str!(
            "../../../../../assets/config/v4/grand-v4/world.ron"
        ))?;
        Ok(GrandCompiler::new(GrandSpec {
            full_dressing: true,
            ..source
        })?)
    }
    #[test]
    fn distant_forest_exactly_reconstructs_every_actual_authored_tree_and_budget() {
        let g = compiler().expect("integrated actual Grand source");
        let forest = g.forest.as_ref().expect("full dressing exports forest");
        assert!(!forest.instances.is_empty());
        assert!(forest.instances.len() <= MAX_FOREST_INSTANCES);
        assert!(forest.shapes.len() <= MAX_FOREST_SHAPES);
        assert!(
            forest
                .shapes
                .iter()
                .flat_map(|s| &s.columns)
                .map(|c| c.runs.len())
                .sum::<usize>()
                <= MAX_FOREST_SOURCE_RUNS
        );
        let meshes: Vec<_> = forest
            .shapes
            .iter()
            .map(|s| forest_mesh(&s.columns, &BTreeSet::new()).expect("bounded template"))
            .collect();
        let mut vertices = 0;
        let mut roots = 0;
        for instance in &forest.instances {
            let source = g
                .objects
                .values()
                .flatten()
                .find(|o| o.id == instance.id)
                .expect("authored tree");
            let shape = forest.shapes.get(instance.shape).expect("shape");
            assert!(
                instance
                    .matches_object(shape, source)
                    .expect("exact transform")
            );
            let source_footprint: BTreeSet<_> = source
                .occupancy
                .iter()
                .map(|c| c.position.chunk())
                .chain(std::iter::once(source.origin.column.chunk()))
                .collect();
            assert_eq!(instance.footprint, source_footprint);
            vertices += meshes
                .get(instance.shape)
                .expect("shared mesh")
                .positions
                .len();
            roots += forest_mesh(&instance.roots, &BTreeSet::new())
                .expect("exact roots")
                .positions
                .len();
        }
        assert!(vertices + roots <= MAX_FOREST_VERTICES);
        let encoded = ron::to_string(forest).expect("companion");
        assert!(encoded.len() <= 2 * 1024 * 1024);
        let decoded: ForestOverview = ron::from_str(&encoded).expect("read companion");
        assert_eq!(&decoded, forest);
        println!(
            "Grand forest: {} instances, {} shapes, {} shared vertices, {} submitted vertices + {} root vertices, {} bytes",
            forest.instances.len(),
            forest.shapes.len(),
            meshes.iter().map(|m| m.positions.len()).sum::<usize>(),
            vertices,
            roots,
            encoded.len()
        );
    }
    #[test]
    fn every_actual_forest_instance_keeps_its_budget_after_trunk_and_crown_cuts() {
        let g = compiler().expect("actual authored Grand");
        let forest = g.forest.as_ref().expect("forest");
        let mut submitted = 0;
        for instance in &forest.instances {
            let shape = forest.shapes.get(instance.shape).expect("shape");
            let columns = instance
                .local_columns(shape)
                .expect("exact grounded source");
            let mut cuts = BTreeSet::new();
            for timber in [true, false] {
                let (column, run) = columns
                    .iter()
                    .flat_map(|c| c.runs.iter().map(move |r| (c, r)))
                    .find(|(_, r)| (r.material == "timber") == timber)
                    .expect("trunk and crown");
                cuts.insert(VoxelPosition {
                    column: column.position,
                    level: run.bottom + (run.top - run.bottom) / 2,
                });
            }
            let mesh =
                forest_mesh(&columns, &cuts).expect("actual edited instance remains bounded");
            submitted += mesh.positions.len();
        }
        assert!(submitted <= MAX_FOREST_VERTICES);
        println!("Grand forest simultaneous trunk/crown cuts: {submitted} submitted vertices");
    }
    #[test]
    fn old_overview_without_forest_remains_readable() {
        let mut source: GrandSpec = ron::from_str(include_str!(
            "../../../../../assets/config/v4/grand-v4/world.ron"
        ))
        .expect("source");
        source.full_dressing = false;
        let overview = GrandCompiler::new(source)
            .expect("plain compiler")
            .overview();
        assert!(overview.forest.is_none());
        let encoded = ron::to_string(&overview).expect("old compatible overview");
        assert!(!encoded.contains("forest:Some("));
        let decoded: NorthernOverview = ron::from_str(&encoded).expect("missing field uses None");
        assert!(decoded.forest.is_none());
        assert_eq!(decoded.world_id, overview.world_id);
    }
    #[test]
    fn distant_forest_rejects_truncated_catalog_invalid_masks_and_overbudget_counts() {
        let g = compiler().expect("actual Grand");
        let forest = g.forest.as_ref().expect("forest");
        let catalog: Vec<_> = g
            .objects
            .values()
            .flatten()
            .filter(|o| o.id.starts_with("grand/tree/"))
            .map(|o| FeatureSummary {
                id: o.id.clone(),
                region_id: o.region_id.clone(),
                kind: "tree".into(),
                anchor: o.origin,
                asset: Some(o.asset.clone()),
            })
            .collect();
        forest.validate_catalog(&catalog).expect("exact catalog");
        assert!(forest.validate_catalog(&[]).is_err());
        let mut bad = forest.clone();
        bad.version = 2;
        assert!(bad.validate().is_err());
        let mut bad = forest.clone();
        bad.instances.first_mut().expect("tree").footprint.clear();
        assert!(bad.validate().is_err());
        let mut bad = forest.clone();
        bad.instances.first_mut().expect("tree").rotation = 6;
        assert!(bad.validate().is_err());
        let mut bad = forest.clone();
        let tree = bad.instances.first().expect("tree").clone();
        bad.instances.resize(MAX_FOREST_INSTANCES + 1, tree);
        assert!(bad.validate().is_err());
    }
}
