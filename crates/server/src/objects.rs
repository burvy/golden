//! Objects in a search area that cost functions can be about (docs/metrics.md, "Relations between objects"):
//! CoStar buildings (where the kit covers) and Vexcel detections (open vocabulary, `vexcel::detect`), merged
//! into `contracts::Object`s with heights from the area's height map.
//!
//! - CoStar: id `costar:<id>`, at CoStar's point (the street address, often 50-100 m from the building's
//!   middle); top = ground + CoStar's height if it has one, else the highest surface within `BUILDING_REACH_M`.
//! - Vexcel: id `vexcel:<kind>:<lat>,<lon>`, at the centre of the detected box; top = highest surface inside it.
//! - The same thing found twice (two boxes, or CoStar and Vexcel) is kept once, CoStar first, then the more
//!   confident detection. At most `MAX_PER_KIND` per kind, nearest the area's centre.

use contracts::{Area, Building, DsmGrid, Object, OBJECT_KINDS};
use golden_core::search::skyline_need_m;
use vexcel::detect::Detection;

use crate::state::{ApiErr, AppState};

pub const MAX_PER_KIND: usize = 60;
/// At most this many kinds per call (each costs Vexcel work on a cache miss).
pub const MAX_KINDS: usize = 4;
/// Footprint radius we assume for a CoStar building, and how far from its point we look for its roof.
pub const BUILDING_RADIUS_M: f64 = 20.0;
pub const BUILDING_REACH_M: f64 = 40.0;
/// Ground at an object's foot: the lowest surface within its radius plus this.
const GROUND_MARGIN_M: f64 = 10.0;
/// Two buildings closer than their radii plus this are one (CoStar's point vs Vexcel's box centre).
const BUILDING_SAME_M: f64 = 15.0;

/// The objects, and what the agent should know about how they were found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub objects: Vec<Object>,
    pub notes: Vec<String>,
}

/// Pure: the kinds asked for, as `OBJECT_KINDS` spells them, or why not.
pub fn check_kinds(kinds: &[String]) -> Result<Vec<String>, String> {
    if kinds.is_empty() {
        return Err(format!("name at least one kind; kinds are {}", OBJECT_KINDS.join(", ")));
    }
    let mut out: Vec<String> = Vec::new();
    for k in kinds {
        let Some(known) = OBJECT_KINDS.iter().find(|o| o.eq_ignore_ascii_case(k.trim())) else {
            return Err(format!("unknown kind \"{k}\"; kinds are {} (adding one is a contracts change)", OBJECT_KINDS.join(", ")));
        };
        if !out.iter().any(|o| o == known) {
            out.push(known.to_string());
        }
    }
    if out.len() > MAX_KINDS {
        return Err(format!("at most {MAX_KINDS} kinds per call"));
    }
    Ok(out)
}

/// Objects of `kinds` in `area`. Vexcel answers come from the disk cache first; with no Vexcel key only cached
/// areas have detections. A Vexcel failure is a note, not an error (CoStar still answers).
pub async fn find_objects(st: &AppState, area: &Area, kinds: &[String]) -> Result<Found, ApiErr> {
    find_objects_with(st, area, kinds, vexcel::auth::ApiKey::from_env().ok()).await
}

/// `find_objects` with the Vexcel key passed in (`None` = cache only; tests use it so a key in the
/// environment can't make them call Vexcel).
pub async fn find_objects_with(st: &AppState, area: &Area, kinds: &[String], key: Option<vexcel::auth::ApiKey>) -> Result<Found, ApiErr> {
    let kinds = check_kinds(kinds).map_err(ApiErr::bad)?;
    // the same grid search uses, so it's usually already in memory
    let grid = st.grids.around(area.lat, area.lon, skyline_need_m(area.radius_m)).await?;
    let buildings = match st.kit {
        Some(kit) if kinds.iter().any(|k| k == "building") => kit.buildings_near(area.lat, area.lon, area.radius_m),
        _ => Vec::new(),
    };
    let mut notes = Vec::new();
    let found = match vexcel::detect::detect(&st.http, key.as_ref(), &st.cache_root, area.lat, area.lon, area.radius_m, &kinds).await {
        Ok(d) => {
            notes.extend(detect_notes(&d));
            d.found
        }
        Err(e) => {
            eprintln!("objects: Vexcel detection failed: {e}");
            notes.push("Vexcel detection failed; only CoStar buildings are listed".into());
            Vec::new()
        }
    };
    if kinds.iter().any(|k| k == "building") && st.kit.is_none() {
        notes.push("CoStar kit not loaded: buildings come from Vexcel detection only".into());
    }
    let objects = assemble(&grid, &buildings, &found, area);
    eprintln!(
        "objects: {} near {:.4},{:.4} r {:.0} m ({} CoStar, {} Vexcel before merging)",
        objects.len(),
        area.lat,
        area.lon,
        area.radius_m,
        buildings.len(),
        found.len()
    );
    Ok(Found { objects, notes })
}

