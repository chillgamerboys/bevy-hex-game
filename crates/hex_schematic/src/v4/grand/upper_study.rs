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
fn mountain_envelope_accepts_foothill_buttresses_but_rejects_unbounded_shapes() {
    use super::geography::MountainEnvelope;
    let canonical: GrandGeographyDocument = serde_json::from_slice(include_bytes!(
        "../../../../../assets/config/v4/grand-v4/geography-r02.json"
    ))
    .expect("canonical geography");
    assert!(super::GrandGeography::new(canonical.clone()).is_ok());
    let invalid: [fn(&mut MountainEnvelope); 7] = [
        |p| p.bodies.resize(13, p.bodies.first().expect("body").clone()),
        |p| {
            p.bodies
                .first_mut()
                .expect("body")
                .spine
                .first_mut()
                .expect("node")[3] = 0.
        },
        |p| {
            p.bodies
                .first_mut()
                .expect("body")
                .spine
                .first_mut()
                .expect("node")[3] = 701.
        },
        |p| {
            p.bodies
                .first_mut()
                .expect("body")
                .spine
                .first_mut()
                .expect("node")[2] = -1.
        },
        |p| {
            p.bodies
                .first_mut()
                .expect("body")
                .spine
                .first_mut()
                .expect("node")[0] = f64::NAN
        },
        |p| p.bodies.first_mut().expect("body").crest_rounding_radius = f64::NAN,
        |p| {
            let body = p.bodies.first_mut().expect("body");
            body.spine = vec![*body.spine.first().expect("node"); 2];
        },
    ];
    for mutate in invalid {
        let mut document = canonical.clone();
        mutate(document.mountain_envelope.as_mut().expect("profile"));
        assert!(super::GrandGeography::new(document).is_err());
    }
}

#[test]
#[ignore = "explicit full-mainland exact source study; no traversal or pixel acceptance"]
fn export_upper_mountain_source_study() -> Result<(), Box<dyn Error>> {
    export_source_study("upper_bodies")
}

#[test]
#[ignore = "explicit final-column bank comparison; no controller acceptance"]
fn export_bank_source_study() -> Result<(), Box<dyn Error>> {
    export_source_study("banks")
}

#[test]
#[ignore = "explicit connected-envelope source study, not game acceptance"]
fn export_envelope_source_study() -> Result<(), Box<dyn Error>> {
    export_source_study("envelope")
}

#[test]
#[ignore = "explicit western composition against a fixed source; no game acceptance"]
fn export_western_envelope_source_study() -> Result<(), Box<dyn Error>> {
    export_source_study("western")
}

#[test]
#[ignore = "read exact source around preselected bank ownership boundaries"]
fn sample_bank_ownership_boundaries() -> Result<(), Box<dyn Error>> {
    let input = std::env::var("HEX_GRAND_BANK_SEAMS")?;
    let output = std::env::var("HEX_GRAND_BANK_SEAMS_OUT")?;
    if Path::new(&output).exists() {
        return Err("preserve prior boundary samples".into());
    }
    #[derive(serde::Deserialize)]
    struct Samples {
        points: Vec<[f64; 2]>,
    }
    let points: Samples = serde_json::from_slice(&fs::read(input)?)?;
    if points.points.len() > 10000 {
        return Err("bounded bank sample budget exceeded".into());
    }
    let g = super::tests::compiler(false);
    let d = g
        .geography
        .document
        .as_ref()
        .ok_or("canonical geography missing")?;
    let mut before = d.clone();
    before
        .ordinary_channel_banks
        .as_mut()
        .ok_or("bank profile missing")?
        .dry_reach_support = None;
    let mut rows = Vec::new();
    for point in points.points {
        let p = g.geography.world_hex(point);
        let coast = f64::from(grid_value(&g.coast, p, 0)) * 1.5;
        rows.push(json!({"point":point,"hex":[p.q,p.r],"coast_distance":coast,
            "baseline":oracle::mainland(&before,point,coast),"candidate":oracle::mainland(d,point,coast)}));
    }
    write_json(
        Path::new(&output),
        &json!({"scope":"Exact continuous Rust surface on both sides of selected boundaries; source hex coast-depth retained, not a controller verdict.","rows":rows}),
    )
}

