//! Regenerate `fixtures/dsm_grid.{json,f32}`: `cargo run -p vexcel --example make_fixture`
//!
//! Synthetic, not Vexcel data (keeps their licence out of git). 64 x 64 cells of 1 m around
//! Pershing Square: flat ground at 90 m, two towers with a 6 m gap between them to the west
//! (where the sun sets), a low roof east of centre to stand on, and a nodata corner.

use std::path::Path;
use vexcel::{DsmGrid, DsmMeta, DsmSource, NODATA};

fn height(col: u32, row: u32) -> f32 {
    match (col, row) {
        (0..=3, 0..=3) => NODATA,                // missing data, like the edge of a collection
        (6..=13, 18..=28) => 150.0,              // north tower, 60 m tall
        (6..=13, 35..=45) => 130.0,              // south tower, 40 m tall (6 m gap at rows 29..=34)
        (36..=45, 26..=37) => 102.0,             // 12 m roof just east of centre, our observer
        _ => 90.0,
    }
}

fn main() -> Result<(), vexcel::Error> {
    let (w, h) = (64u32, 64u32);
    let grid = DsmGrid {
        meta: DsmMeta {
            center_lat: 34.0490,
            center_lon: -118.2510,
            cell_m: 1.0,
            width: w,
            height: h,
            nodata: NODATA,
            source: DsmSource {
                provider: "synthetic".into(),
                collection: None,
                captured_from: None,
                captured_to: None,
                native_res_m: 1.0,
            },
        },
        data: (0..h).flat_map(|r| (0..w).map(move |c| height(c, r))).collect(),
    };
    vexcel::grid::write_files(&grid, Path::new("fixtures"), "dsm_grid")?;
    println!("wrote fixtures/dsm_grid.json + dsm_grid.f32 ({} cells)", grid.data.len());
    Ok(())
}
