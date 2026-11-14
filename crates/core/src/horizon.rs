//! The skyline from one eye point on a height map (`HorizonProfile`, docs/contracts.md).
//!
//! For each azimuth we walk outward from the observer in a straight line, one grid cell at a time,
//! and look at the angle up (or down) to every surface we pass:
//!
//! ```text
//!   elev = atan2(height - eye_m, distance)
//! ```
//!
//! The skyline in that direction is the largest such angle, and its distance is where it happened.
//!
//! Simplifications, on purpose:
//! - **Flat earth.** We ignore the earth's curvature. Over 1.5 km the ground drops away by
//!   d^2 / 2R ~ 0.18 m, under 0.01 deg of elevation: far below the 1 m grid's own error.
//!   Refraction is ignored too (it partly cancels curvature anyway).
//! - **Nearest cell, no interpolation.** Each sample reads the cell it falls in.
//! - **One sample per cell size.** A wall at least one cell thick, crossed at any angle, is always
//!   hit; a one-cell wall crossed exactly diagonally could be clipped at a corner.
//!
//! Pure: no IO, no clock. The caller loads the `DsmGrid`.

use std::fmt;

use contracts::dsm::DsmGrid;
use contracts::horizon::{HorizonPoint, HorizonProfile, Observer};

/// What to compute: the `/api/horizon` query, minus `date`/`tz` (that is the sunblock task).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizonQuery {
    pub lat: f64,
    pub lon: f64,
    /// Eye height above the DSM surface at (lat, lon). On a roof the surface is the roof.
    pub above_m: f64,
    /// How far out to look for obstructions, metres.
    pub radius_m: f64,
    /// Azimuth step, degrees.
    pub step_deg: f64,
    /// First azimuth, degrees clockwise from north. Points run from here, `step_deg` apart.
    pub az_from: f64,
    /// End azimuth (exclusive), clockwise from `az_from`; may be smaller to wrap past 360.
    /// `az_from == az_to` (e.g. 0 and 360) means the full circle.
    pub az_to: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HorizonError {
    /// The observer is outside the grid.
    ObserverOffGrid,
    /// The grid has no height data under the observer.
    ObserverNoData,
    /// A query number is out of range (non-finite, or radius/step not positive).
    BadQuery(&'static str),
}

impl fmt::Display for HorizonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HorizonError::ObserverOffGrid => write!(f, "observer is outside the height map"),
            HorizonError::ObserverNoData => write!(f, "no height data under the observer"),
            HorizonError::BadQuery(why) => write!(f, "bad horizon query: {why}"),
        }
    }
}

impl std::error::Error for HorizonError {}

/// The observer's place on the grid: fractional (col, row) and eye height. Work out once,
/// then reuse for every ray (search does many rays per spot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Eye {
    pub col: f64,
    pub row: f64,
    pub eye_m: f64,
}

/// The highest obstruction along one ray.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayMax {
    pub elev_deg: f64,
    /// None if the ray met no height data at all.
    pub dist_m: Option<f64>,
}

/// The skyline from `q.lat, q.lon` over the azimuth window in `q`. `sun` is left `None`.
pub fn horizon(grid: &DsmGrid, q: &HorizonQuery) -> Result<HorizonProfile, HorizonError> {
    check_query(q)?;
    let ground_m = observer_ground(grid, q.lat, q.lon)?;
    let eye_m = ground_m + q.above_m;
    let (col, row) = grid.cell_of(q.lat, q.lon);
    let eye = Eye { col, row, eye_m };

    let n = point_count(q.az_from, q.az_to, q.step_deg);
    let mut points = Vec::with_capacity(n);
    for i in 0..n {
        let az_deg = normalize_az(q.az_from + i as f64 * q.step_deg);
        let m = ray_max(grid, eye, az_deg, q.radius_m);
        points.push(HorizonPoint { az_deg, elev_deg: m.elev_deg, dist_m: m.dist_m });
    }

    Ok(HorizonProfile {
        observer: Observer { lat: q.lat, lon: q.lon, ground_m, eye_m },
        radius_m: q.radius_m,
        step_deg: q.step_deg,
        points,
        sun: None,
    })
}

