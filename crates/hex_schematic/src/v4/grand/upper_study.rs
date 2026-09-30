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
        "western" | "common_coast" => {
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
    let bodies: Vec<(&str, &[[f64; 4]])> =
        if matches!(mode, "envelope" | "western" | "common_coast") {
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
    if mode != "common_coast" && before.source.mainland_rows != after.source.mainland_rows {
        return Err("upper mountain study changed the mainland footprint".into());
    }
    if mode != "common_coast" && before.coast != after.coast {
        return Err("upper mountain study changed the coast/depth field".into());
    }
    write_json(
        &output.join("mainland-membership.json"),
        &json!({"baseline":before.source.mainland_rows,"candidate":after.source.mainland_rows,
            "scope":"Exact connected mainland row spans from the normally admitted compilers"}),
    )?;
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
    let sample_columns: std::collections::BTreeSet<_> = before
        .source
        .mainland_rows
        .iter()
        .chain(if mode == "common_coast" {
            after.source.mainland_rows.as_slice()
        } else {
            &[]
        })
        .flat_map(|&(r, start, end)| (start..=end).map(move |q| WorldHex::new(q, r)))
        .collect();
    for p in sample_columns {
        let (q, r) = (p.q, p.r);

        let [east, north] = before.geography.model_xz(p);
        let coast = f64::from(grid_value(&before.coast, p, 0)) * 1.5;
        let old_surface =
            oracle::mainland(&baseline, [east, north], coast) * baseline.transform.vertical_scale;
        let new_surface =
            oracle::mainland(&candidate, [east, north], coast) * candidate.transform.vertical_scale;
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
    columns.flush()?;
    eprintln!(
        "UPPER_STUDY exported {count} final columns, changed tops {changed_top}, water {changed_water}"
    );
    if mode == "common_coast" {
        export_common_coast_marine(output, &before, &after)?;
    }
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
    } else if matches!(mode, "western" | "common_coast") {
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
    let (origin, cols, rows) = if mode == "common_coast" {
        ([-1200., -900.], 400, 400)
    } else {
        ([-820., -100.], 280, 205)
    };
    for row in 0_i32..=rows {
        let mut old_row = Vec::new();
        let mut new_row = Vec::new();
        let mut water_row = Vec::new();
        for col in 0_i32..=cols {
            let point = [
                origin[0] + f64::from(col) * 6.,
                origin[1] + f64::from(row) * 6.,
            ];
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
            "origin":origin,"spacing":6.,"horizontal_scale":candidate.transform.horizontal_scale,
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
            "mainland_columns":after.source.mainland_rows.iter().map(|(_,a,b)|b-a+1).sum::<i64>(),
            "baseline_mainland_columns":before.source.mainland_rows.iter().map(|(_,a,b)|b-a+1).sum::<i64>(),
            "sampled_union_columns":count,"changed_top_columns":changed_top,"changed_water_columns":changed_water,
            "changed_wet_beds":changed_wet_beds,"changed_original_near_channel_tops":changed_near_channel,
            "coast_field_identical":before.coast == after.coast,"crest_samples":crest_samples,
            "scope":"All mainland final columns include layer carving but omit objects. Gradients/components describe exterior tops, not stacked connectivity or ordinary controller acceptance."
        }),
    )?;
    Ok(())
}

#[test]
#[ignore = "explicit common-field coast study; rejected admission still preserves field images"]
fn export_common_coast_source_study() -> Result<(), Box<dyn Error>> {
    let output = std::env::var("HEX_GRAND_UPPER_STUDY_OUT")?;
    let output = Path::new(&output);
    fs::create_dir_all(output)?;
    if output.join("source-receipt.json").exists() {
        return Err("preserve prior common-field study evidence".into());
    }
    let baseline_path = std::env::var("HEX_GRAND_STUDY_BASELINE")?;
    let baseline_bytes = fs::read(&baseline_path)?;
    let candidate_bytes =
        include_bytes!("../../../../../assets/config/v4/grand-v4/geography-r02.json");
    let baseline: GrandGeographyDocument = serde_json::from_slice(&baseline_bytes)?;
    let candidate: GrandGeographyDocument = serde_json::from_slice(candidate_bytes)?;
    let mut source: GrandSpec = ron::from_str(include_str!(
        "../../../../../assets/config/v4/grand-v4/world.ron"
    ))?;
    source.full_dressing = false;
    let before = GrandCompiler::with_geography(source.clone(), baseline.clone(), &baseline_bytes)?;
    // Admission is never bypassed to get a passing final-column study. A failed
    // first composition still yields raw field pictures and its exact error.
    let error = match GrandCompiler::with_geography(source, candidate.clone(), candidate_bytes) {
        Ok(after) => {
            write_json(
                &output.join("coast-admission.json"),
                &json!({
                    "status":"ADMITTED_FOR_SOURCE_STUDY_NOT_GAME_ACCEPTANCE", "baseline_columns":before.source.mainland_rows.iter().map(|(_,a,b)|b-a+1).sum::<i64>(),
                    "candidate_columns":after.source.mainland_rows.iter().map(|(_,a,b)|b-a+1).sum::<i64>(),
                    "fixed_area_envelope":[6.86,7.14],"policy":"2% approved before first measured result; exact Crystal area unchanged"
                }),
            )?;
            drop(after);
            drop(before);
            return export_source_study("common_coast");
        }
        Err(error) => error,
    };
    let geography = super::GrandGeography::new(candidate.clone())?;
    let mut old_grid = Vec::new();
    let mut new_grid = Vec::new();
    let mut water_grid = Vec::new();
    for row in 0..=400 {
        let mut old = Vec::new();
        let mut new = Vec::new();
        let mut water = Vec::new();
        for col in 0..=400 {
            let p = geography.world_hex([-1200. + col as f64 * 6., -900. + row as f64 * 6.]);
            let point = geography.model_xz(p);
            let coast = f64::from(grid_value(&before.coast, p, 0)) * 1.5;
            let height = oracle::coast_reference(&candidate, point);
            old.push(oracle::mainland(&baseline, point, coast) * baseline.transform.vertical_scale);
            new.push(height * candidate.transform.vertical_scale);
            water.push(
                oracle::water(&candidate, point, height)
                    .map(|h| h * candidate.transform.vertical_scale)
                    .or_else(|| (height < 0.).then_some(0.)),
            );
        }
        old_grid.push(old);
        new_grid.push(new);
        water_grid.push(water);
    }
    fs::write(output.join("candidate-geography.json"), candidate_bytes)?;
    fs::write(output.join("baseline-geography.json"), baseline_bytes)?;
    write_json(
        &output.join("grid.json"),
        &json!({"origin":[-1200.,-900.],"spacing":6.,"horizontal_scale":candidate.transform.horizontal_scale,"vertical_scale":1.,"authoring_vertical_scale":candidate.transform.vertical_scale,"upper_lake_height":candidate.upper_lake.level*candidate.transform.vertical_scale,"baseline":old_grid,"candidate":new_grid,"water":water_grid,"scope":"Raw exact Rust terrain field only. Candidate failed compiler admission; no final layered columns or body/route acceptance."}),
    )?;
    write_json(
        &output.join("coast-admission.json"),
        &json!({"status":"REJECTED_BEFORE_FINAL_COLUMN_STUDY","error":error.to_string(),"fixed_area_envelope":[6.86,7.14]}),
    )?;
    write_json(
        &output.join("source-receipt.json"),
        &json!({"status":"FIELD_ONLY_CANDIDATE_REJECTED","baseline_definition":baseline_path,"candidate_admission_error":error.to_string(),"crest_samples":[],"scope":"Raw field pictures preserved despite failed admission. Final-column graph, caves, routes and game package remain blocked."}),
    )?;
    Err(error.into())
}

fn export_common_coast_marine(
    output: &Path,
    before: &GrandCompiler,
    after: &GrandCompiler,
) -> Result<(), Box<dyn Error>> {
    let anchor = |g: &GrandCompiler, id: &str| {
        g.anchors
            .iter()
            .find(|a| a.id == format!("grand/anchor/{id}"))
            .map(|a| a.position)
            .ok_or_else(|| format!("missing marine anchor {id}"))
    };
    let courses = [
        ("western", "sailing_start", "volcano_berth"),
        ("bay_exit", "sailing_start_bay", "sailing_bay_offshore"),
        ("bay_offshore", "sailing_bay_offshore", "volcano_berth"),
    ];
    let mut course_rows = Vec::new();
    for (id, from, to) in courses {
        let a = anchor(after, from)?;
        let b = anchor(after, to)?;
        let aw = super::world_xz(a.column);
        let bw = super::world_xz(b.column);
        let length = (bw[0] - aw[0]).hypot(bw[1] - aw[1]);
        let steps = length.ceil() as i64;
        let mut columns = std::collections::BTreeSet::new();
        for step in 0..=steps {
            let t = step as f64 / steps.max(1) as f64;
            let p = super::nearest_hex(aw[0] + t * (bw[0] - aw[0]), aw[1] + t * (bw[1] - aw[1]));
            for dq in -5..=5 {
                for dr in -5..=5 {
                    let n = WorldHex::new(p.q + dq, p.r + dr);
                    if super::geography::segment(super::world_xz(n), aw, bw).0 <= 4.5 {
                        columns.insert(n);
                    }
                }
            }
        }
        let mut rows = Vec::new();
        let mut changed = 0;
        let mut dry_before = 0;
        let mut dry_after = 0;
        let mut min_before = f64::INFINITY;
        let mut min_after = f64::INFINITY;
        for p in columns {
            let (old, old_water) = before.column(p);
            let (new, new_water) = after.column(p);
            let old_top = solid_top(&old);
            let new_top = solid_top(&new);
            let old_wet = old_water
                .as_ref()
                .filter(|w| w.body_id == "grand/ocean" && w.top > old_top);
            let new_wet = new_water
                .as_ref()
                .filter(|w| w.body_id == "grand/ocean" && w.top > new_top);
            dry_before += usize::from(old_wet.is_none());
            dry_after += usize::from(new_wet.is_none());
            min_before =
                min_before.min(old_wet.map_or(0., |w| f64::from(w.top - old_top) * LEVEL_HEIGHT));
            min_after =
                min_after.min(new_wet.map_or(0., |w| f64::from(w.top - new_top) * LEVEL_HEIGHT));
            let old_water = old_water.map(|w| w.top);
            let new_water = new_water.map(|w| w.top);
            changed += usize::from(old_top != new_top || old_water != new_water);
            rows.push((p.q, p.r, old_top, new_top, old_water, new_water));
        }
        course_rows.push(json!({"id":id,"from":from,"to":to,"length_physical":length,
            "anchors_unchanged":anchor(before,from)?==a && anchor(before,to)?==b,
            "corridor_radius_physical":4.5,"columns":rows.len(),"changed_ground_or_water":changed,
            "dry_before":dry_before,"dry_after":dry_after,"minimum_mean_depth_before":min_before,
            "minimum_mean_depth_after":min_after,"rows":rows}));
    }
    write_json(
        &output.join("marine-approaches.json"),
        &json!({
            "status":"EXACT_PLAIN_COLUMNS_NOT_HULL_OR_CONTROLLER_ACCEPTANCE",
            "row_fields":["q","r","baseline_exclusive_solid_top","candidate_exclusive_solid_top","baseline_water_top","candidate_water_top"],
            "all_authored_anchors_identical":before.anchors == after.anchors,
            "baseline_anchors":before.anchors,"candidate_anchors":after.anchors,
            "courses":course_rows,"scope":"World-owned course segments with a 4.5-unit static corridor; canonical plain column depths. No objects, waves, marine dynamics or arrival acceptance."
        }),
    )?;
    Ok(())
}

#[test]
fn common_coast_policy_preserves_absent_and_legacy_area_contracts() {
    let document: GrandGeographyDocument = serde_json::from_slice(include_bytes!(
        "../../../../../assets/config/v4/grand-v4/geography-r02.json"
    ))
    .expect("canonical document");
    assert!(document.landform_coast);
    let common = super::GrandGeography::new(document.clone()).expect("current schema");
    assert_eq!(super::mainland_area_tolerance(&common, 93326 * 7), 13065);
    let mut old_json = serde_json::to_value(document).expect("document JSON");
    old_json
        .as_object_mut()
        .expect("document object")
        .remove("landform_coast");
    old_json
        .as_object_mut()
        .expect("object")
        .remove("basin_backing");
    let old: GrandGeographyDocument = serde_json::from_value(old_json).expect("older document");
    assert!(!old.landform_coast, "absent field keeps historical sampler");
    let old = super::GrandGeography::new(old).expect("older valid schema");
    assert_eq!(super::mainland_area_tolerance(&old, 93326 * 7), 65);
    assert_eq!(
        super::mainland_area_tolerance(&super::GrandGeography::legacy(), 93326 * 7),
        0
    );
}