/// Pure: what a detection run couldn't do, in words.
fn detect_notes(d: &vexcel::detect::Detected) -> Vec<String> {
    let mut n = Vec::new();
    if d.skipped_boxes > 0 {
        n.push(format!("Vexcel detection covered the {} boxes nearest the centre; {} further out were left out", d.boxes, d.skipped_boxes));
    }
    if d.missing > 0 && d.errors.is_empty() {
        n.push(format!("{} Vexcel detection answers aren't cached and there's no Vexcel key, so some objects may be missing", d.missing));
    }
    if !d.errors.is_empty() {
        n.push(format!("{} Vexcel detection call(s) failed; some objects may be missing", d.errors.len()));
        for e in &d.errors {
            eprintln!("objects: detect: {e}");
        }
    }
    n
}

/// Pure: CoStar buildings and detections as objects, merged, capped per kind, nearest the area centre first.
pub fn assemble(grid: &DsmGrid, buildings: &[Building], found: &[Detection], area: &Area) -> Vec<Object> {
    // (object, priority): CoStar first, then by confidence
    let mut all: Vec<(Object, f64)> = buildings.iter().filter_map(|b| from_building(grid, b).map(|o| (o, 2.0))).collect();
    all.extend(found.iter().filter_map(|d| from_detection(grid, d).map(|o| (o, d.confidence.unwrap_or(0.5)))));
    all.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut kept: Vec<Object> = Vec::new();
    for (o, _) in all {
        if !kept.iter().any(|k| same_thing(k, &o)) {
            kept.push(o);
        }
    }
    let dist = |o: &Object| metres(area.lat, area.lon, o.lat, o.lon);
    kept.sort_by(|a, b| a.kind.cmp(&b.kind).then(dist(a).total_cmp(&dist(b))));
    let mut out: Vec<Object> = Vec::new();
    for o in kept {
        if out.iter().filter(|x| x.kind == o.kind).count() < MAX_PER_KIND {
            out.push(o);
        }
    }
    out
}

fn same_thing(a: &Object, b: &Object) -> bool {
    if a.kind != b.kind {
        return false;
    }
    let slack = if a.kind == "building" { BUILDING_SAME_M } else { 0.0 };
    metres(a.lat, a.lon, b.lat, b.lon) <= a.radius_m.max(b.radius_m) + slack
}

fn from_building(grid: &DsmGrid, b: &Building) -> Option<Object> {
    let (base_m, dsm_top) = base_and_top(grid, b.lat, b.lon, BUILDING_REACH_M)?;
    let top_m = b.height_m.filter(|h| *h > 0.0).map_or(dsm_top, |h| base_m + h);
    Some(Object {
        id: b.id.clone(),
        kind: "building".into(),
        lat: b.lat,
        lon: b.lon,
        base_m: round1(base_m),
        top_m: round1(top_m),
        radius_m: BUILDING_RADIUS_M,
        source: "costar".into(),
    })
}

fn from_detection(grid: &DsmGrid, d: &Detection) -> Option<Object> {
    let radius_m = d.half_ew_m.max(d.half_ns_m).max(0.5);
    let (base_m, top_m) = base_and_top(grid, d.lat, d.lon, radius_m)?;
    let kind = OBJECT_KINDS.iter().find(|k| k.eq_ignore_ascii_case(&d.label))?.to_string();
    Some(Object {
        id: format!("vexcel:{}:{:.5},{:.5}", kind.replace(' ', "-"), d.lat, d.lon),
        kind,
        lat: d.lat,
        lon: d.lon,
        base_m: round1(base_m),
        top_m: round1(top_m),
        radius_m: round1(radius_m),
        source: "vexcel".into(),
    })
}