/// DSM surface height under the observer, or why there isn't one.
pub fn observer_ground(grid: &DsmGrid, lat: f64, lon: f64) -> Result<f64, HorizonError> {
    let (c, r) = grid.cell_of(lat, lon);
    if !in_grid(grid, c, r) {
        return Err(HorizonError::ObserverOffGrid);
    }
    grid.height_at(lat, lon).map(f64::from).ok_or(HorizonError::ObserverNoData)
}

/// Walk from `eye` towards `az_deg`, one cell size per step, up to `radius_m`, and keep the
/// highest angle. Nodata cells are skipped; leaving the grid ends the walk (a straight line
/// never comes back into a rectangle). No data at all gives the contract's flat horizon:
/// elev 0, dist None. Allocates nothing.
pub fn ray_max(grid: &DsmGrid, eye: Eye, az_deg: f64, radius_m: f64) -> RayMax {
    ray_max_below(grid, eye, az_deg, radius_m, f64::INFINITY)
}

/// `ray_max`, but told that nothing in the grid is higher than `top_m` (e.g. the grid's
/// maximum, worked out once). Then the walk stops as soon as even a `top_m` tower further out
/// could not beat the best slope so far: same answer, far fewer steps. Search uses this.
/// Only when `top_m` is above the eye: below it, the bound (top_m - eye) / d is negative and
/// grows towards 0 with distance, so a farther cell could still win and we walk to the end.
pub fn ray_max_below(grid: &DsmGrid, eye: Eye, az_deg: f64, radius_m: f64, top_m: f64) -> RayMax {
    let cell_m = grid.meta.cell_m;
    let (sin_az, cos_az) = az_deg.to_radians().sin_cos();
    // Grid steps per metre walked: east is +col, north is -row.
    let dcol = sin_az / cell_m;
    let drow = -cos_az / cell_m;
    let steps = (radius_m / cell_m).floor() as usize;

    let mut best = RayMax { elev_deg: 0.0, dist_m: None };
    let mut best_tan = f64::NEG_INFINITY;
    let can_stop_early = top_m > eye.eye_m;
    for k in 1..=steps {
        let d = k as f64 * cell_m;
        let (c, r) = (eye.col + d * dcol, eye.row + d * drow);
        if !in_grid(grid, c, r) || (can_stop_early && (top_m - eye.eye_m) / d <= best_tan) {
            break;
        }
        let Some(h) = cell_height(grid, c, r) else { continue };
        // Compare slopes (cheap, same order as angles since d > 0); atan only for the winner.
        let tan = (h - eye.eye_m) / d;
        if tan > best_tan {
            best_tan = tan;
            best.dist_m = Some(d);
        }
    }
    if best.dist_m.is_some() {
        best.elev_deg = best_tan.atan().to_degrees();
    }
    best
}

/// How many points `[az_from, az_to)` holds at `step_deg`. Equal ends mean the full circle.
pub fn point_count(az_from: f64, az_to: f64, step_deg: f64) -> usize {
    let mut span = (az_to - az_from).rem_euclid(360.0);
    if span == 0.0 {
        span = 360.0;
    }
    // Small tolerance so 360 / 0.1 doesn't give 3601 from rounding.
    (span / step_deg - 1e-9).ceil() as usize
}

/// Azimuth folded into [0, 360).
pub fn normalize_az(az: f64) -> f64 {
    let a = az.rem_euclid(360.0);
    if a >= 360.0 {
        0.0
    } else {
        a
    }
}

fn check_query(q: &HorizonQuery) -> Result<(), HorizonError> {
    let nums = [q.lat, q.lon, q.above_m, q.radius_m, q.step_deg, q.az_from, q.az_to];
    if nums.iter().any(|x| !x.is_finite()) {
        return Err(HorizonError::BadQuery("numbers must be finite"));
    }
    if q.radius_m <= 0.0 {
        return Err(HorizonError::BadQuery("radius_m must be positive"));
    }
    if q.step_deg <= 0.0 {
        return Err(HorizonError::BadQuery("step_deg must be positive"));
    }
    Ok(())
}

