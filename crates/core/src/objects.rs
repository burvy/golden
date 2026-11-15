//! Objects seen from a standpoint: geometry, line of sight, which objects a goal means, and where
//! to look for standpoints that line two objects up (docs/metrics.md v2).
//!
//! Objects come in as plain `contracts::Object`s (CoStar buildings, Vexcel detections, ...); core
//! doesn't care where from. Everything here is a pure function of the objects, the height map and
//! the standpoint.
//!
//! - **Geometry** (`dist`, `bearing`, `size_deg`, `bearing_diff`, `in_frame`): from the eye to the
//!   object's centre, on the same flat-earth formula as `DsmGrid`.
//! - **Line of sight** (`visible`): one march from the eye towards the object's near side, keeping
//!   the steepest blocker; each of a few heights up the object is in view if it's above that.
//! - **Binding** (`bind`): a goal's object references (`{"id": ..}`, `{"kind": .., "near": ..,
//!   "within_m": ..}`) become a short list of concrete choices. A goal scores the best choice at
//!   each standpoint, which is how "the best fountain near the tower" works.
//! - **Marching** (`march`): standpoints on the line from b through a and beyond, where a stands in
//!   front of b. Search adds them as extra candidates when a goal compares two bearings.

use contracts::{DsmGrid, ObjRef, Object, EARTH_R};

use crate::score::{ramp, wrap180};

/// Objects further than this outside the search area are never picked for a `kind` reference
/// (the skyline is traced this far, so nothing beyond it can be judged anyway).
pub const OBJECT_REACH_M: f64 = 1500.0;
/// `near` without `within_m` means within this many metres.
pub const DEFAULT_NEAR_M: f64 = 300.0;
/// At most this many object choices per goal. Kept small: every choice is scored at every candidate.
pub const MAX_BINDINGS: usize = 64;
/// A cost function can name at most this many different objects.
pub const MAX_OBJECTS_PER_FN: usize = 8;
/// Heights up the object that `visible` checks, as fractions from base to top.
pub const VISIBLE_SAMPLES: [f64; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];
/// Something must stick up this much above the line of sight to block it (DSM noise, rails).
pub const BLOCK_TOL_M: f64 = 0.5;
/// The march stops this many radii short of the object's centre, so a footprint that isn't
/// round (a square's corners are 1.41 radii out) doesn't hide its own object.
pub const SELF_RADII: f64 = 1.5;
/// Marching: first standpoint this far beyond a's near side, then each step grows by
/// `MARCH_GROWTH` plus `MARCH_STEP_M`, out to where a's height spans only `MARCH_MIN_SIZE_DEG`.
pub const MARCH_START_M: f64 = 5.0;
pub const MARCH_STEP_M: f64 = 4.0;
pub const MARCH_GROWTH: f64 = 1.06;
pub const MARCH_MIN_SIZE_DEG: f64 = 1.0;
/// Marching: besides the line itself, one standpoint either side, this many degrees off it as
/// seen from a (the lens has room to frame a slightly off-line pair).
pub const MARCH_CONE_DEG: f64 = 2.0;
/// Marching uses only a goal's first this-many object choices (they're nearest the area's centre).
pub const MAX_MARCH_CHOICES: usize = 8;
/// Marching never adds more than this many standpoints to one search.
pub const MAX_MARCH_POINTS: usize = 800;

// ---- geometry ---------------------------------------------------------------------------

/// Metres east and north from `from` to `to` (lat, lon), same flat formula as `DsmGrid`.
pub fn east_north(from: (f64, f64), to: (f64, f64)) -> (f64, f64) {
    let north = (to.0 - from.0).to_radians() * EARTH_R;
    let east = (to.1 - from.1).to_radians() * EARTH_R * from.0.to_radians().cos();
    (east, north)
}

/// The point `east`, `north` metres from `at` (lat, lon). Inverse of `east_north`.
pub fn offset(at: (f64, f64), east: f64, north: f64) -> (f64, f64) {
    (at.0 + (north / EARTH_R).to_degrees(), at.1 + (east / (EARTH_R * at.0.to_radians().cos())).to_degrees())
}

