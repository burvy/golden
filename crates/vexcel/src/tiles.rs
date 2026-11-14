//! Pure tile maths on Vexcel's global EPSG:3857 pixel grid.
//!
//! Measured against the live API (see docs/Nube-Lube/DEVLOG.md):
//! - Level 0 pixels are exactly `WORLD / 2^29` = 0.0746 3857 units (~6.2 cm on the ground in LA).
//! - Vexcel keeps level 0 while the requested box is at most ~8192 px across, then halves the detail.
//! - A POLYGON request comes back snapped to this global grid, so tiles line up and cache cleanly.

/// Half the width of the EPSG:3857 world, in 3857 units.
pub const HALF_WORLD: f64 = 20_037_508.342_789_244;
/// Earth radius used by EPSG:3857 (and by our ground-metre grid).
pub const EARTH_R: f64 = 6_378_137.0;
/// Level 0 pixel size in 3857 units.
pub const LEVEL0_SCALE: f64 = 2.0 * HALF_WORLD / (1u64 << 29) as f64;

/// Pixels per tile side. Level 0 tiles are small (~63 m) so a spot fetches little.
/// Level 1 tiles must be over 4096 px: if they were smaller, the same box at level 0
/// would fit under Vexcel's ~8192 px limit and it would send us the finer (4x bigger) data.
pub fn tile_px(level: u8) -> u32 {
    if level == 0 {
        1024
    } else {
        4608
    }
}

/// Pixel size in 3857 units at a level.
pub fn scale(level: u8) -> f64 {
    LEVEL0_SCALE * f64::from(1u32 << level)
}

/// Tile side in 3857 units.
pub fn tile_span(level: u8) -> f64 {
    scale(level) * f64::from(tile_px(level))
}

pub fn lonlat_to_merc(lon: f64, lat: f64) -> (f64, f64) {
    let x = EARTH_R * lon.to_radians();
    let y = EARTH_R * (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln();
    (x, y)
}

pub fn merc_to_lonlat(x: f64, y: f64) -> (f64, f64) {
    let lon = (x / EARTH_R).to_degrees();
    let lat = (2.0 * (y / EARTH_R).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
    (lon, lat)
}

/// A tile on the global grid. `ty` counts down from the north edge of the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub level: u8,
    pub tx: i64,
    pub ty: i64,
}

/// 3857 bounds of a box: (min_x, min_y, max_x, max_y).
pub type MercBox = (f64, f64, f64, f64);

pub fn tile_bounds(t: TileId) -> MercBox {
    let span = tile_span(t.level);
    let min_x = -HALF_WORLD + t.tx as f64 * span;
    let max_y = HALF_WORLD - t.ty as f64 * span;
    (min_x, max_y - span, min_x + span, max_y)
}

/// Every tile at `level` that touches `b`.
pub fn tiles_covering(level: u8, b: MercBox) -> Vec<TileId> {
    let span = tile_span(level);
    let (min_x, min_y, max_x, max_y) = b;
    let tx0 = ((min_x + HALF_WORLD) / span).floor() as i64;
    let tx1 = ((max_x + HALF_WORLD) / span).floor() as i64;
    let ty0 = ((HALF_WORLD - max_y) / span).floor() as i64;
    let ty1 = ((HALF_WORLD - min_y) / span).floor() as i64;
    let mut out = Vec::new();
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            out.push(TileId { level, tx, ty });
        }
    }
    out
}

/// 3857 box around a point that covers `half_ground_m` ground metres in every direction.
/// 3857 units are stretched by 1/cos(lat), so a ground metre is more than one unit.
pub fn ground_box(lat: f64, lon: f64, half_ground_m: f64) -> MercBox {
    let (x, y) = lonlat_to_merc(lon, lat);
    let half = half_ground_m / lat.to_radians().cos();
    (x - half, y - half, x + half, y + half)
}

/// WKT polygon (lon lat, EPSG:4326) of a tile, as Vexcel's `extract` wants it.
pub fn tile_wkt(t: TileId) -> String {
    let (min_x, min_y, max_x, max_y) = tile_bounds(t);
    let (w, s) = merc_to_lonlat(min_x, min_y);
    let (e, n) = merc_to_lonlat(max_x, max_y);
    format!("POLYGON(({w:.9} {s:.9},{e:.9} {s:.9},{e:.9} {n:.9},{w:.9} {n:.9},{w:.9} {s:.9}))")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level0_scale_matches_vexcel_response() {
        // pixelscale tag from a real downtown LA response
        assert!((LEVEL0_SCALE - 0.074_645_535_434).abs() < 1e-9);
        assert!((scale(1) - 0.149_291_070_87).abs() < 1e-9);
    }

    #[test]
    fn merc_round_trip() {
        let (x, y) = lonlat_to_merc(-118.251, 34.049);
        let (lon, lat) = merc_to_lonlat(x, y);
        assert!((lon + 118.251).abs() < 1e-9 && (lat - 34.049).abs() < 1e-9);
    }

    #[test]
    fn global_pixel_matches_vexcel_pixel_bbox() {
        // Vexcel reported pixel-bbox x=92086298 for the 60 m box around this point;
        // that box's west edge was at lon -118.2513253390789.
        let (x, _) = lonlat_to_merc(-118.251_325_339_078_9, 34.049);
        let gx = (x + HALF_WORLD) / LEVEL0_SCALE;
        assert!((gx - 92_086_298.0).abs() < 1.0, "gx = {gx}");
    }

    #[test]
    fn tile_contains_its_own_centre() {
        let t = TileId { level: 0, tx: 89_928, ty: 209_350 };
        let (a, b, c, d) = tile_bounds(t);
        let got = tiles_covering(0, ((a + c) / 2.0, (b + d) / 2.0, (a + c) / 2.0, (b + d) / 2.0));
        assert_eq!(got, vec![t]);
    }

    #[test]
    fn covering_a_box_spans_whole_tiles() {
        // 150 m ground radius at LA is ~181 3857 units each way; level 0 tiles are ~76 units.
        let tiles = tiles_covering(0, ground_box(34.049, -118.251, 150.0));
        let n = (tiles.len() as f64).sqrt();
        assert!((5.0..=7.0).contains(&n), "{} tiles", tiles.len());
        for t in &tiles {
            let (a, b, c, d) = tile_bounds(*t);
            assert!(c > a && d > b);
        }
    }

    #[test]
    fn level1_tile_is_too_big_for_level0() {
        // the reason tile_px(1) > 4096: twice it must exceed Vexcel's ~8192 px limit
        assert!(tile_px(1) * 2 > 8192 && tile_px(1) <= 8192);
        assert!(tile_px(0) <= 8192);
    }

    #[test]
    fn ground_box_accounts_for_mercator_stretch() {
        let (a, _, c, _) = ground_box(34.0, -118.0, 100.0);
        let want = 200.0 / 34f64.to_radians().cos();
        assert!(((c - a) - want).abs() < 1e-6);
    }

    #[test]
    fn wkt_is_closed_polygon() {
        let w = tile_wkt(TileId { level: 0, tx: 89_928, ty: 209_350 });
        assert!(w.starts_with("POLYGON((") && w.ends_with("))"));
        let pts: Vec<&str> = w[9..w.len() - 2].split(',').collect();
        assert_eq!(pts.len(), 5);
        assert_eq!(pts[0], pts[4]);
    }
}