fn export_source_study(mode: &str) -> Result<(), Box<dyn Error>> {
    let output = std::env::var("HEX_GRAND_UPPER_STUDY_OUT")?;
    let output = Path::new(&output);
    fs::create_dir_all(output)?;
    if output.join("columns.csv").exists() {
        return Err("refusing to overwrite prior study columns".into());
    }
    let source_bytes =
        include_bytes!("../../../../../assets/config/v4/grand-v4/geography-r02.json");
    let candidate: GrandGeographyDocument = serde_json::from_slice(source_bytes)?;
    let mut baseline = candidate.clone();
    let definition = match mode {
        "upper_bodies" => {
            baseline.upper_mountain_bodies = None;
            "candidate document with only upper_mountain_bodies removed"
        }
        "banks" => {
            baseline
                .ordinary_channel_banks
                .as_mut()
                .ok_or("bank profile missing")?
                .dry_reach_support = None;
            "current accepted terrain without the new continuous dry reach support"
        }
        "envelope" => {
            baseline.mountain_envelope = None;
            "same candidate document with the shared mountain envelope removed; all other authoring controls retained"
        }
        "western" => {
            if std::env::var("HEX_GRAND_STUDY_BASELINE").is_err() {
                return Err("western composition requires an explicit fixed baseline".into());
            }
            "explicit fixed geography source"
        }
        _ => return Err("unknown source study mode".into()),
    };
    // Later composition studies compare against an explicit accepted source,
    // rather than silently treating removal of an older profile as baseline.
    let definition = if let Ok(path) = std::env::var("HEX_GRAND_STUDY_BASELINE") {
        baseline = serde_json::from_slice(&fs::read(&path)?)?;
        format!("explicit fixed geography source: {path}")
    } else {
        definition.to_owned()
    };
    let candidate_bytes = source_bytes.to_vec();
    let bodies: Vec<(&str, &[[f64; 4]])> = if matches!(mode, "envelope" | "western") {
        candidate
            .mountain_envelope
            .as_ref()
            .expect("envelope")
            .bodies
            .iter()
            .map(|body| (body.name.as_str(), body.spine.as_slice()))
            .collect()
    } else {
        candidate
            .upper_mountain_bodies
            .as_ref()
            .expect("study profile")
            .bodies
            .iter()
            .map(|body| (body.name.as_str(), body.spine.as_slice()))
            .collect()
    };
    let baseline_bytes = serde_json::to_vec(&baseline)?;
    let mut source: GrandSpec = ron::from_str(include_str!(
        "../../../../../assets/config/v4/grand-v4/world.ron"
    ))?;
    source.full_dressing = false;
    fs::write(output.join("candidate-geography.json"), &candidate_bytes)?;
    fs::write(output.join("baseline-geography.json"), &baseline_bytes)?;
    eprintln!("UPPER_STUDY constructing baseline");
    let before = GrandCompiler::with_geography(source.clone(), baseline.clone(), &baseline_bytes)?;
    eprintln!("UPPER_STUDY constructing candidate");
    let after = GrandCompiler::with_geography(source, candidate.clone(), &candidate_bytes)?;
    if before.source.mainland_rows != after.source.mainland_rows {
        return Err("upper mountain study changed the mainland footprint".into());
    }
    if before.coast != after.coast {
        return Err("upper mountain study changed the coast/depth field".into());
    }
    let mut columns = BufWriter::new(File::create(output.join("columns.csv"))?);
    writeln!(
        columns,
        "q,r,model_east,model_north,baseline_surface,candidate_surface,baseline_top,candidate_top,baseline_water_top,candidate_water_top"
    )?;
    let mut count = 0_usize;
    let mut changed_top = 0_usize;
    let mut changed_water = 0_usize;
    let mut changed_wet_beds = 0_usize;
    let mut changed_near_channel = 0_usize;
    for &(r, start, end) in &before.source.mainland_rows {
        for q in start..=end {
            let p = WorldHex::new(q, r);
            let [east, north] = before.geography.model_xz(p);
            let coast = f64::from(grid_value(&before.coast, p, 0)) * 1.5;
            let old_surface = oracle::mainland(&baseline, [east, north], coast)
                * baseline.transform.vertical_scale;
            let new_surface = oracle::mainland(&candidate, [east, north], coast)
                * candidate.transform.vertical_scale;
            let (old_column, old_water) = before.column(p);
            let (new_column, new_water) = after.column(p);
            let old_top = solid_top(&old_column);
            let new_top = solid_top(&new_column);
            let old_water = old_water.map_or(-1, |water| water.top);
            let new_water = new_water.map_or(-1, |water| water.top);
            changed_top += usize::from(old_top != new_top);
            changed_water += usize::from(old_water != new_water);
            changed_wet_beds += usize::from(old_top != new_top && old_water > old_top);
            let near_channel = [&candidate.falls, &candidate.river]
                .into_iter()
                .any(|channel| {
                    super::geography::route_distance([east, north], &channel.points).0
                        <= channel.width * 0.5 + 20.
                });
            changed_near_channel += usize::from(old_top != new_top && near_channel);
            count += 1;
            writeln!(
                columns,
                "{q},{r},{east:.9},{north:.9},{old_surface:.9},{new_surface:.9},{old_top},{new_top},{old_water},{new_water}"
            )?;
        }
    }
    columns.flush()?;
    eprintln!(
        "UPPER_STUDY exported {count} final columns, changed tops {changed_top}, water {changed_water}"
    );
    let mut sections = Vec::new();
    let section_paths = if mode == "banks" {
        [
            ("lower-falls-ownership-switch", [430., 280.], [550., 280.]),
            ("upper-pool-support-end", [470., 350.], [570., 350.]),
            ("same-swim-exit-shoulder", [405., 300.], [465., 300.]),
            ("ordinary-reach-through-plunge", [470., 265.], [515., 430.]),
            ("lower-lake-shore", [195., 30.], [355., 145.]),
            ("river-support", [220., -60.], [400., -60.]),
        ]
    } else if mode == "western" {
        [
            ("western-foot-to-summit", [-470., 150.], [-470., 620.]),
            ("lower-buttresses-crossing", [-760., 390.], [-220., 390.]),
            ("upper-buttresses-crossing", [-720., 460.], [-240., 460.]),
            ("rear-ridge-saddle", [-650., 600.], [0., 600.]),
            ("summit-to-crystal", [-600., 550.], [0., 550.]),
            ("frozen-connector", [-100., 550.], [-100., 950.]),
        ]
    } else {
        [
            ("northwest-shoulder", [100., 330.], [200., 990.]),
            ("northern-body", [350., 400.], [350., 1100.]),
            ("eastern-body", [820., 610.], [330., 610.]),
            ("outlet-body", [565., 250.], [565., 800.]),
            ("upper-rock-west-east", [-600., 700.], [800., 700.]),
            ("frozen-crossing", [-10., 520.], [-10., 950.]),
        ]
    };
    for (id, from, to) in section_paths {
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
                oracle::mainland(&baseline, [east, north], coast)
                    * baseline.transform.vertical_scale,
                oracle::mainland(&candidate, [east, north], coast)
                    * candidate.transform.vertical_scale,
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
            old_row.push(
                oracle::mainland(&baseline, point, coast) * baseline.transform.vertical_scale,
            );
            new_row.push(
                oracle::mainland(&candidate, point, coast) * candidate.transform.vertical_scale,
            );
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
            "vertical_scale":1.,"authoring_vertical_scale":candidate.transform.vertical_scale,
            "upper_lake_height":candidate.upper_lake.level * candidate.transform.vertical_scale,
            "baseline":before_grid,"candidate":after_grid,"water":water_grid,
            "scope":"Physical sea-relative Rust source heights sampled on nearest production hex at six-model-unit spacing. No decoration, game renderer or body proof."
        }),
    )?;
    let mut crest_samples = Vec::new();
    for (name, spine) in bodies {
        for node in spine {
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
                    let old = oracle::mainland(&baseline, point, coast)
                        * baseline.transform.vertical_scale;
                    let new = oracle::mainland(&candidate, point, coast)
                        * candidate.transform.vertical_scale;
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
            crest_samples.push(json!({"body":name,"authored_node":node,"search_half_width_model":20,
                "baseline_max":old_max,"baseline_at":old_at,"candidate_max":new_max,"candidate_at":new_at}));
        }
    }
    write_json(
        &output.join("source-receipt.json"),
        &json!({
            "status":"SOURCE_STUDY_COMPLETE_NOT_GAME_ACCEPTANCE",
            "mode":mode,"baseline_definition":definition,
            "height_units":"physical world units above sea, without display exaggeration",
            "baseline_authoring_vertical_scale":baseline.transform.vertical_scale,
            "candidate_authoring_vertical_scale":candidate.transform.vertical_scale,
            "baseline_source_fingerprint":before.source_fingerprint,"candidate_source_fingerprint":after.source_fingerprint,
            "mainland_columns":count,"changed_top_columns":changed_top,"changed_water_columns":changed_water,
            "changed_wet_beds":changed_wet_beds,"changed_original_near_channel_tops":changed_near_channel,
            "coast_field_identical":true,"crest_samples":crest_samples,
            "scope":"All mainland final columns include layer carving but omit objects. Gradients/components describe exterior tops, not stacked connectivity or ordinary controller acceptance."
        }),
    )?;
    Ok(())
}