fn pos(o: &Object) -> (f64, f64) {
    (o.lat, o.lon)
}

/// Where the camera is: the candidate's position and eye height, and the brief's lens angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub lat: f64,
    pub lon: f64,
    pub eye_m: f64,
    pub lens_deg: f64,
}

impl View {
    fn at(&self) -> (f64, f64) {
        (self.lat, self.lon)
    }
}

/// Ground distance from the eye to the object's centre, metres.
pub fn dist(v: &View, o: &Object) -> f64 {
    let (e, n) = east_north(v.at(), pos(o));
    e.hypot(n)
}

/// Compass bearing from the eye to the object's centre, degrees 0..360 (0 = north, 90 = east).
pub fn bearing(v: &View, o: &Object) -> f64 {
    let (e, n) = east_north(v.at(), pos(o));
    crate::horizon::normalize_az(e.atan2(n).to_degrees())
}

/// Bearing of `a` minus bearing of `b`, wrapped to -180..180. Positive: `a` is right of `b` in the frame.
pub fn bearing_diff(v: &View, a: &Object, b: &Object) -> f64 {
    wrap180(bearing(v, a) - bearing(v, b))
}

/// Distance to the object's near side (at least 1 m, so standing on it doesn't blow up).
fn front(v: &View, o: &Object) -> f64 {
    (dist(v, o) - o.radius_m).max(1.0)
}

/// How tall the object looks: the angle from its base to its top, degrees, seen at its near side.
pub fn size_deg(v: &View, o: &Object) -> f64 {
    let d = front(v, o);
    (((o.top_m - v.eye_m) / d).atan() - ((o.base_m - v.eye_m) / d).atan()).to_degrees()
}

/// 1 when `a` and `b` are within half the lens angle of each other (both comfortably in one frame),
/// falling to 0 when they're a whole lens angle apart (each on an edge of the frame).
pub fn in_frame(v: &View, a: &Object, b: &Object) -> f64 {
    ramp(bearing_diff(v, a, b).abs(), v.lens_deg, v.lens_deg / 2.0)
}

/// The sun's azimuth minus the object's bearing, wrapped to -180..180.
pub fn sun_bearing_diff(v: &View, sun_az_deg: f64, o: &Object) -> f64 {
    wrap180(sun_az_deg - bearing(v, o))
}

// ---- line of sight ----------------------------------------------------------------------

/// Share of the object in view, 0..1: of `VISIBLE_SAMPLES` heights on its near side, how many
/// the eye can see over the height map. One march, one cell per step, from the eye to
/// `SELF_RADII` short of the object's centre (so the object doesn't hide itself), keeping the
/// steepest blocker. Nodata is skipped and leaving the grid ends the march (unknown counts as clear).
pub fn visible(grid: &DsmGrid, v: &View, o: &Object) -> f64 {
    let (e, n) = east_north(v.at(), pos(o));
    let d = e.hypot(n);
    if d <= o.radius_m {
        return 1.0; // standing at (or on) it
    }
    let reach = d - o.radius_m;
    let tans: Vec<f64> = VISIBLE_SAMPLES.iter().map(|f| (o.base_m + f * (o.top_m - o.base_m) - v.eye_m) / reach).collect();
    let top_tan = tans.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let cell_m = grid.meta.cell_m;
    let (col, row) = grid.cell_of(v.lat, v.lon);
    let (dcol, drow) = (e / d / cell_m, -n / d / cell_m);
    let mut worst = f64::NEG_INFINITY;
    let mut k = 1;
    let stop = d - (SELF_RADII * o.radius_m).max(cell_m);
    loop {
        let s = k as f64 * cell_m;
        if s >= stop {
            break;
        }
        let (c, r) = (col + s * dcol, row + s * drow);
        if c < 0.0 || r < 0.0 || c >= f64::from(grid.meta.width) || r >= f64::from(grid.meta.height) {
            break;
        }
        let h = grid.data[r as usize * grid.meta.width as usize + c as usize];
        if h != grid.meta.nodata {
            worst = worst.max((f64::from(h) - BLOCK_TOL_M - v.eye_m) / s);
            if worst > top_tan {
                return 0.0;
            }
        }
        k += 1;
    }
    tans.iter().filter(|&&t| t >= worst).count() as f64 / tans.len() as f64
}