/// Pure: (ground at the foot, highest surface) of something `radius_m` across at (lat, lon), from the height
/// map. Ground is the lowest surface within `radius_m + GROUND_MARGIN_M`. None if the grid has nothing there.
pub fn base_and_top(grid: &DsmGrid, lat: f64, lon: f64, radius_m: f64) -> Option<(f64, f64)> {
    let (c, r) = grid.cell_of(lat, lon);
    let cell = grid.meta.cell_m;
    let reach = ((radius_m + GROUND_MARGIN_M) / cell).ceil() as i64;
    let (mut base, mut top) = (f64::INFINITY, f64::NEG_INFINITY);
    for dr in -reach..=reach {
        for dc in -reach..=reach {
            let (cc, rr) = (c.floor() as i64 + dc, r.floor() as i64 + dr);
            if cc < 0 || rr < 0 || cc >= i64::from(grid.meta.width) || rr >= i64::from(grid.meta.height) {
                continue;
            }
            let h = grid.data[(rr as usize) * grid.meta.width as usize + cc as usize];
            if h == grid.meta.nodata {
                continue;
            }
            let d = (dc as f64).hypot(dr as f64) * cell;
            let h = f64::from(h);
            if d <= radius_m + GROUND_MARGIN_M {
                base = base.min(h);
            }
            if d <= radius_m.max(cell) {
                top = top.max(h);
            }
        }
    }
    (base.is_finite() && top.is_finite()).then_some((base, top.max(base)))
}

