//! File IO for `contracts::DsmGrid`: `<dir>/<name>.json` + `<dir>/<name>.f32`.
//! The shape and its byte format live in `crates/contracts` (pure); this is only the disk part.

use std::path::Path;

use contracts::DsmGrid;

use crate::Error;

/// Write `<dir>/<name>.json` and `<dir>/<name>.f32`.
pub fn write_files(grid: &DsmGrid, dir: &Path, name: &str) -> Result<(), Error> {
    std::fs::create_dir_all(dir)?;
    let (json, bin) = grid.to_file_bytes(name);
    std::fs::write(dir.join(format!("{name}.json")), json)?;
    std::fs::write(dir.join(format!("{name}.f32")), bin)?;
    Ok(())
}

/// Read a grid (e.g. a fixture) from its JSON path; the f32 file is found next to it.
pub fn read_files(json_path: &Path) -> Result<DsmGrid, Error> {
    let file = DsmGrid::parse_meta(&std::fs::read_to_string(json_path)?)?;
    let bin = std::fs::read(json_path.parent().unwrap_or(Path::new(".")).join(&file.data_file))?;
    Ok(DsmGrid::from_parts(file.meta, &bin)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{DsmMeta, DsmSource, NODATA};

    #[test]
    fn files_round_trip() {
        let g = DsmGrid {
            meta: DsmMeta {
                center_lat: 34.049,
                center_lon: -118.251,
                cell_m: 1.0,
                width: 2,
                height: 1,
                nodata: NODATA,
                source: DsmSource {
                    provider: "synthetic".into(),
                    collection: None,
                    captured_from: None,
                    captured_to: None,
                    native_res_m: 1.0,
                },
            },
            data: vec![1.5, NODATA],
        };
        let dir = std::env::temp_dir().join(format!("vexcel_grid_test_{}", std::process::id()));
        write_files(&g, &dir, "g").unwrap();
        let back = read_files(&dir.join("g.json")).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(back, g);
    }

    #[test]
    fn reads_the_committed_fixture() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dsm_grid.json");
        assert_eq!(read_files(&p).unwrap().data.len(), 64 * 64);
    }
}