// ---- binding: which objects a goal means ------------------------------------------------

/// A goal's object choices: `choices[k][slot]` is the index in the object list of the object for
/// `refs[slot]` in choice `k`. Always at least one choice (an empty one if there are no refs).
pub type Choices = Vec<Vec<usize>>;

/// Turn object references into concrete choices from `objects`, for a search around `centre`
/// (lat, lon) with radius `area_m`.
///
/// - `{"id": x}` is that object.
/// - `{"kind": k}` is any object of that kind (case-insensitive) within `area_m + OBJECT_REACH_M`
///   of the centre; with `within_m` and no `near`, within that many metres of the centre instead.
/// - `near: r` adds: within `within_m` (default `DEFAULT_NEAR_M`) of the object chosen for `r`. If
///   `r` is written the same as one of the goal's references, it's that object, so
///   `a = {"kind":"tree","near":{"kind":"building"}}, b = {"kind":"building"}` picks a tree near
///   whichever building b is.
/// - Different references always get different objects, even if written the same (`a` and `b`
///   both `{"kind":"building"}` are two buildings).
///
/// Choices are tried nearest the centre first and capped at `MAX_BINDINGS`. An error says which
/// reference found nothing, so the agent can loosen it.
pub fn bind(refs: &[ObjRef], objects: &[Object], centre: (f64, f64), area_m: f64) -> Result<Choices, String> {
    // One variable per reference, then one for each `near` target that isn't one of them.
    let mut vars: Vec<&ObjRef> = refs.iter().collect();
    let slot_var: Vec<usize> = (0..refs.len()).collect();
    let mut near_of: Vec<Option<(usize, f64)>> = Vec::new();
    let mut i = 0;
    while i < vars.len() {
        near_of.push(match vars[i] {
            ObjRef::Kind { near: Some(n), within_m, .. } => {
                let t = vars.iter().position(|w| *w == &**n).unwrap_or_else(|| {
                    vars.push(n);
                    vars.len() - 1
                });
                Some((t, within_m.unwrap_or(DEFAULT_NEAR_M)))
            }
            _ => None,
        });
        i += 1;
    }
    let order = near_first(&near_of);
    let mut domains = Vec::with_capacity(vars.len());
    for v in &vars {
        let d = domain(v, objects, centre, area_m);
        if d.is_empty() {
            return Err(nothing_for(v, objects, area_m));
        }
        domains.push(d);
    }

    let mut out: Choices = Vec::new();
    let mut pick = vec![usize::MAX; vars.len()];
    let mut budget = 200_000usize; // steps, so a near-impossible combination can't hang a search
    let mut st = Walk { objects, domains: &domains, near_of: &near_of, order: &order, slot_var: &slot_var, out: &mut out, budget: &mut budget };
    st.go(0, &mut pick);
    if out.is_empty() {
        let text: Vec<String> = refs.iter().map(|r| serde_json::to_string(r).unwrap_or_default()).collect();
        return Err(format!("no objects fit {} together; loosen \"near\"/\"within_m\"", text.join(" and ")));
    }
    Ok(out)
}

/// The variables in an order where each `near` target comes before what's near it. (No cycles:
/// a target is written inside its dependent, so it's strictly smaller.)
fn near_first(near_of: &[Option<(usize, f64)>]) -> Vec<usize> {
    fn visit(v: usize, near_of: &[Option<(usize, f64)>], order: &mut Vec<usize>) {
        if order.contains(&v) {
            return;
        }
        if let Some((t, _)) = near_of[v] {
            visit(t, near_of, order);
        }
        order.push(v);
    }
    let mut order = Vec::with_capacity(near_of.len());
    for v in 0..near_of.len() {
        visit(v, near_of, &mut order);
    }
    order
}