fn in_grid(grid: &DsmGrid, c: f64, r: f64) -> bool {
    c >= 0.0 && r >= 0.0 && c < f64::from(grid.meta.width) && r < f64::from(grid.meta.height)
}

/// Height of the cell containing fractional (c, r), which must be in the grid; None if nodata.
fn cell_height(grid: &DsmGrid, c: f64, r: f64) -> Option<f64> {
    let v = grid.data[r as usize * grid.meta.width as usize + c as usize];
    (v != grid.meta.nodata).then_some(f64::from(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::dsm::{DsmMeta, DsmSource, NODATA};

    const LAT: f64 = 34.049;
    const LON: f64 = -118.251;
    const N: u32 = 201; // odd, so the centre point sits in the middle of cell (100, 100)

    /// A flat N x N grid of 1 m cells at `ground`, then `paint` sets cells by (col, row).
    fn grid(ground: f32, paint: impl Fn(u32, u32) -> Option<f32>) -> DsmGrid {
        let mut data = Vec::with_capacity((N * N) as usize);
        for r in 0..N {
            for c in 0..N {
                data.push(paint(c, r).unwrap_or(ground));
            }
        }
        DsmGrid {
            meta: DsmMeta {
                center_lat: LAT,
                center_lon: LON,
                cell_m: 1.0,
                width: N,
                height: N,
                nodata: NODATA,
                source: DsmSource {
                    provider: "synthetic".into(),
                    collection: None,
                    captured_from: None,
                    captured_to: None,
                    native_res_m: 1.0,
                },
            },
            data,
        }
    }

    fn query(above_m: f64, az_from: f64, az_to: f64, step_deg: f64) -> HorizonQuery {
        HorizonQuery { lat: LAT, lon: LON, above_m, radius_m: 1500.0, step_deg, az_from, az_to }
    }

    fn point(p: &HorizonProfile, az: f64) -> &HorizonPoint {
        p.points.iter().find(|x| (x.az_deg - az).abs() < 1e-9).expect("azimuth in profile")
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn wall_to_the_east_gives_exact_angle() {
        // Observer at col 100.5 (middle of cell 100). A 20 m tall, 3-cell-thick wall in
        // cols 130..133 spanning all rows. Walking east we land in col 130 after 30 m.
        let g = grid(0.0, |c, _| (130..133).contains(&c).then_some(20.0));
        let p = horizon(&g, &query(1.7, 0.0, 360.0, 1.0)).unwrap();
        assert_eq!(p.points.len(), 360);
        assert_eq!(p.observer.ground_m, 0.0);
        assert_eq!(p.observer.eye_m, 1.7);

        let east = point(&p, 90.0);
        assert!(close(east.elev_deg, (18.3f64 / 30.0).atan().to_degrees()), "{east:?}");
        assert_eq!(east.dist_m, Some(30.0));
    }

    #[test]
    fn flat_ground_away_from_the_wall() {
        let g = grid(0.0, |c, _| (130..133).contains(&c).then_some(20.0));
        let p = horizon(&g, &query(1.7, 0.0, 360.0, 1.0)).unwrap();
        // West: only flat ground at 0 m, the grid ends ~100 m out, so the farthest ground cell
        // is the highest angle: atan(-1.7 / 100), just below level.
        let west = point(&p, 270.0);
        assert!(close(west.elev_deg, (-1.7f64 / 100.0).atan().to_degrees()), "{west:?}");
        assert_eq!(west.dist_m, Some(100.0));
        // Every direction pointing away from the wall stays below eye level.
        for pt in p.points.iter().filter(|x| x.az_deg > 180.0) {
            assert!(pt.elev_deg < 0.0, "{pt:?}");
        }
    }

    #[test]
    fn observer_on_a_tall_roof_sees_down() {
        // 60 m roof, cols/rows 90..=110 around the observer, flat 0 m streets elsewhere.
        let roof = |c: u32, r: u32| ((90..=110).contains(&c) && (90..=110).contains(&r)).then_some(60.0);
        let g = grid(0.0, roof);
        let p = horizon(&g, &query(1.7, 0.0, 360.0, 10.0)).unwrap();
        assert_eq!(p.observer.ground_m, 60.0);
        for pt in &p.points {
            assert!(pt.elev_deg < 0.0, "{pt:?}");
        }
        // East: roof cells to col 110 (10 m out); the roof edge is the skyline.
        let east = point(&p, 90.0);
        assert!(close(east.elev_deg, (-1.7f64 / 10.0).atan().to_degrees()), "{east:?}");
        assert_eq!(east.dist_m, Some(10.0));
    }

    #[test]
    fn nodata_direction_is_flat_with_no_distance() {
        // Everything east of the observer's cell is nodata.
        let g = grid(0.0, |c, _| (c > 100).then_some(NODATA));
        let p = horizon(&g, &query(1.7, 0.0, 360.0, 1.0)).unwrap();
        let east = point(&p, 90.0);
        assert_eq!((east.elev_deg, east.dist_m), (0.0, None));
        // West still sees data.
        assert!(point(&p, 270.0).dist_m.is_some());
    }

    #[test]
    fn nodata_cells_are_skipped_not_fatal() {
        // A nodata gap then a wall: the wall still counts.
        let g = grid(0.0, |c, _| match c {
            101..=120 => Some(NODATA),
            130..=132 => Some(20.0),
            _ => None,
        });
        let east = horizon(&g, &query(1.7, 85.0, 95.0, 5.0)).unwrap().points[1].clone();
        assert_eq!(east.az_deg, 90.0);
        assert_eq!(east.dist_m, Some(30.0));
    }

    #[test]
    fn radius_limits_the_search() {
        let g = grid(0.0, |c, _| (130..133).contains(&c).then_some(20.0));
        let q = HorizonQuery { radius_m: 25.0, ..query(1.7, 90.0, 91.0, 1.0) };
        let east = horizon(&g, &q).unwrap().points[0].clone();
        // The wall at 30 m is out of range; the farthest ground cell, 25 m, wins.
        assert_eq!(east.dist_m, Some(25.0));
        assert!(east.elev_deg < 0.0);
    }

    #[test]
    fn early_exit_gives_the_same_answer() {
        let g = grid(0.0, |c, r| match (c, r) {
            (130..=132, _) => Some(20.0),
            (150..=152, _) => Some(45.0),
            (_, 0..=60) => Some(8.0),
            _ => None,
        });
        let eye = Eye { col: 100.5, row: 100.5, eye_m: 1.7 };
        for az in (0..360).map(f64::from) {
            assert_eq!(ray_max(&g, eye, az, 1500.0), ray_max_below(&g, eye, az, 1500.0, 45.0), "az {az}");
        }
    }

    #[test]
    fn early_exit_is_safe_on_the_highest_roof() {
        // Eye 1.7 m above the tallest thing in the grid: the bound only grows with distance,
        // so ray_max_below must walk on (the roof edge at 10 m, not the first roof cell, wins).
        let g = grid(0.0, |c, r| match (c, r) {
            (90..=110, 90..=110) => Some(60.0),
            (150..=152, _) => Some(50.0),
            _ => None,
        });
        let eye = Eye { col: 100.5, row: 100.5, eye_m: 61.7 };
        for az in (0..360).map(f64::from) {
            assert_eq!(ray_max(&g, eye, az, 1500.0), ray_max_below(&g, eye, az, 1500.0, 60.0), "az {az}");
        }
        assert_eq!(ray_max_below(&g, eye, 90.0, 1500.0, 60.0).dist_m, Some(10.0));
    }

    #[test]
    fn window_wraps_past_north() {
        let g = grid(0.0, |_, _| None);
        let p = horizon(&g, &query(1.7, 350.0, 10.0, 5.0)).unwrap();
        let azs: Vec<f64> = p.points.iter().map(|x| x.az_deg).collect();
        assert_eq!(azs, vec![350.0, 355.0, 0.0, 5.0]);
    }

    #[test]
    fn window_past_360_numbers_also_wrap() {
        let g = grid(0.0, |_, _| None);
        let p = horizon(&g, &query(1.7, 355.0, 365.0, 5.0)).unwrap();
        let azs: Vec<f64> = p.points.iter().map(|x| x.az_deg).collect();
        assert_eq!(azs, vec![355.0, 0.0]);
    }

    #[test]
    fn point_counts() {
        assert_eq!(point_count(0.0, 360.0, 1.0), 360);
        assert_eq!(point_count(0.0, 0.0, 10.0), 36);
        assert_eq!(point_count(0.0, 360.0, 0.1), 3600);
        assert_eq!(point_count(280.0, 300.0, 0.5), 40);
        assert_eq!(point_count(0.0, 360.0, 7.0), 52); // 0, 7, ..., 357
    }

    #[test]
    fn observer_off_grid_or_on_nodata_is_an_error() {
        let g = grid(0.0, |c, r| (c == 100 && r == 100).then_some(NODATA));
        assert_eq!(horizon(&g, &query(1.7, 0.0, 360.0, 1.0)), Err(HorizonError::ObserverNoData));
        let far = HorizonQuery { lat: LAT + 1.0, ..query(1.7, 0.0, 360.0, 1.0) };
        assert_eq!(horizon(&g, &far), Err(HorizonError::ObserverOffGrid));
    }

    #[test]
    fn bad_numbers_are_rejected() {
        let g = grid(0.0, |_, _| None);
        let zero_step = query(1.7, 0.0, 360.0, 0.0);
        assert!(matches!(horizon(&g, &zero_step), Err(HorizonError::BadQuery(_))));
        let nan = HorizonQuery { radius_m: f64::NAN, ..query(1.7, 0.0, 360.0, 1.0) };
        assert!(matches!(horizon(&g, &nan), Err(HorizonError::BadQuery(_))));
    }

    #[test]
    fn diagonal_ray_hits_a_tower() {
        // Tower 10x10 cells to the north-east, its near corner 20 cells east and 20 north.
        let g = grid(0.0, |c, r| ((120..130).contains(&c) && (71..81).contains(&r)).then_some(50.0));
        let ne = horizon(&g, &query(1.7, 45.0, 46.0, 1.0)).unwrap().points[0].clone();
        let d = ne.dist_m.unwrap();
        // First sample inside the tower: ~19.5 * sqrt(2) = 27.6 m out, give or take a step.
        assert!((27.0..30.0).contains(&d), "{ne:?}");
        assert!(close(ne.elev_deg, (48.3 / d).atan().to_degrees()));
    }

    #[test]
    fn fixture_grid_centre_is_90_m() {
        // tests may read files; the library itself never does
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let meta = DsmGrid::parse_meta(&std::fs::read_to_string(dir.join("dsm_grid.json")).unwrap()).unwrap();
        let g = DsmGrid::from_parts(meta.meta, &std::fs::read(dir.join(&meta.data_file)).unwrap()).unwrap();

        let q = HorizonQuery {
            lat: g.meta.center_lat,
            lon: g.meta.center_lon,
            above_m: 1.7,
            radius_m: 1500.0,
            step_deg: 1.0,
            az_from: 0.0,
            az_to: 360.0,
        };
        let p = horizon(&g, &q).unwrap();
        assert_eq!(p.observer.ground_m, 90.0);
        assert!(close(p.observer.eye_m, 91.7));
        assert_eq!(p.points.len(), 360);

        // The fixture has a 102 m block east of centre (cols 36..=45) starting 4 cells out,
        // and a 150 m tower to the west-north-west (cols 6..=13, rows 18..=28).
        let east = point(&p, 90.0);
        assert!(close(east.elev_deg, (10.3f64 / 4.0).atan().to_degrees()), "{east:?}");
        assert_eq!(east.dist_m, Some(4.0));

        let tower_az = (-22.5f64).atan2(9.0).to_degrees().rem_euclid(360.0).round(); // tower centre: 22.5 m W, 9 m N
        let tower = point(&p, tower_az);
        assert!(tower.elev_deg > 60.0 && tower.dist_m.unwrap() < 25.0, "{tower:?}");

        // South is open flat 90 m roofs/ground to the edge, 32 m out: just below level.
        let south = point(&p, 180.0);
        assert!(south.elev_deg < 0.0 && south.elev_deg > -5.0, "{south:?}");

    }
}
