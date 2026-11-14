//! Pure: turn 3857 rasters into a ground-metre `DsmGrid`.
//!
//! Each output cell keeps the HIGHEST source pixel whose centre falls in it. For a horizon,
//! a roof edge that rises anywhere in the cell blocks the sun, so the max is the honest choice
//! (a mean would shave building edges down).

use crate::geotiff::Raster;
use crate::grid::NODATA;
use crate::tiles::{merc_to_lonlat, EARTH_R};

/// Where the output grid sits. Same meaning as in `DsmMeta`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridSpec {
    pub center_lat: f64,
    pub center_lon: f64,
    pub cell_m: f64,
    pub width: u32,
    pub height: u32,
}

impl GridSpec {
    /// Square grid covering `radius_m` ground metres around the centre.
    pub fn around(lat: f64, lon: f64, radius_m: f64, cell_m: f64) -> GridSpec {
        let n = (2.0 * radius_m / cell_m).ceil().max(1.0) as u32;
        GridSpec { center_lat: lat, center_lon: lon, cell_m, width: n, height: n }
    }

    pub fn empty(&self) -> Vec<f32> {
        vec![NODATA; self.width as usize * self.height as usize]
    }
}

/// Fold one raster into `out` (len width*height) with a per-cell max.
pub fn accumulate_max(spec: &GridSpec, out: &mut [f32], r: &Raster) {
    let cos_c = spec.center_lat.to_radians().cos();
    let (w, h) = (i64::from(spec.width), i64::from(spec.height));
    let center_x = EARTH_R * spec.center_lon.to_radians();

    // column of every source pixel: longitude is linear in 3857 x, so east_m = dx * cos(center_lat)
    let cols: Vec<i64> = (0..r.width)
        .map(|i| {
            let x = r.west + (f64::from(i) + 0.5) * r.scale_x;
            let east = (x - center_x) * cos_c;
            (w as f64 / 2.0 + east / spec.cell_m).floor() as i64
        })
        .collect();

    for j in 0..r.height {
        // latitude is not linear in 3857 y, so convert each row exactly
        let y = r.north - (f64::from(j) + 0.5) * r.scale_y;
        let (_, lat) = merc_to_lonlat(0.0, y);
        let north = (lat - spec.center_lat).to_radians() * EARTH_R;
        let row = (h as f64 / 2.0 - north / spec.cell_m).floor() as i64;
        if row < 0 || row >= h {
            continue;
        }
        let src_row = &r.data[(j * r.width) as usize..((j + 1) * r.width) as usize];
        let out_row = &mut out[(row * w) as usize..((row + 1) * w) as usize];
        for (&v, &col) in src_row.iter().zip(&cols) {
            if col < 0 || col >= w || !is_valid(v, r.nodata) {
                continue;
            }
            let cell = &mut out_row[col as usize];
            if *cell == NODATA || v > *cell {
                *cell = v;
            }
        }
    }
}

fn is_valid(v: f32, nodata: f32) -> bool {
    v.is_finite() && v != nodata && v > -1000.0
}

/// Fine data wins wherever it exists; coarse fills the rest.
pub fn overlay(fine: &[f32], coarse: &[f32]) -> Vec<f32> {
    fine.iter().zip(coarse).map(|(&f, &c)| if f != NODATA { f } else { c }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geotiff::XmpInfo;
    use crate::tiles::lonlat_to_merc;

    const LAT: f64 = 34.049;
    const LON: f64 = -118.251;

    /// A raster of `n` x `n` pixels of `px_m` ground metres, centred on (LAT, LON).
    fn raster(n: u32, px_m: f64, f: impl Fn(u32, u32) -> f32) -> Raster {
        let scale = px_m / LAT.to_radians().cos();
        let (x, y) = lonlat_to_merc(LON, LAT);
        let half = f64::from(n) * scale / 2.0;
        Raster {
            width: n,
            height: n,
            west: x - half,
            north: y + half,
            scale_x: scale,
            scale_y: scale,
            nodata: -9999.0,
            data: (0..n * n).map(|k| f(k % n, k / n)).collect(),
            info: XmpInfo::default(),
        }
    }

    #[test]
    fn flat_ground_stays_flat() {
        let spec = GridSpec::around(LAT, LON, 5.0, 1.0);
        let mut out = spec.empty();
        accumulate_max(&spec, &mut out, &raster(200, 0.1, |_, _| 90.0));
        assert_eq!(spec.width, 10);
        assert!(out.iter().all(|&v| v == 90.0), "{out:?}");
    }

    #[test]
    fn a_thin_spike_survives_downsampling() {
        // 10 cm pixels: one 140 m pixel just NE of centre must set its 1 m cell to 140
        let spec = GridSpec::around(LAT, LON, 5.0, 1.0);
        let mut out = spec.empty();
        accumulate_max(&spec, &mut out, &raster(100, 0.1, |i, j| if i == 52 && j == 47 { 140.0 } else { 90.0 }));
        let w = spec.width as usize;
        assert_eq!(out[4 * w + 5], 140.0); // row 4 is just north of centre, col 5 just east
        assert_eq!(out.iter().filter(|&&v| v == 140.0).count(), 1);
    }

    #[test]
    fn north_is_up_and_west_is_left() {
        // left half low, top half high -> NW corner cell = max of both = 3
        let spec = GridSpec::around(LAT, LON, 2.0, 1.0);
        let mut out = spec.empty();
        accumulate_max(&spec, &mut out, &raster(40, 0.1, |i, j| if j < 20 { 3.0 } else if i < 20 { 1.0 } else { 2.0 }));
        let w = spec.width as usize;
        assert_eq!(out[0], 3.0); // north row
        assert_eq!(out[(w - 1) * w], 1.0); // south-west
        assert_eq!(out[w * w - 1], 2.0); // south-east
    }

    #[test]
    fn nodata_pixels_are_skipped() {
        let spec = GridSpec::around(LAT, LON, 1.0, 1.0);
        let mut out = spec.empty();
        accumulate_max(&spec, &mut out, &raster(20, 0.1, |i, _| if i < 10 { -9999.0 } else { 50.0 }));
        let w = spec.width as usize;
        assert_eq!(out[0], NODATA);
        assert_eq!(out[w - 1], 50.0);
    }

    #[test]
    fn pixels_outside_grid_are_ignored() {
        let spec = GridSpec::around(LAT, LON, 1.0, 1.0);
        let mut out = spec.empty();
        accumulate_max(&spec, &mut out, &raster(400, 0.1, |_, _| 7.0)); // 40 m raster, 2 m grid
        assert!(out.iter().all(|&v| v == 7.0));
    }

    #[test]
    fn overlay_prefers_fine() {
        assert_eq!(overlay(&[NODATA, 5.0], &[1.0, 2.0]), vec![1.0, 5.0]);
    }
}