/// Objects a reference can be, nearest the centre first (ties by id), ignoring `near`.
fn domain(r: &ObjRef, objects: &[Object], centre: (f64, f64), area_m: f64) -> Vec<usize> {
    let from_centre = |o: &Object| {
        let (e, n) = east_north(centre, pos(o));
        e.hypot(n)
    };
    let mut d: Vec<usize> = match r {
        ObjRef::Id(id) => objects.iter().position(|o| &o.id == id).into_iter().collect(),
        ObjRef::Kind { kind, near, within_m } => {
            let reach = match (near, within_m) {
                (None, Some(m)) => *m,
                _ => area_m + OBJECT_REACH_M,
            };
            (0..objects.len()).filter(|&i| objects[i].kind.eq_ignore_ascii_case(kind) && from_centre(&objects[i]) <= reach).collect()
        }
    };
    d.sort_by(|&a, &b| from_centre(&objects[a]).total_cmp(&from_centre(&objects[b])).then(objects[a].id.cmp(&objects[b].id)));
    d
}

fn nothing_for(r: &ObjRef, objects: &[Object], area_m: f64) -> String {
    match r {
        ObjRef::Id(id) => format!("no object with id \"{id}\" in this area's objects"),
        ObjRef::Kind { kind, near, within_m } => {
            let mut kinds: Vec<(&str, usize)> = Vec::new();
            for o in objects {
                match kinds.iter_mut().find(|(k, _)| *k == o.kind) {
                    Some((_, n)) => *n += 1,
                    None => kinds.push((&o.kind, 1)),
                }
            }
            let have = if kinds.is_empty() {
                "there are no objects here at all".to_string()
            } else {
                let list: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} x{n}")).collect();
                format!("objects here: {}", list.join(", "))
            };
            let reach = match (near, within_m) {
                (None, Some(m)) => *m,
                _ => area_m + OBJECT_REACH_M,
            };
            format!("no object of kind \"{kind}\" within {reach:.0} m of the area's centre ({have})")
        }
    }
}

/// Depth-first walk over the variables in `order` (a `near` target always comes first).
struct Walk<'a> {
    objects: &'a [Object],
    domains: &'a [Vec<usize>],
    near_of: &'a [Option<(usize, f64)>],
    order: &'a [usize],
    slot_var: &'a [usize],
    out: &'a mut Choices,
    budget: &'a mut usize,
}

impl Walk<'_> {
    /// Returns false once enough choices are found (or the budget is spent), to stop the walk.
    fn go(&mut self, step: usize, pick: &mut Vec<usize>) -> bool {
        if step == self.order.len() {
            let choice: Vec<usize> = self.slot_var.iter().map(|&v| pick[v]).collect();
            if !self.out.contains(&choice) {
                self.out.push(choice);
            }
            return self.out.len() < MAX_BINDINGS;
        }
        let var = self.order[step];
        for &o in &self.domains[var] {
            if *self.budget == 0 {
                return false;
            }
            *self.budget -= 1;
            if self.order[..step].iter().any(|&v| pick[v] == o) {
                continue;
            }
            if let Some((n, m)) = self.near_of[var] {
                let (e, nn) = east_north(pos(&self.objects[pick[n]]), pos(&self.objects[o]));
                if e.hypot(nn) > m {
                    continue;
                }
            }
            pick[var] = o;
            if !self.go(step + 1, pick) {
                return false;
            }
        }
        true
    }
}

// ---- marching: standpoints that put a in front of b -------------------------------------

