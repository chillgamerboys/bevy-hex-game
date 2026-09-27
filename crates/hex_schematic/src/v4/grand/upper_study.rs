//! Explicit source-study exporter. No renderer, physics or gameplay verdict.
use super::{
    grid_value, oracle, GrandCompiler, GrandGeographyDocument, GrandSpec, LEVEL_HEIGHT, SEA_TOP,
};
use hex_world_contracts::{ColumnData, WorldHex};
use serde_json::json;
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

fn solid_top(column: &ColumnData) -> i32 {
    column
        .runs
        .iter()
        .filter(|run| run.material != "water")
        .map(|run| run.top)
        .max()
        .expect("finite mainland solid support")
}

fn relief(level: i32) -> f64 {
    f64::from(level - SEA_TOP) * LEVEL_HEIGHT
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    let mut file = BufWriter::new(File::create(path)?);
    serde_json::to_writer(&mut file, value)?;
    writeln!(file)?;
    file.flush()?;
    Ok(())
}

#[test]
fn upper_mountain_schema_rejects_unbounded_authoring_and_keeps_old_documents() {
    use super::geography::UpperMountainBodies;
    let canonical: GrandGeographyDocument = serde_json::from_slice(include_bytes!(
        "../../../../../assets/config/v4/grand-v4/geography-r02.json"
    ))
    .expect("canonical geography");
    assert!(super::GrandGeography::new(canonical.clone()).is_ok());
    let invalid: [fn(&mut UpperMountainBodies); 8] = [
        |p| p.height_transition[0] = f64::NAN,
        |p| p.height_transition = [140., 139.],
        |p| p.maximum_uplift = 1000.,
        |p| p.bodies.clear(),
        |p| p.bodies.first_mut().expect("body").spine.truncate(1),
        |p| {
            p.bodies
                .first_mut()
                .expect("body")
                .spine
                .first_mut()
                .expect("node")[3] = 1000.
        },
        |p| {
            p.bodies
                .first_mut()
                .expect("body")
                .spine
                .first_mut()
                .expect("node")[2] = 200.
        },
        |p| {
            p.bodies
                .first_mut()
                .expect("body")
                .spine
                .first_mut()
                .expect("node")[0] = f64::INFINITY
        },
    ];
    for mutate in invalid {
        let mut document = canonical.clone();
        mutate(document.upper_mountain_bodies.as_mut().expect("profile"));
        assert!(super::GrandGeography::new(document).is_err());
    }
    // Existing documents explicitly retain their old sampler when the new
    // bounded profile is absent; this is not a mandatory schema migration.
    let mut serialized = serde_json::to_value(canonical).expect("canonical value");
    serialized
        .as_object_mut()
        .expect("object")
        .remove("upper_mountain_bodies");
    let old: GrandGeographyDocument = serde_json::from_value(serialized).expect("old schema");
    assert!(old.upper_mountain_bodies.is_none());
    assert!(super::GrandGeography::new(old).is_ok());
}

