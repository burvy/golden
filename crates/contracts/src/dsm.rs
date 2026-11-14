//! `DsmGrid` (docs/contracts.md#dsmgrid).
//!
//! A north-up grid of heights in ground metres around a centre point. Row 0 is the north edge,
//! column 0 the west edge, data is row-major. To find the cell for a lat/lon:
//!   east_m  = (lon - center_lon) * pi/180 * R * cos(center_lat)
//!   north_m = (lat - center_lat) * pi/180 * R            (R = 6378137)
//!   col = floor(width/2  + east_m  / cell_m)
//!   row = floor(height/2 - north_m / cell_m)
//!
//! On disk a grid is a JSON meta plus a little-endian f32 file. This module only turns grids into
//! those bytes and back; reading and writing the files is up to the caller (e.g. `vexcel::grid`).

use serde::{Deserialize, Serialize};

/// Earth radius (metres) used by the grid formula, same as EPSG:3857.
pub const EARTH_R: f64 = 6_378_137.0;
/// Value of a cell with no data.
pub const NODATA: f32 = -9999.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DsmMeta {
    pub center_lat: f64,
    pub center_lon: f64,
    /// Cell size in ground metres.
    pub cell_m: f64,
    pub width: u32,
    pub height: u32,
    /// Value of cells with no data.
    pub nodata: f32,
    pub source: DsmSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DsmSource {
    /// "vexcel" or "synthetic".
    pub provider: String,
    /// Vexcel collection, e.g. "n34w118-us-ca-losangeles-2026".
    pub collection: Option<String>,
    /// First and last capture date of the imagery, as Vexcel reports them.
    pub captured_from: Option<String>,
    pub captured_to: Option<String>,
    /// Finest source pixel size used, in ground metres, before downsampling.
    pub native_res_m: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DsmGrid {
    pub meta: DsmMeta,
    /// width * height heights in metres, row-major, north to south, west to east.
    pub data: Vec<f32>,
}

/// The on-disk JSON: the meta plus the name of the f32 file next to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DsmFileMeta {
    #[serde(flatten)]
    pub meta: DsmMeta,
    pub data_file: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DsmFileError {
    #[error("bad DsmGrid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("DsmGrid has {got} heights, expected width*height = {want}")]
    WrongSize { got: usize, want: usize },
}

impl DsmGrid {
    /// Fractional (col, row) of a lat/lon, per the contract formula. May be outside the grid.
    pub fn cell_of(&self, lat: f64, lon: f64) -> (f64, f64) {
        let m = &self.meta;
        let east = (lon - m.center_lon).to_radians() * EARTH_R * m.center_lat.to_radians().cos();
        let north = (lat - m.center_lat).to_radians() * EARTH_R;
        (f64::from(m.width) / 2.0 + east / m.cell_m, f64::from(m.height) / 2.0 - north / m.cell_m)
    }

    /// Height at a lat/lon, or None outside the grid or on a nodata cell.
    pub fn height_at(&self, lat: f64, lon: f64) -> Option<f32> {
        let (c, r) = self.cell_of(lat, lon);
        if c < 0.0 || r < 0.0 || c >= f64::from(self.meta.width) || r >= f64::from(self.meta.height) {
            return None;
        }
        let v = self.data[r as usize * self.meta.width as usize + c as usize];
        (v != self.meta.nodata).then_some(v)
    }

    /// The two files' contents: pretty JSON (naming `<name>.f32`) and the f32 bytes.
    pub fn to_file_bytes(&self, name: &str) -> (String, Vec<u8>) {
        let file = DsmFileMeta { meta: self.meta.clone(), data_file: format!("{name}.f32") };
        let json = serde_json::to_string_pretty(&file).expect("DsmMeta always serializes") + "\n";
        (json, f32s_to_le(&self.data))
    }

    /// Parse the JSON meta (to learn `data_file`) before reading the f32 file.
    pub fn parse_meta(json: &str) -> Result<DsmFileMeta, DsmFileError> {
        Ok(serde_json::from_str(json)?)
    }

    /// Build a grid from a parsed meta and the f32 file's bytes, checking the size.
    pub fn from_parts(meta: DsmMeta, data_bytes: &[u8]) -> Result<DsmGrid, DsmFileError> {
        let data = le_to_f32s(data_bytes);
        let want = meta.width as usize * meta.height as usize;
        if data.len() != want {
            return Err(DsmFileError::WrongSize { got: data.len(), want });
        }
        Ok(DsmGrid { meta, data })
    }
}

pub fn f32s_to_le(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn le_to_f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> DsmGrid {
        DsmGrid {
            meta: DsmMeta {
                center_lat: 34.049,
                center_lon: -118.251,
                cell_m: 1.0,
                width: 4,
                height: 2,
                nodata: NODATA,
                source: DsmSource {
                    provider: "synthetic".into(),
                    collection: None,
                    captured_from: None,
                    captured_to: None,
                    native_res_m: 1.0,
                },
            },
            data: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, NODATA, 8.0],
        }
    }

    #[test]
    fn centre_maps_to_middle_of_grid() {
        assert_eq!(tiny().cell_of(34.049, -118.251), (2.0, 1.0));
    }

    #[test]
    fn one_metre_east_is_one_column() {
        let dlon = (1.0 / (EARTH_R * 34.049f64.to_radians().cos())).to_degrees();
        let (c, r) = tiny().cell_of(34.049, -118.251 + dlon);
        assert!((c - 3.0).abs() < 1e-9 && (r - 1.0).abs() < 1e-9);
    }

    #[test]
    fn north_is_row_zero() {
        let g = tiny();
        let dlat = (0.5 / EARTH_R).to_degrees();
        assert_eq!(g.height_at(34.049 + dlat, -118.251), Some(3.0)); // row 0, col 2
        assert_eq!(g.height_at(34.049 - dlat, -118.251), None); // row 1, col 2 is nodata
    }

    #[test]
    fn outside_is_none() {
        assert_eq!(tiny().height_at(35.0, -118.251), None);
    }

    #[test]
    fn file_bytes_round_trip() {
        let g = tiny();
        let (json, bin) = g.to_file_bytes("g");
        let meta = DsmGrid::parse_meta(&json).unwrap();
        assert_eq!(meta.data_file, "g.f32");
        assert_eq!(DsmGrid::from_parts(meta.meta, &bin).unwrap(), g);
    }

    #[test]
    fn wrong_size_is_an_error() {
        let g = tiny();
        assert!(matches!(DsmGrid::from_parts(g.meta, &[0u8; 12]), Err(DsmFileError::WrongSize { got: 3, want: 8 })));
    }

    #[test]
    fn committed_fixture_matches_contract() {
        // tests may read files; the library itself never does
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let meta = DsmGrid::parse_meta(&std::fs::read_to_string(dir.join("dsm_grid.json")).unwrap()).unwrap();
        let g = DsmGrid::from_parts(meta.meta, &std::fs::read(dir.join(&meta.data_file)).unwrap()).unwrap();
        assert_eq!((g.meta.width, g.meta.height, g.meta.cell_m), (64, 64, 1.0));
        assert_eq!(g.height_at(g.meta.center_lat, g.meta.center_lon), Some(90.0));
        assert!(g.data.contains(&NODATA) && g.data.contains(&150.0));
    }
}