/// Standpoints (lat, lon) on the line from `b` through `a` and on beyond `a`, where `a` stands in
/// front of `b`, plus one either side at `MARCH_CONE_DEG`. Starts `MARCH_START_M` past a's near
/// side, steps grow with distance, and it stops once `a` would look smaller than
/// `MARCH_MIN_SIZE_DEG` (a's height over the distance). Empty if a and b are in the same place.
pub fn march(a: &Object, b: &Object) -> Vec<(f64, f64)> {
    let (e, n) = east_north(pos(b), pos(a));
    let len = e.hypot(n);
    if len < 1e-6 {
        return Vec::new();
    }
    let (ue, un) = (e / len, n / len);
    let far = (a.top_m - a.base_m).max(0.0) / MARCH_MIN_SIZE_DEG.to_radians().tan();
    let side = MARCH_CONE_DEG.to_radians().tan();
    let mut out = Vec::new();
    let mut s = a.radius_m + MARCH_START_M;
    while s <= far {
        for k in [0.0, -1.0, 1.0] {
            let w = k * s * side;
            out.push(offset(pos(a), ue * s - un * w, un * s + ue * w));
        }
        s = s * MARCH_GROWTH + MARCH_STEP_M;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{DsmMeta, DsmSource, NODATA};

    const LAT: f64 = 34.049;
    const LON: f64 = -118.251;

    /// An object `east`, `north` metres from the test centre.
    fn obj(id: &str, kind: &str, east: f64, north: f64, top_m: f64, radius_m: f64) -> Object {
        let (lat, lon) = offset((LAT, LON), east, north);
        Object { id: id.into(), kind: kind.into(), lat, lon, base_m: 0.0, top_m, radius_m, source: "test".into() }
    }

    fn view(east: f64, north: f64) -> View {
        let (lat, lon) = offset((LAT, LON), east, north);
        View { lat, lon, eye_m: 1.7, lens_deg: 60.0 }
    }

    /// A flat 1 m grid, `n` wide, centred on the test centre; `paint(east, north)` raises cells.
    fn grid(n: u32, paint: impl Fn(f64, f64) -> Option<f32>) -> DsmGrid {
        let half = f64::from(n) / 2.0;
        let mut data = Vec::new();
        for r in 0..n {
            for c in 0..n {
                data.push(paint(f64::from(c) + 0.5 - half, half - f64::from(r) - 0.5).unwrap_or(0.0));
            }
        }
        let source = DsmSource { provider: "synthetic".into(), collection: None, captured_from: None, captured_to: None, native_res_m: 1.0 };
        DsmGrid { meta: DsmMeta { center_lat: LAT, center_lon: LON, cell_m: 1.0, width: n, height: n, nodata: NODATA, source }, data }
    }

    fn near(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn geometry_from_a_standpoint() {
        let v = view(0.0, 0.0);
        let east = obj("e", "building", 100.0, 0.0, 51.7, 10.0);
        let north = obj("n", "tree", 0.0, 50.0, 10.0, 3.0);
        assert!(near(dist(&v, &east), 100.0, 0.01));
        assert!(near(bearing(&v, &east), 90.0, 0.01));
        assert!(near(bearing(&v, &north), 0.0, 0.01) || near(bearing(&v, &north), 360.0, 0.01));
        assert!(near(bearing_diff(&v, &east, &north), 90.0, 0.01), "east is right of north");
        assert!(near(bearing_diff(&v, &north, &east), -90.0, 0.01));
        // Top 50 m above the eye, 90 m to the near side: atan(50/90) up, atan(1.7/90) down.
        let want = ((50.0f64 / 90.0).atan() + (1.7f64 / 90.0).atan()).to_degrees();
        assert!(near(size_deg(&v, &east), want, 1e-9));
        assert_eq!(in_frame(&v, &east, &north), 0.0, "90 deg apart doesn't fit a 60 deg lens");
        let ne = obj("ne", "tree", 100.0, 100.0, 10.0, 3.0);
        assert!(near(in_frame(&v, &east, &ne), 0.5, 1e-6), "45 deg apart: halfway down the ramp");
        assert!(near(sun_bearing_diff(&v, 100.0, &east), 10.0, 0.01));
    }

    #[test]
    fn visible_sees_over_low_things_and_not_through_walls() {
        // Tower 40 m tall, 10 m radius, 200 m east.
        let tower = obj("t", "building", 200.0, 0.0, 40.0, 10.0);
        let open = grid(500, |e, n| ((e - 200.0).hypot(n) < 10.0).then_some(40.0));
        assert_eq!(visible(&open, &view(0.0, 0.0), &tower), 1.0, "nothing in the way; the tower doesn't hide itself");
        let walled = grid(500, |e, n| {
            if (e - 200.0).hypot(n) < 10.0 {
                Some(40.0)
            } else if (e - 100.0).abs() < 1.0 && n.abs() < 30.0 {
                Some(22.0)
            } else {
                None
            }
        });
        // A 22 m wall at 100 m east. Heights 4..36 m on the near side (190 m): the line to the top
        // one is at ~19.6 m when it reaches the wall's first cell (99 m), below 22 - 0.5. All hidden.
        assert_eq!(visible(&walled, &view(0.0, 0.0), &tower), 0.0);
        // From a 30 m roof-ish eye the wall is below: everything in view.
        // From a 31.7 m eye the wall's top is below, but the lines down to 4 and 12 m still hit it.
        let high = View { eye_m: 31.7, ..view(0.0, 0.0) };
        assert!(near(visible(&walled, &high, &tower), 0.6, 1e-9));
        // A square tower seen across its corner doesn't hide itself.
        let square = grid(500, |e, n| ((e - 150.0).abs() < 10.0 && (n - 150.0).abs() < 10.0).then_some(40.0));
        let sq = obj("sq", "building", 150.0, 150.0, 40.0, 10.0);
        assert_eq!(visible(&square, &view(0.0, 0.0), &sq), 1.0);
        // A 5 m wall halfway: the bottom sample (4 m) is hidden, the rest are seen.
        let low = grid(500, |e, n| {
            if (e - 200.0).hypot(n) < 10.0 {
                Some(40.0)
            } else {
                ((e - 95.0).abs() < 1.0 && n.abs() < 30.0).then_some(5.0)
            }
        });
        assert!(near(visible(&low, &view(0.0, 0.0), &tower), 0.8, 1e-9), "{}", visible(&low, &view(0.0, 0.0), &tower));
    }

    #[test]
    fn bind_by_id_kind_and_near() {
        let objs = vec![
            obj("costar:1", "building", 0.0, 100.0, 60.0, 20.0),
            obj("costar:2", "building", 0.0, -900.0, 60.0, 20.0),
            obj("tree:a", "tree", 50.0, 120.0, 10.0, 3.0),
            obj("tree:b", "Tree", 10.0, -700.0, 10.0, 3.0),
            obj("tree:far", "tree", 0.0, 5000.0, 10.0, 3.0),
        ];
        let r = |j: &str| -> ObjRef { serde_json::from_str(j).unwrap() };
        let c = (LAT, LON);
        assert_eq!(bind(&[], &objs, c, 300.0).unwrap(), vec![Vec::<usize>::new()]);
        assert_eq!(bind(&[r(r#"{"id":"costar:2"}"#)], &objs, c, 300.0).unwrap(), vec![vec![1]]);
        // Any tree in reach, nearest the centre first; not the one 5 km away; kind is case-insensitive.
        assert_eq!(bind(&[r(r#"{"kind":"tree"}"#)], &objs, c, 300.0).unwrap(), vec![vec![2], vec![3]]);
        // Trees near costar:1, and b is that same costar:1 because it's written the same.
        let a = r(r#"{"kind":"tree","near":{"id":"costar:1"},"within_m":100}"#);
        assert_eq!(bind(&[a.clone(), r(r#"{"id":"costar:1"}"#)], &objs, c, 300.0).unwrap(), vec![vec![2, 0]]);
        // A tree near any building, and b is that building: pairs within range.
        let a = r(r#"{"kind":"tree","near":{"kind":"building"}}"#);
        assert_eq!(bind(&[a, r(r#"{"kind":"building"}"#)], &objs, c, 300.0).unwrap(), vec![vec![2, 0], vec![3, 1]]);
        // b written after a still links: trees near costar:1, which is b.
        let a = r(r#"{"kind":"tree","near":{"id":"costar:1"}}"#);
        assert_eq!(bind(&[r(r#"{"id":"costar:1"}"#), a], &objs, c, 300.0).unwrap(), vec![vec![0, 2]]);
        // Two buildings are two different buildings, even when written the same.
        assert_eq!(bind(&[r(r#"{"kind":"building"}"#), r(r#"{"kind":"building"}"#)], &objs, c, 300.0).unwrap(), vec![vec![0, 1], vec![1, 0]]);
        let ab = [r(r#"{"kind":"building"}"#), r(r#"{"kind":"building","near":{"id":"costar:2"}}"#)];
        // "A building near costar:2" can't be costar:2 itself (it's named, so it's taken), and
        // costar:1 is 1 km away: nothing fits.
        assert!(bind(&ab, &objs, c, 300.0).is_err());
    }

    #[test]
    fn bind_errors_say_what_is_missing() {
        let objs = vec![obj("costar:1", "building", 0.0, 100.0, 60.0, 20.0), obj("t", "tree", 0.0, 400.0, 10.0, 3.0)];
        let r = |j: &str| -> ObjRef { serde_json::from_str(j).unwrap() };
        let c = (LAT, LON);
        let e = bind(&[r(r#"{"id":"costar:9"}"#)], &objs, c, 300.0).unwrap_err();
        assert!(e.contains("no object with id \"costar:9\""), "{e}");
        let e = bind(&[r(r#"{"kind":"fountain"}"#)], &objs, c, 300.0).unwrap_err();
        assert!(e.contains("no object of kind \"fountain\" within 1800 m") && e.contains("building x1, tree x1"), "{e}");
        let e = bind(&[r(r#"{"kind":"tree","within_m":200}"#)], &objs, c, 300.0).unwrap_err();
        assert!(e.contains("within 200 m"), "{e}");
        let e = bind(&[r(r#"{"kind":"tree","near":{"id":"costar:1"},"within_m":50}"#)], &objs, c, 300.0).unwrap_err();
        assert!(e.contains("no objects fit") && e.contains("within_m"), "{e}");
        let e = bind(&[r(r#"{"kind":"tree"}"#)], &[], c, 300.0).unwrap_err();
        assert!(e.contains("no objects here at all"), "{e}");
    }

    #[test]
    fn bind_is_capped() {
        let objs: Vec<Object> = (0..20).map(|i| obj(&format!("t{i:02}"), "tree", f64::from(i) * 10.0, 0.0, 10.0, 3.0)).collect();
        let r: ObjRef = serde_json::from_str(r#"{"kind":"tree"}"#).unwrap();
        let s: ObjRef = serde_json::from_str(r#"{"kind":"tree","within_m":1000}"#).unwrap();
        let c = bind(&[r, s], &objs, (LAT, LON), 300.0).unwrap();
        assert_eq!(c.len(), MAX_BINDINGS);
        assert!(c.iter().all(|p| p[0] != p[1]));
        assert_eq!(c[0], vec![0, 1], "nearest first");
    }

    #[test]
    fn march_runs_from_b_through_a_and_on() {
        // b 300 m north of a; a is 20 m tall, so it spans 1 deg at ~1146 m.
        let a = obj("a", "tree", 0.0, 0.0, 20.0, 5.0);
        let b = obj("b", "building", 0.0, 300.0, 100.0, 20.0);
        let pts = march(&a, &b);
        assert!(pts.len() % 3 == 0 && pts.len() >= 3 * 40 && pts.len() <= 3 * 70, "{}", pts.len());
        for (i, p) in pts.iter().enumerate() {
            let (e, n) = east_north((LAT, LON), *p);
            assert!(n < -9.9 && n > -1150.0, "south of a, so a is in front of b: {n}");
            let v = View { lat: p.0, lon: p.1, eye_m: 1.7, lens_deg: 60.0 };
            let off = bearing_diff(&v, &a, &b).abs();
            if i % 3 == 0 {
                assert!(e.abs() < 1e-6 && off < 1e-6);
            } else {
                assert!(off < MARCH_CONE_DEG + 1e-6, "{off}");
            }
        }
        assert!(march(&a, &a).is_empty());
    }
}
