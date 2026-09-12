//! Bounded, manifest-bound companion facts for the Forest expedition package.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::Path,
};

use hex_core::{
    arena::{
        ArenaDeploymentRegion, ArenaEncounterSite, ArenaExpeditionRoute, ArenaExpeditionSites,
        ArenaFountainVolume, ArenaVoxelGeometry,
    },
    HexCoord, TilePos,
};
use hex_world_contracts::{VoxelPosition, WorldHex, WorldManifest};
use serde::Deserialize;

const WORLD_ID: &str = "forest-massif-expedition";
const MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SiteFile {
    version: u32,
    world_id: String,
    manifest_fingerprint: u64,
    encounters: Vec<Encounter>,
    route_nodes: Vec<Node>,
    routes: Vec<Route>,
    fountains: Vec<Fountain>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Encounter {
    id: String,
    preferred: VoxelPosition,
    surfaces: Vec<VoxelPosition>,
    rally_entry: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    id: String,
    position: VoxelPosition,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Route {
    id: String,
    from: String,
    to: String,
    clearance_levels: u32,
    supports: Vec<VoxelPosition>,
    ribbon: Vec<VoxelPosition>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fountain {
    id: String,
    cells: Vec<VoxelPosition>,
}

pub(super) fn load(
    directory: &Path,
    manifest: &WorldManifest,
    geometry: ArenaVoxelGeometry,
) -> Result<Option<ArenaExpeditionSites>, String> {
    load_identity(
        directory,
        &manifest.world_id,
        manifest.fingerprint,
        geometry,
    )
}

fn load_identity(
    directory: &Path,
    world_id: &str,
    fingerprint: u64,
    geometry: ArenaVoxelGeometry,
) -> Result<Option<ArenaExpeditionSites>, String> {
    if world_id == "forest-massif-battle" {
        return Ok(None);
    }
    if world_id != WORLD_ID {
        return Err("unsupported Forest world identity".into());
    }
    let path = directory.join("arena-sites.ron");
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("Required expedition companion {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err("expedition companion must be a regular file of at most 16 MiB".into());
    }
    let mut bytes = Vec::new();
    File::open(&path)
        .map_err(|error| error.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("expedition companion exceeds 16 MiB".into());
    }
    decode(&bytes, world_id, fingerprint, geometry).map(Some)
}

fn named<T>(map: &mut BTreeMap<String, T>, id: String, value: T) -> Result<(), String> {
    if id.trim().is_empty() || map.insert(id.clone(), value).is_some() {
        return Err(format!("empty or duplicate expedition identity {id}"));
    }
    Ok(())
}

fn decode(
    bytes: &[u8],
    world_id: &str,
    fingerprint: u64,
    geometry: ArenaVoxelGeometry,
) -> Result<ArenaExpeditionSites, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("expedition companion exceeds 16 MiB".into());
    }
    let file: SiteFile =
        ron::de::from_bytes(bytes).map_err(|error| format!("Expedition companion: {error}"))?;
    if file.version != 1
        || file.world_id != WORLD_ID
        || file.world_id != world_id
        || file.manifest_fingerprint != fingerprint
    {
        return Err("expedition companion version/world/fingerprint mismatch".into());
    }
    let position = |voxel: VoxelPosition| -> Result<TilePos, String> {
        if voxel
            .column
            .checked_distance(WorldHex::new(0, 0))
            .map_err(|error| error.to_string())?
            > u64::from(geometry.radius)
            || voxel.level < geometry.min_level
            || voxel.level > geometry.max_level
        {
            return Err("expedition companion position outside published geometry".into());
        }
        Ok(TilePos::new(
            HexCoord::from_axial(
                i32::try_from(voxel.column.q).map_err(|error| error.to_string())?,
                i32::try_from(voxel.column.r).map_err(|error| error.to_string())?,
            ),
            voxel.level,
        ))
    };
    let positions = |values: Vec<VoxelPosition>| -> Result<BTreeSet<TilePos>, String> {
        let mut result = BTreeSet::new();
        for value in values {
            if !result.insert(position(value)?) {
                return Err("duplicate expedition surface or volume cell".into());
            }
        }
        Ok(result)
    };
    let mut sites = ArenaExpeditionSites::default();
    for entry in file.encounters {
        named(
            &mut sites.encounters,
            entry.id,
            ArenaEncounterSite {
                deployment: ArenaDeploymentRegion {
                    preferred: position(entry.preferred)?,
                    surfaces: positions(entry.surfaces)?,
                },
                rally_entry: entry.rally_entry,
            },
        )?;
    }
    for entry in file.route_nodes {
        named(&mut sites.route_nodes, entry.id, position(entry.position)?)?;
    }
    for entry in file.routes {
        named(
            &mut sites.routes,
            entry.id,
            ArenaExpeditionRoute {
                from: entry.from,
                to: entry.to,
                clearance_levels: entry.clearance_levels,
                supports: entry
                    .supports
                    .into_iter()
                    .map(position)
                    .collect::<Result<_, _>>()?,
                ribbon: positions(entry.ribbon)?,
            },
        )?;
    }
    for entry in file.fountains {
        named(
            &mut sites.fountains,
            entry.id,
            ArenaFountainVolume {
                cells: positions(entry.cells)?,
            },
        )?;
    }
    Ok(sites)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        r#"(version:1,world_id:"forest-massif-expedition",manifest_fingerprint:42,
encounters:[(id:"camp",preferred:(column:(q:0,r:0),level:8),surfaces:[(column:(q:0,r:0),level:8)],rally_entry:Some("a"))],
route_nodes:[(id:"a",position:(column:(q:0,r:0),level:8)),(id:"b",position:(column:(q:1,r:0),level:8))],
routes:[(id:"trail",from:"a",to:"b",clearance_levels:3,supports:[(column:(q:0,r:0),level:8),(column:(q:1,r:0),level:8)],ribbon:[(column:(q:0,r:0),level:8),(column:(q:1,r:0),level:8)])],
fountains:[(id:"spring",cells:[(column:(q:2,r:0),level:9)])])"#.into()
    }
    fn parse(source: &str) -> Result<ArenaExpeditionSites, String> {
        decode(
            source.as_bytes(),
            WORLD_ID,
            42,
            ArenaVoxelGeometry::default(),
        )
    }

    #[test]
    fn companion_converts_only_geometry_and_preserves_ordered_routes() {
        let sites = parse(&fixture()).expect("strict companion");
        assert_eq!(sites.encounters.len(), 1);
        assert_eq!(sites.fountains.len(), 1);
        let route = sites.routes.get("trail").expect("route");
        assert_eq!(route.clearance_levels, 3);
        assert_eq!(route.supports.first(), sites.route_nodes.get("a"));
        assert_eq!(route.supports.last(), sites.route_nodes.get("b"));
    }
    #[test]
    fn companion_rejects_version_identity_fingerprint_and_gameplay_fields() {
        for source in [
            fixture().replace("version:1", "version:2"),
            fixture().replace(WORLD_ID, "wrong-world"),
            fixture().replace("fingerprint:42", "fingerprint:43"),
            fixture().replace("id:\"camp\"", "id:\"camp\",hp:100"),
        ] {
            assert!(parse(&source).is_err());
        }
    }
    #[test]
    fn companion_rejects_duplicate_names_cells_and_outside_coordinates() {
        for source in [
            fixture().replace("id:\"b\"", "id:\"a\""),
            fixture().replace(
                "surfaces:[(column:(q:0,r:0),level:8)]",
                "surfaces:[(column:(q:0,r:0),level:8),(column:(q:0,r:0),level:8)]",
            ),
            fixture().replace("q:2", "q:999999999999"),
        ] {
            assert!(parse(&source).is_err());
        }
    }
    #[test]
    fn companion_size_limit_precedes_parsing() {
        assert!(decode(
            &vec![b' '; MAX_BYTES as usize + 1],
            WORLD_ID,
            42,
            ArenaVoxelGeometry::default()
        )
        .expect_err("bounded read")
        .contains("16 MiB"));
    }

    #[test]
    fn expedition_requires_companion_while_legacy_keeps_optional_geometry_absent() {
        let missing = std::env::temp_dir()
            .join(format!("hex-sites-not-created-{}", std::process::id()))
            .join("package");
        let geometry = ArenaVoxelGeometry::default();
        assert!(load_identity(&missing, WORLD_ID, 42, geometry)
            .expect_err("new world needs companion")
            .contains("Required expedition companion"));
        assert_eq!(
            load_identity(&missing, "forest-massif-battle", 42, geometry)
                .expect("legacy unaffected"),
            None
        );
        assert!(load_identity(&missing, "unknown-world", 42, geometry).is_err());
    }
}