#[test]
#[ignore = "explicit full-mainland exact source study; no traversal or pixel acceptance"]
fn export_upper_mountain_source_study() -> Result<(), Box<dyn Error>> {
    let output = std::env::var("HEX_GRAND_UPPER_STUDY_OUT")?;
    let output = Path::new(&output);
    fs::create_dir_all(output)?;
    if output.join("columns.csv").exists() {
        return Err("refusing to overwrite prior study columns".into());
    }
    let candidate_bytes =
        include_bytes!("../../../../../assets/config/v4/grand-v4/geography-r02.json");
    let candidate: GrandGeographyDocument = serde_json::from_slice(candidate_bytes)?;
    let profile = candidate
        .upper_mountain_bodies
        .as_ref()
        .expect("study profile");
    let mut baseline = candidate.clone();
    baseline.upper_mountain_bodies = None;
    let baseline_bytes = serde_json::to_vec(&baseline)?;
    let mut source: GrandSpec = ron::from_str(include_str!(
        "../../../../../assets/config/v4/grand-v4/world.ron"
    ))?;
    source.full_dressing = false;
    fs::write(output.join("candidate-geography.json"), candidate_bytes)?;
    fs::write(output.join("baseline-geography.json"), &baseline_bytes)?;
    eprintln!("UPPER_STUDY constructing baseline");
    let before = GrandCompiler::with_geography(source.clone(), baseline.clone(), &baseline_bytes)?;
    eprintln!("UPPER_STUDY constructing candidate");
    let after = GrandCompiler::with_geography(source, candidate.clone(), candidate_bytes)?;
    if before.source.mainland_rows != after.source.mainland_rows {
        return Err("upper mountain study changed the mainland footprint".into());
    }
    if before.coast != after.coast {
        return Err("upper mountain study changed the coast/depth field".into());
    }
    let mut columns = BufWriter::new(File::create(output.join("columns.csv"))?);
    writeln!(columns, "q,r,model_east,model_north,baseline_surface,candidate_surface,baseline_top,candidate_top,baseline_water_top,candidate_water_top")?;
    let mut count = 0_usize;
    let mut changed_top = 0_usize;
    let mut changed_water = 0_usize;
    for &(r, start, end) in &before.source.mainland_rows {
        for q in start..=end {
            let p = WorldHex::new(q, r);
            let [east, north] = before.geography.model_xz(p);
            let coast = f64::from(grid_value(&before.coast, p, 0)) * 1.5;
            let old_surface = oracle::mainland(&baseline, [east, north], coast);
            let new_surface = oracle::mainland(&candidate, [east, north], coast);
            let (old_column, old_water) = before.column(p);
            let (new_column, new_water) = after.column(p);
            let old_top = solid_top(&old_column);
            let new_top = solid_top(&new_column);
            let old_water = old_water.map_or(-1, |water| water.top);
            let new_water = new_water.map_or(-1, |water| water.top);
            changed_top += usize::from(old_top != new_top);
            changed_water += usize::from(old_water != new_water);
            count += 1;
            writeln!(columns, "{q},{r},{east:.9},{north:.9},{old_surface:.9},{new_surface:.9},{old_top},{new_top},{old_water},{new_water}")?;
        }
    }
    columns.flush()?;
    eprintln!("UPPER_STUDY exported {count} final columns, changed tops {changed_top}, water {changed_water}");
    let mut sections = Vec::new();
    for (id, from, to) in [
        ("northwest-shoulder", [100., 330.], [200., 990.]),
        ("northern-body", [350., 400.], [350., 1100.]),
        ("eastern-body", [820., 610.], [330., 610.]),
        ("outlet-body", [565., 250.], [565., 800.]),
        ("upper-rock-west-east", [-600., 700.], [800., 700.]),
        ("frozen-crossing", [-10., 520.], [-10., 950.]),
    ] {
        let mut points = Vec::new();
        for step in 0_u16..=250 {
            let t = f64::from(step) / 250.;
            let point = [
                from[0] * (1. - t) + to[0] * t,
                from[1] * (1. - t) + to[1] * t,
            ];
            let p = before.geography.world_hex(point);
            let [east, north] = before.geography.model_xz(p);
            let coast = f64::from(grid_value(&before.coast, p, 0)) * 1.5;
            points.push([
                east,
                north,
                oracle::mainland(&baseline, [east, north], coast),
                oracle::mainland(&candidate, [east, north], coast),
                relief(solid_top(&before.column(p).0)),
                relief(solid_top(&after.column(p).0)),
            ]);
        }
        sections.push(json!({"id":id,"points":points}));
    }
    write_json(
        &output.join("sections.json"),
        &json!({
            "columns":["model_east","model_north","baseline_surface","candidate_surface","baseline_final_top_relief","candidate_final_top_relief"],
            "sections":sections
        }),
    )?;
    let mut before_grid = Vec::new();
    let mut after_grid = Vec::new();
    let mut water_grid = Vec::new();
    for row in 0_i32..=205 {
        let mut old_row = Vec::new();
        let mut new_row = Vec::new();
        let mut water_row = Vec::new();
        for col in 0_i32..=280 {
            let point = [-820. + f64::from(col) * 6., -100. + f64::from(row) * 6.];
            let p = before.geography.world_hex(point);
            let point = before.geography.model_xz(p);
            let coast = f64::from(grid_value(&before.coast, p, 0)) * 1.5;
            old_row.push(oracle::mainland(&baseline, point, coast));
            new_row.push(oracle::mainland(&candidate, point, coast));
            water_row.push(after.surface(p).water.map(relief));
        }
        before_grid.push(old_row);
        after_grid.push(new_row);
        water_grid.push(water_row);
    }
    write_json(
        &output.join("grid.json"),
        &json!({
            "origin":[-820.,-100.],"spacing":6.,"horizontal_scale":candidate.transform.horizontal_scale,
            "vertical_scale":1.,"baseline":before_grid,"candidate":after_grid,"water":water_grid,
            "scope":"Exact Rust source sampled on nearest production hex at six-model-unit spacing. No decoration, game renderer or body proof."
        }),
    )?;
    let mut crest_samples = Vec::new();
    for body in &profile.bodies {
        for node in &body.spine {
            let mut old_max = f64::NEG_INFINITY;
            let mut new_max = f64::NEG_INFINITY;
            let mut old_at = [0.; 2];
            let mut new_at = [0.; 2];
            for east_offset in -20_i32..=20 {
                for north_offset in -20_i32..=20 {
                    let point = [
                        node[0] + f64::from(east_offset),
                        node[1] + f64::from(north_offset),
                    ];
                    let p = before.geography.world_hex(point);
                    let point = before.geography.model_xz(p);
                    let coast = f64::from(grid_value(&before.coast, p, 0)) * 1.5;
                    let old = oracle::mainland(&baseline, point, coast);
                    let new = oracle::mainland(&candidate, point, coast);
                    if old > old_max {
                        old_max = old;
                        old_at = point;
                    }
                    if new > new_max {
                        new_max = new;
                        new_at = point;
                    }
                }
            }
            crest_samples.push(json!({"body":body.name,"authored_node":node,"search_half_width_model":20,
                "baseline_max":old_max,"baseline_at":old_at,"candidate_max":new_max,"candidate_at":new_at}));
        }
    }
    write_json(
        &output.join("source-receipt.json"),
        &json!({
            "status":"SOURCE_STUDY_COMPLETE_NOT_GAME_ACCEPTANCE",
            "baseline_definition":"candidate document with only upper_mountain_bodies removed; all three corrective commits retained",
            "baseline_source_fingerprint":before.source_fingerprint,"candidate_source_fingerprint":after.source_fingerprint,
            "mainland_columns":count,"changed_top_columns":changed_top,"changed_water_columns":changed_water,
            "coast_field_identical":true,"crest_samples":crest_samples,
            "scope":"All mainland final columns include layer carving but omit objects. Gradients/components describe exterior tops, not stacked connectivity or ordinary controller acceptance."
        }),
    )?;
    Ok(())
}
