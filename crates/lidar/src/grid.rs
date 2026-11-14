//! Pure: LiDAR points (EPSG:3857 x, y; height z) to a ground-metre `DsmGrid` layout.
//!
//! Each cell keeps its HIGHEST point, like Vexcel's DSM (a roof edge anywhere in the cell blocks the sun).
//! Noise classes are dropped. Small gaps (a cell no return landed in) are filled from their neighbours.

use contracts::{EARTH_R, NODATA};

/// ASPRS classes to drop: 7 low noise, 18 high noise (birds, clouds), 0 never classified is kept.
pub const NOISE: [u8; 2] = [7, 18];

/// Grid placement, same meaning as `DsmMeta`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spec {
    pub center_lat: f64,
    pub center_lon: f64,
    pub cell_m: f64,
    pub width: u32,
    pub height: u32,
}

impl Spec {
    /// Square grid covering `radius_m` ground metres around the centre.
    pub fn around(lat: f64, lon: f64, radius_m: f64, cell_m: f64) -> Spec {
        let n = (2.0 * radius_m / cell_m).ceil().max(1.0) as u32;
        Spec { center_lat: lat, center_lon: lon, cell_m, width: n, height: n }
    }

    pub fn empty(&self) -> Vec<f32> {
        vec![NODATA; self.width as usize * self.height as usize]
    }

    /// Cell index of a 3857 point, per the contract formula (equirectangular around the centre).
    pub fn cell(&self, x: f64, y: f64) -> Option<usize> {
        let lon = (x / EARTH_R).to_degrees();
        let lat = (y / EARTH_R).sinh().atan().to_degrees();
        let east = (lon - self.center_lon).to_radians() * EARTH_R * self.center_lat.to_radians().cos();
        let north = (lat - self.center_lat).to_radians() * EARTH_R;
        let c = (f64::from(self.width) / 2.0 + east / self.cell_m).floor();
        let r = (f64::from(self.height) / 2.0 - north / self.cell_m).floor();
        let inside = c >= 0.0 && r >= 0.0 && c < f64::from(self.width) && r < f64::from(self.height);
        inside.then(|| r as usize * self.width as usize + c as usize)
    }

    /// 3857 box that contains the grid plus `pad_m` ground metres.
    pub fn merc_box(&self, pad_m: f64) -> crate::ept::Rect {
        let half_e = f64::from(self.width) * self.cell_m / 2.0 + pad_m;
        let half_n = f64::from(self.height) * self.cell_m / 2.0 + pad_m;
        let cos = self.center_lat.to_radians().cos();
        let cx = EARTH_R * self.center_lon.to_radians();
        let lat_n = self.center_lat + (half_n / EARTH_R).to_degrees();
        let lat_s = self.center_lat - (half_n / EARTH_R).to_degrees();
        crate::ept::Rect {
            min_x: cx - half_e / cos,
            max_x: cx + half_e / cos,
            min_y: merc_y(lat_s),
            max_y: merc_y(lat_n),
        }
    }
}

pub fn merc_y(lat_deg: f64) -> f64 {
    EARTH_R * (std::f64::consts::FRAC_PI_4 + lat_deg.to_radians() / 2.0).tan().ln()
}

/// One LiDAR return.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub class: u8,
}

/// Fold points into `out` with a per-cell max. Returns how many landed in the grid.
pub fn accumulate_max(spec: &Spec, out: &mut [f32], points: impl IntoIterator<Item = Point>) -> usize {
    let mut n = 0;
    for p in points {
        if NOISE.contains(&p.class) || !p.z.is_finite() {
            continue;
        }
        if let Some(i) = spec.cell(p.x, p.y) {
            let z = p.z as f32;
            if out[i] == NODATA || z > out[i] {
                out[i] = z;
            }
            n += 1;
        }
    }
    n
}

/// Fill NODATA cells that have at least 3 filled neighbours with the max of those neighbours.
/// One pass closes the pinholes between returns without growing buildings into empty areas.
pub fn fill_gaps(spec: &Spec, data: &[f32]) -> Vec<f32> {
    let (w, h) = (spec.width as i64, spec.height as i64);
    let mut out = data.to_vec();
    for r in 0..h {
        for c in 0..w {
            let i = (r * w + c) as usize;
            if data[i] != NODATA {
                continue;
            }
            let (mut best, mut count) = (NODATA, 0);
            for (dr, dc) in [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)] {
                let (rr, cc) = (r + dr, c + dc);
                if rr < 0 || cc < 0 || rr >= h || cc >= w {
                    continue;
                }
                let v = data[(rr * w + cc) as usize];
                if v != NODATA {
                    count += 1;
                    if best == NODATA || v > best {
                        best = v;
                    }
                }
            }
            if count >= 3 {
                out[i] = best;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merc(lat: f64, lon: f64) -> (f64, f64) {
        (EARTH_R * lon.to_radians(), merc_y(lat))
    }

    fn spec() -> Spec {
        Spec::around(40.3487, -74.6593, 10.0, 1.0)
    }

    #[test]
    fn centre_and_directions_land_in_the_right_cells() {
        let s = spec();
        assert_eq!((s.width, s.height), (20, 20));
        let (x, y) = merc(40.3487, -74.6593);
        assert_eq!(s.cell(x, y), Some(10 * 20 + 10));
        // 4.5 m north -> row floor(10 - 4.5) = 5; 4.5 m east -> column floor(10 + 4.5) = 14
        let (x, y) = merc(40.3487 + (4.5f64 / EARTH_R).to_degrees(), -74.6593);
        assert_eq!(s.cell(x, y), Some(5 * 20 + 10));
        let east_deg = (4.5f64 / (EARTH_R * 40.3487f64.to_radians().cos())).to_degrees();
        let (x, y) = merc(40.3487, -74.6593 + east_deg);
        assert_eq!(s.cell(x, y), Some(10 * 20 + 14));
        let (x, y) = merc(40.3487, -74.6593 + 0.01);
        assert_eq!(s.cell(x, y), None);
    }

    #[test]
    fn keeps_the_highest_point_and_drops_noise() {
        let s = spec();
        let (x, y) = merc(40.3487, -74.6593);
        let mut g = s.empty();
        let n = accumulate_max(
            &s,
            &mut g,
            [
                Point { x, y, z: 64.0, class: 2 },
                Point { x, y, z: 80.0, class: 1 },
                Point { x, y, z: 300.0, class: 18 },
            ],
        );
        assert_eq!(n, 2);
        assert_eq!(g[10 * 20 + 10], 80.0);
    }

    #[test]
    fn merc_box_contains_the_grid() {
        let s = spec();
        let b = s.merc_box(0.0);
        let (x, y) = merc(40.3487, -74.6593);
        assert!(b.min_x < x && x < b.max_x && b.min_y < y && y < b.max_y);
        // 20 ground metres wide -> 20 / cos(lat) mercator metres
        assert!(((b.max_x - b.min_x) - 20.0 / 40.3487f64.to_radians().cos()).abs() < 1e-6);
    }

    #[test]
    fn fills_pinholes_but_not_open_ground() {
        let s = Spec { center_lat: 0.0, center_lon: 0.0, cell_m: 1.0, width: 3, height: 3 };
        let mut d = vec![5.0; 9];
        d[4] = NODATA; // hole in a roof
        assert_eq!(fill_gaps(&s, &d)[4], 5.0);
        let mut lone = vec![NODATA; 9];
        lone[0] = 9.0; // one point; centre has 1 neighbour -> stays empty
        assert_eq!(fill_gaps(&s, &lone)[4], NODATA);
    }
}
