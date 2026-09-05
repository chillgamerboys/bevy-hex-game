//! Generates exact renderer-free structural review artifacts for Grand V3.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use bevy::prelude::World;
use hex_assets::{
    ArtPalette, ObjectBlueprint, ObjectCatalogFile, RuntimeArtCatalog, SubstanceFile,
    SubstanceTable, VoxelStyleCatalog,
};
use hex_map::structural_preview::{
    begin_grand_v3_structural_preview_publication, build_grand_v3_structural_preview,
    write_grand_v3_structural_preview, GRAND_V3_STRUCTURAL_PREVIEW_HERO_SEED,
};
use hex_map::{compile_schematic, export_world_snapshot_v1, MapSettings};

#[derive(Debug)]
struct Arguments {
    seed: u64,
    output: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    let Some(arguments) = arguments()? else {
        return Ok(());
    };
    // Invalidate any earlier successful pack before generation starts. Compilation can fail well
    // before the artifact writer runs, and an old manifest must never make that failure look current.
    begin_grand_v3_structural_preview_publication(&arguments.output)?;
    let template = hex_schematic::grand_v3_reference_template()?;
    let generated = hex_schematic::generate(&template, arguments.seed)?;
    let settings: MapSettings = ron::de::from_str(include_str!(
        "../../../assets/config/worlds/procedural-grand-v3-baseline.ron"
    ))?;
    let palette: ArtPalette = ron::de::from_str(include_str!("../../../assets/art/palette.ron"))?;
    let substances: SubstanceFile =
        ron::de::from_str(include_str!("../../../assets/config/substances.ron"))?;
    let table = SubstanceTable::from_file(&substances, &palette)
        .map_err(|error| io::Error::other(format!("cannot resolve substances: {error}")))?;
    let art_catalog = runtime_art_catalog(&palette)?;

    let compiled = compile_schematic(&generated.plan, &settings, &table, &art_catalog)?;
    let observation_anchors = compiled.observation_anchors.clone();
    let mut world = World::new();
    world.insert_resource(settings);
    world.insert_resource(table);
    world.insert_resource(art_catalog);
    compiled.publish(&mut world);
    let snapshot = export_world_snapshot_v1(&world)?;
    let preview = build_grand_v3_structural_preview(
        &generated.plan,
        &snapshot,
        &observation_anchors,
        arguments.seed,
    )?;
    let outputs = write_grand_v3_structural_preview(&preview, &arguments.output)?;

    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    writeln!(
        stdout,
        "Grand V3 structural preview for seed {} written to {}",
        arguments.seed,
        arguments.output.display()
    )?;
    for output in outputs {
        writeln!(stdout, "  {}", output.display())?;
    }
    Ok(())
}

fn arguments() -> Result<Option<Arguments>, Box<dyn Error>> {
    let mut seed = GRAND_V3_STRUCTURAL_PREVIEW_HERO_SEED;
    let mut output = None;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--seed" => {
                let value = arguments.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--seed requires a value")
                })?;
                seed = value.parse().map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("invalid --seed value {value:?}: {error}"),
                    )
                })?;
            }
            "--output" => {
                output = Some(PathBuf::from(arguments.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--output requires a path")
                })?));
            }
            "--help" | "-h" => {
                print_usage()?;
                return Ok(None);
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument {argument:?}; use --help for usage"),
                )
                .into());
            }
        }
    }
    Ok(Some(Arguments {
        seed,
        output: output.unwrap_or_else(|| {
            PathBuf::from("target/grand-v3-structural-preview").join(seed.to_string())
        }),
    }))
}

fn print_usage() -> io::Result<()> {
    writeln!(
        io::stdout().lock(),
        "Usage: cargo run -p hex_map --example grand_v3_structural_preview -- \
         [--seed U64] [--output DIRECTORY]"
    )
}

fn runtime_art_catalog(palette: &ArtPalette) -> Result<RuntimeArtCatalog, Box<dyn Error>> {
    let styles: VoxelStyleCatalog =
        ron::from_str(include_str!("../../../assets/art/voxel_styles.ron"))?;
    let manifest: ObjectCatalogFile =
        ron::from_str(include_str!("../../../assets/art/object_catalog.ron"))?;
    let object_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/art/objects");
    let mut objects = BTreeMap::new();
    for id in manifest.ids() {
        let path = object_directory.join(format!("{}.ron", id.as_str()));
        let source = fs::read_to_string(&path)?;
        let blueprint: ObjectBlueprint = ron::from_str(&source)?;
        if objects.insert(blueprint.id.clone(), blueprint).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("duplicate object blueprint in {}", path.display()),
            )
            .into());
        }
    }
    RuntimeArtCatalog::from_sources(palette, &styles, &manifest, objects)
        .map_err(|error| io::Error::other(format!("cannot resolve runtime art catalog: {error}")))
        .map_err(Into::into)
}