fn metres(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    costar::distance_m(lat1, lon1, lat2, lon2)
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    const SAMPLE: &str = include_str!("../../../fixtures/vexcel_detect_sample.json");

    fn area(lat: f64, lon: f64, r: f64) -> Area {
        Area { lat, lon, radius_m: r }
    }

    fn building(id: &str, lat: f64, lon: f64, height_m: Option<f64>) -> Building {
        Building {
            id: id.into(),
            name: None,
            address: None,
            lat,
            lon,
            dist_m: None,
            height_m,
            floors: None,
            roof_m: None,
            property_type: None,
            contacts: vec![],
            matterport: vec![],
        }
    }

    /// A point `east`, `north` metres from the test grid's centre.
    fn at(east: f64, north: f64) -> (f64, f64) {
        let lat = testkit::LAT + north / 111_320.0;
        (lat, testkit::LON + east / (111_320.0 * testkit::LAT.to_radians().cos()))
    }

    #[test]
    fn kinds_are_checked_against_the_contract_list() {
        assert_eq!(check_kinds(&["Tree".into(), "tree".into(), " swimming pool".into()]).unwrap(), ["tree", "swimming pool"]);
        let e = check_kinds(&["fountain".into()]).unwrap_err();
        assert!(e.contains("unknown kind \"fountain\"") && e.contains("water tower"), "{e}");
        assert!(check_kinds(&[]).is_err());
    }

    #[test]
    fn heights_come_from_the_grid() {
        // the 45 m block 100-140 m east, 100-140 m north of the centre
        let g = testkit::city();
        let (lat, lon) = at(120.0, 120.0);
        let (base, top) = base_and_top(&g, lat, lon, 20.0).unwrap();
        assert_eq!((base, top), (0.0, 45.0));
        // a tree-sized thing on open ground: top = ground
        let (lat, lon) = at(0.0, 0.0);
        assert_eq!(base_and_top(&g, lat, lon, 3.0), Some((0.0, 0.0)));
        // off the grid
        assert_eq!(base_and_top(&g, 35.0, -118.25, 3.0), None);
    }

    #[test]
    fn costar_and_vexcel_merge_into_one_list() {
        let g = testkit::city();
        let (blat, blon) = at(115.0, 110.0);
        let (vlat, vlon) = at(125.0, 125.0); // Vexcel's box for the same 45 m block: merged into the CoStar one
        let (tlat, tlon) = at(0.0, 30.0);
        let found = vec![
            Detection { label: "building".into(), lat: vlat, lon: vlon, half_ew_m: 20.0, half_ns_m: 20.0, confidence: Some(0.9) },
            Detection { label: "tree".into(), lat: tlat, lon: tlon, half_ew_m: 3.0, half_ns_m: 3.0, confidence: Some(0.8) },
            // the same tree seen from the next box, less sure: dropped
            Detection { label: "tree".into(), lat: tlat + 1e-5, lon: tlon, half_ew_m: 3.0, half_ns_m: 3.0, confidence: Some(0.6) },
        ];
        let objs = assemble(&g, &[building("costar:7", blat, blon, Some(44.0))], &found, &area(testkit::LAT, testkit::LON, 300.0));
        assert_eq!(objs.len(), 2, "{objs:?}");
        let b = objs.iter().find(|o| o.kind == "building").unwrap();
        assert_eq!((b.id.as_str(), b.source.as_str(), b.base_m, b.top_m), ("costar:7", "costar", 0.0, 44.0));
        let t = objs.iter().find(|o| o.kind == "tree").unwrap();
        assert!(t.id.starts_with("vexcel:tree:") && t.source == "vexcel" && t.radius_m == 3.0, "{t:?}");
    }

    #[test]
    fn the_probe_sample_becomes_objects() {
        // The sample is downtown LA, inside the test grid (it's centred at 34.05, -118.25).
        let found = vexcel::detect::parse(SAMPLE.as_bytes()).unwrap();
        let objs = assemble(&testkit::city(), &[], &found, &area(34.0505, -118.2550, 300.0));
        assert_eq!(objs.len(), found.len());
        assert!(objs.iter().all(|o| o.source == "vexcel" && o.top_m >= o.base_m && contracts::OBJECT_KINDS.contains(&o.kind.as_str())));
        let ids: std::collections::HashSet<&str> = objs.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids.len(), objs.len(), "ids are unique");
    }

    #[test]
    fn each_kind_is_capped_nearest_first() {
        let g = testkit::city();
        let found: Vec<Detection> = (0..(MAX_PER_KIND + 10))
            .map(|i| {
                let (lat, lon) = at(10.0 * i as f64, -500.0);
                Detection { label: "car".into(), lat, lon, half_ew_m: 2.0, half_ns_m: 2.0, confidence: Some(0.7) }
            })
            .collect();
        let objs = assemble(&g, &[], &found, &area(testkit::LAT, testkit::LON, 800.0));
        assert_eq!(objs.len(), MAX_PER_KIND);
        let far = at(10.0 * (MAX_PER_KIND + 9) as f64, -500.0);
        assert!(objs.iter().all(|o| (o.lat, o.lon) != far), "the furthest went");
    }

    #[tokio::test]
    async fn finds_costar_buildings_and_cached_detections_offline() {
        // a one-building CoStar kit on disk, and the probe's trees cached for the box the point is in
        let dir = testkit::temp_dir("kit");
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let (blat, blon) = at(120.0, 120.0);
        std::fs::write(
            dir.join("data/commercial-property.jsonl"),
            format!("{{\"id\":7,\"primaryAddress\":{{\"latitude\":{blat},\"longitude\":{blon}}},\"buildingHeightInFeet\":150}}\n"),
        )
        .unwrap();
        let kit: &'static costar::Kit = Box::leak(Box::new(costar::Kit::load(&dir).unwrap()));
        let st = AppState { kit: Some(kit), ..testkit::state(testkit::city()) };
        // 500 m: the probe's trees (~420 m west of the grid centre) are in, and the grid still covers the skyline
        let a = area(testkit::LAT, testkit::LON, 500.0);
        let (bxs, _) = vexcel::detect::boxes(a.lat, a.lon, a.radius_m);
        let (_, trees) = vexcel::detect::split_by_label(SAMPLE.as_bytes(), &["tree".to_string()]).unwrap().remove(0);
        // cached for every box: the same tree "seen" in several boxes must come back once
        for &b in &bxs {
            let p = vexcel::detect::cache_path(&st.cache_root, b, "tree");
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, &trees).unwrap();
        }
        let got = find_objects_with(&st, &a, &["building".into(), "Tree".into()], None).await.unwrap();
        let b = got.objects.iter().find(|o| o.id == "costar:7").expect("the CoStar building");
        assert_eq!((b.base_m, b.top_m), (0.0, 45.7), "150 ft above the ground");
        let trees: Vec<&Object> = got.objects.iter().filter(|o| o.kind == "tree").collect();
        assert_eq!(trees.len(), 2, "the sample's 2 trees, once each though every box has them: {trees:?}");
        assert!(trees.iter().all(|t| t.source == "vexcel"));
        // no key, and the building boxes aren't cached: said, not failed
        assert!(got.notes.iter().any(|n| n.contains("aren't cached")), "{:?}", got.notes);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&st.cache_root);
    }
}
