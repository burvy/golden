//! Every fixture in `fixtures/` must parse into its contract struct, with no unknown fields,
//! and survive a JSON round trip. If this fails, the fixture and docs/contracts.md disagree.

use std::path::PathBuf;

use contracts::scene::{decode_dm, SCENE_ENCODING};
use contracts::*;
use serde::de::DeserializeOwned;
use serde::Serialize;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
}

fn load<T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug>(name: &str) -> T {
    let text = std::fs::read_to_string(fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let v: T = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} does not match its contract: {e}"));
    let back: T = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
    assert_eq!(back, v, "{name} changes on a round trip");
    v
}

#[test]
fn sun_path() {
    let s: SunPath = load("sun_path.json");
    assert_eq!(s.sunset.unwrap().t, "2026-06-21T20:07:41-07:00");
    assert!(s.points.windows(2).all(|w| w[0].t < w[1].t), "points in time order");
}

#[test]
fn horizon_profile() {
    let h: HorizonProfile = load("horizon_profile.json");
    assert_eq!(h.points.len(), (360.0 / h.step_deg) as usize);
    assert!((h.observer.eye_m - h.observer.ground_m - 1.7).abs() < 1e-9);
    assert!(h.points.iter().any(|p| p.dist_m.is_none() && p.elev_deg == 0.0));
    assert!(h.sun.is_some());
}

#[test]
fn horizon_without_date_omits_sun() {
    let mut h: HorizonProfile = load("horizon_profile.json");
    h.sun = None;
    assert!(!serde_json::to_string(&h).unwrap().contains("\"sun\""));
}

#[test]
fn brief_and_defaults() {
    let b: Brief = load("brief.json");
    assert_eq!((b.standing, b.framing, b.event), (Standing::Roof, Framing::Between, Event::Sunset));
    let minimal: Brief =
        serde_json::from_str(r#"{"area":{"lat":34.049,"lon":-118.251,"radius_m":300},"date":"2026-06-24"}"#).unwrap();
    assert_eq!(minimal.tz, "America/Los_Angeles");
    assert_eq!((minimal.standing, minimal.framing, minimal.max_spots), (Standing::Any, Framing::Clear, 10));
}

#[test]
fn brief_rejects_typos() {
    let typo = r#"{"area":{"lat":34,"lon":-118,"radius_m":300},"date":"2026-06-24","framming":"between"}"#;
    assert!(serde_json::from_str::<Brief>(typo).is_err());
}

#[test]
fn search_result() {
    let r: SearchResult = load("search_result.json");
    assert!(r.spots.windows(2).all(|w| w[0].rank < w[1].rank && w[0].score >= w[1].score));
    assert!(r.spots.iter().all(|s| s.standing != Standing::Any));
    assert_eq!(r.heat.values.len(), (r.heat.width * r.heat.height) as usize);
    assert!(r.heat.width <= search::MAX_HEAT_SIDE);
}

#[test]
fn scene_meta_and_binary_agree() {
    let m: SceneMeta = load("scene_meta.json");
    assert_eq!(m.encoding, SCENE_ENCODING);
    let bin = std::fs::read(fixture("scene.bin")).unwrap();
    assert_eq!(bin.len() as u64, m.bytes);
    let mut next = 0;
    for r in &m.rings {
        assert_eq!(r.byte_offset, next, "rings are back to back");
        next += r.byte_len();
    }
    assert_eq!(next, m.bytes);
    let first = u16::from_le_bytes([bin[0], bin[1]]);
    assert_eq!(decode_dm(first, m.base_m), Some(90.0));
}

#[test]
fn photo_meta() {
    let p: PhotoMeta = load("photo_meta.json");
    assert_eq!(p.kind, PhotoKind::Oblique);
    assert!(p.width <= photo::MAX_PHOTO_PX && p.height <= photo::MAX_PHOTO_PX);
}

#[test]
fn buildings() {
    let b: BuildingList = load("buildings.json");
    assert!(b.buildings.windows(2).all(|w| w[0].dist_m <= w[1].dist_m), "nearest first");
    assert!(b.buildings.iter().all(|x| x.id.starts_with("costar:")));
}

#[test]
fn error() {
    let e: ApiError = load("error.json");
    assert!(!e.error.is_empty());
}
