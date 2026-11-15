//! Objects in the aerial photos: Vexcel's open-vocabulary detection, `POST /v2/elements/detect`
//! (docs/burvy/detect-probe.md). Each label is a plain-English prompt ("tree", "swimming pool"); each hit is a
//! lon/lat box, whose centre we use as the object's position.
//!
//! Only the search area is asked about, in `BOX_M` boxes on a fixed lattice (so the same box is asked about,
//! and cached, whichever search it falls in), at most `MAX_BOXES` per call. Each answer is split by label and
//! cached on disk as `<root>/vexcel/detect/<hash of box + label>.json`, a FeatureCollection holding only that
//! label's features, so asking for "tree" after "tree, car" costs nothing. With no key, cached answers still
//! serve and the rest is reported missing.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::auth::ApiKey;
use crate::client::post_json;
use crate::Error;

pub const DETECT_URL: &str = "https://api.vexcelgroup.com/v2/elements/detect";
/// Box side, metres. The probe used 150 m (~3 s per call); 300 m covers a 300 m search in 4-9 boxes.
pub const BOX_M: f64 = 300.0;
/// The most boxes one call asks about (nearest the centre first), so a big area can't fetch a city.
pub const MAX_BOXES: usize = 9;
/// Hits below this confidence are left out (Vexcel's `detection-threshold`).
pub const THRESHOLD: f64 = 0.5;
/// Boxes asked about at once.
const PARALLEL: usize = 3;
const M_PER_DEG: f64 = 111_320.0;

/// One detected object.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub label: String,
    /// Centre of the returned box.
    pub lat: f64,
    pub lon: f64,
    /// Half the box's east-west and north-south size, metres.
    pub half_ew_m: f64,
    pub half_ns_m: f64,
    /// Best confidence among the frames it was seen in, if Vexcel gave one.
    pub confidence: Option<f64>,
}

/// A lattice box: its centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectBox {
    pub lat: f64,
    pub lon: f64,
}

/// What `detect` found, and what it couldn't get.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Detected {
    pub found: Vec<Detection>,
    /// Boxes looked at, and boxes left out because the area needed more than `MAX_BOXES`.
    pub boxes: usize,
    pub skipped_boxes: usize,
    /// Vexcel calls made (0 when everything was cached).
    pub calls: usize,
    /// (box, label) answers neither cached nor fetchable (no key).
    pub missing: usize,
    /// Calls that failed, one line each (no key, no URL with secrets).
    pub errors: Vec<String>,
}

/// The lattice boxes that touch the circle, nearest first, at most `MAX_BOXES`; and how many were left out.
pub fn boxes(lat: f64, lon: f64, radius_m: f64) -> (Vec<DetectBox>, usize) {
    let dlat = BOX_M / M_PER_DEG;
    let reach = (radius_m + BOX_M) / M_PER_DEG;
    let (r0, r1) = (((lat - reach) / dlat).floor() as i64, ((lat + reach) / dlat).ceil() as i64);
    let mut out: Vec<(f64, DetectBox)> = Vec::new();
    for row in r0..=r1 {
        let blat = row as f64 * dlat;
        let dlon = dlat / blat.to_radians().cos();
        let reach_lon = reach / blat.to_radians().cos();
        let (c0, c1) = (((lon - reach_lon) / dlon).floor() as i64, ((lon + reach_lon) / dlon).ceil() as i64);
        for col in c0..=c1 {
            let b = DetectBox { lat: blat, lon: col as f64 * dlon };
            // distance from the point to the box (0 inside), with the box as a square of BOX_M
            let north = ((lat - b.lat).abs() * M_PER_DEG - BOX_M / 2.0).max(0.0);
            let east = ((lon - b.lon).abs() * M_PER_DEG * lat.to_radians().cos() - BOX_M / 2.0).max(0.0);
            if north.hypot(east) <= radius_m {
                let centre = ((lat - b.lat) * M_PER_DEG).hypot((lon - b.lon) * M_PER_DEG * lat.to_radians().cos());
                out.push((centre, b));
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    let skipped = out.len().saturating_sub(MAX_BOXES);
    (out.into_iter().take(MAX_BOXES).map(|(_, b)| b).collect(), skipped)
}

/// The request body for one box, as in the probe: detection on the nadir `urban` frame over the box.
pub fn request_body(b: DetectBox, labels: &[String]) -> Value {
    let wkt = format!("POINT ({:.7} {:.7})", b.lon, b.lat);
    let side = BOX_M as u32;
    let elements: Vec<Value> = labels.iter().map(|l| json!({"label": l, "detection-threshold": THRESHOLD})).collect();
    json!({
        "wkt": wkt,
        "bbox-dimensions": [side, side],
        "images": [{"endpoint": "oriented/extract", "request": {"layer": "urban", "wkt": wkt, "bbox-dimensions": format!("{side},{side}")}}],
        "model-input": {"elements": elements, "include": ["geometry", "confidence"]}
    })
}

/// The detections in a response (or a cached per-label slice of one). Features that aren't a polygon are skipped.
pub fn parse(body: &[u8]) -> Result<Vec<Detection>, Error> {
    let v: Value = serde_json::from_slice(body)?;
    let features = v["features"].as_array().ok_or_else(|| Error::BadData("detect: no \"features\" list".into()))?;
    Ok(features.iter().filter_map(detection).collect())
}

fn detection(f: &Value) -> Option<Detection> {
    let label = f["properties"]["label"].as_str()?.trim().to_string();
    let ring = f["geometry"]["coordinates"].get(0)?.as_array()?;
    let pts: Vec<(f64, f64)> = ring.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect();
    if pts.len() < 3 {
        return None;
    }
    let (w, e) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
    let (s, n) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    let lat = (s + n) / 2.0;
    let confidence = f["properties"]["detections"]
        .as_array()
        .and_then(|ds| ds.iter().filter_map(|d| d["confidence"].as_f64()).reduce(f64::max));
    Some(Detection {
        label,
        lat,
        lon: (w + e) / 2.0,
        half_ew_m: (e - w) / 2.0 * M_PER_DEG * lat.to_radians().cos(),
        half_ns_m: (n - s) / 2.0 * M_PER_DEG,
        confidence,
    })
}

/// A response split by label: one FeatureCollection per asked label (empty if it had no hits).
pub fn split_by_label(body: &[u8], labels: &[String]) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let v: Value = serde_json::from_slice(body)?;
    let features = v["features"].as_array().ok_or_else(|| Error::BadData("detect: no \"features\" list".into()))?;
    labels
        .iter()
        .map(|l| {
            let mine: Vec<&Value> =
                features.iter().filter(|f| f["properties"]["label"].as_str().is_some_and(|x| x.trim().eq_ignore_ascii_case(l))).collect();
            Ok((l.clone(), serde_json::to_vec(&json!({"type": "FeatureCollection", "features": mine}))?))
        })
        .collect()
}

/// `<root>/vexcel/detect/<hash>.json` for one box and label.
pub fn cache_path(root: &Path, b: DetectBox, label: &str) -> PathBuf {
    let key = format!("{:.6},{:.6},{BOX_M},{THRESHOLD},{}", b.lat, b.lon, label.trim().to_lowercase());
    root.join("vexcel").join("detect").join(format!("{:016x}.json", fnv1a(key.as_bytes())))
}

/// Objects with these labels within `radius_m` of (lat, lon): cached answers first, the rest from Vexcel
/// (`key` None = cache only). A failed box is reported in `errors`; the others still count.
pub async fn detect(
    http: &reqwest::Client,
    key: Option<&ApiKey>,
    root: &Path,
    lat: f64,
    lon: f64,
    radius_m: f64,
    labels: &[String],
) -> Result<Detected, Error> {
    let (bxs, skipped) = boxes(lat, lon, radius_m);
    let mut out = Detected { boxes: bxs.len(), skipped_boxes: skipped, ..Detected::default() };
    let mut to_fetch: Vec<(DetectBox, Vec<String>)> = Vec::new();
    for &b in &bxs {
        let mut need = Vec::new();
        for l in labels {
            match std::fs::read(cache_path(root, b, l)) {
                Ok(bytes) => out.found.extend(parse(&bytes)?.into_iter().filter(|d| in_circle(d, lat, lon, radius_m))),
                Err(_) => need.push(l.clone()),
            }
        }
        if !need.is_empty() {
            to_fetch.push((b, need));
        }
    }
    let Some(key) = key else {
        out.missing = to_fetch.iter().map(|(_, n)| n.len()).sum();
        return Ok(out);
    };
    for chunk in to_fetch.chunks(PARALLEL) {
        let mut calls = tokio::task::JoinSet::new();
        for (b, need) in chunk.iter().cloned() {
            let (http, key, root) = (http.clone(), key.clone(), root.to_path_buf());
            calls.spawn(async move { (fetch_box(&http, &key, &root, b, &need).await, need.len()) });
        }
        while let Some(done) = calls.join_next().await {
            out.calls += 1;
            match done.map_err(|e| Error::BadData(format!("detect task: {e}")))? {
                (Ok(found), _) => out.found.extend(found.into_iter().filter(|d| in_circle(d, lat, lon, radius_m))),
                (Err(e), n) => {
                    out.missing += n;
                    out.errors.push(e.to_string());
                }
            }
        }
    }
    Ok(out)
}

/// One Vexcel call for one box; each label's slice is cached before the detections come back.
async fn fetch_box(http: &reqwest::Client, key: &ApiKey, root: &Path, b: DetectBox, labels: &[String]) -> Result<Vec<Detection>, Error> {
    let body = post_json(http, key, DETECT_URL, &request_body(b, labels)).await?;
    let mut found = Vec::new();
    for (label, slice) in split_by_label(&body, labels)? {
        let path = cache_path(root, b, &label);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, &slice)?;
        std::fs::rename(&tmp, &path)?;
        found.extend(parse(&slice)?);
    }
    Ok(found)
}

/// Boxes are bigger than the circle; keep what's inside it.
fn in_circle(d: &Detection, lat: f64, lon: f64, radius_m: f64) -> bool {
    ((d.lat - lat) * M_PER_DEG).hypot((d.lon - lon) * M_PER_DEG * lat.to_radians().cos()) <= radius_m
}

fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../../../fixtures/vexcel_detect_sample.json");

    fn labels(ls: &[&str]) -> Vec<String> {
        ls.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_probe_sample() {
        let found = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(found.len(), 5);
        let tree = &found[0];
        assert_eq!(tree.label, "tree");
        assert!((tree.lat - 34.050472).abs() < 1e-5 && (tree.lon - -118.254544).abs() < 1e-5, "{tree:?}");
        assert!((tree.half_ew_m - 2.7).abs() < 0.3 && (tree.half_ns_m - 2.9).abs() < 0.3, "{tree:?}");
        assert_eq!(tree.confidence, Some(0.8359375));
        let mut kinds: Vec<&str> = found.iter().map(|d| d.label.as_str()).collect();
        kinds.dedup();
        assert!(kinds.contains(&"building") && kinds.contains(&"car"));
    }

    #[test]
    fn bad_bodies_are_errors_not_panics() {
        assert!(parse(b"not json").is_err());
        assert!(parse(b"{\"type\":\"FeatureCollection\"}").is_err());
        let odd = br#"{"features":[{"properties":{"label":"tree"},"geometry":{"type":"Point","coordinates":[1,2]}}]}"#;
        assert_eq!(parse(odd).unwrap(), vec![]);
    }

    #[test]
    fn boxes_cover_the_area_nearest_first_and_are_capped() {
        let (b, skipped) = boxes(34.05, -118.25, 100.0);
        assert!((1..=4).contains(&b.len()) && skipped == 0, "{b:?}");
        let (b, skipped) = boxes(34.05, -118.25, 2000.0);
        assert_eq!(b.len(), MAX_BOXES);
        assert!(skipped > 50);
        // fixed lattice: a nearby centre asks about the same boxes, so the cache is shared
        let (a, _) = boxes(34.0501, -118.2501, 100.0);
        let (c, _) = boxes(34.05, -118.25, 100.0);
        assert!(a.iter().all(|x| c.iter().any(|y| (x.lat - y.lat).abs() < 1e-9 && (x.lon - y.lon).abs() < 1e-9)));
    }

    #[test]
    fn request_names_each_label_and_the_box() {
        let v = request_body(DetectBox { lat: 34.05, lon: -118.25 }, &labels(&["tree", "swimming pool"]));
        assert_eq!(v["wkt"], "POINT (-118.2500000 34.0500000)");
        assert_eq!(v["bbox-dimensions"], json!([300, 300]));
        assert_eq!(v["model-input"]["elements"][1]["label"], "swimming pool");
        assert_eq!(v["images"][0]["request"]["bbox-dimensions"], "300,300");
    }

    #[test]
    fn split_keeps_one_label_per_slice_and_empty_ones() {
        let parts = split_by_label(SAMPLE.as_bytes(), &labels(&["tree", "car", "water tower"])).unwrap();
        let n: Vec<usize> = parts.iter().map(|(_, b)| parse(b).unwrap().len()).collect();
        assert_eq!(parts.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["tree", "car", "water tower"]);
        assert_eq!(n.iter().sum::<usize>(), 4, "tree + car of the 5 (the building isn't asked for)");
        assert_eq!(n[2], 0);
    }

    #[tokio::test]
    async fn cache_only_serves_what_is_cached_and_counts_the_rest() {
        let root = std::env::temp_dir().join(format!("golden-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (lat, lon) = (34.0505, -118.2550);
        let (bxs, _) = boxes(lat, lon, 150.0);
        // the sample's trees, cached for the box the point is in
        let first = bxs[0];
        let path = cache_path(&root, first, "tree");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let (_, trees) = split_by_label(SAMPLE.as_bytes(), &labels(&["tree"])).unwrap().remove(0);
        std::fs::write(&path, trees).unwrap();

        let http = reqwest::Client::new();
        let got = detect(&http, None, &root, lat, lon, 150.0, &labels(&["tree", "car"])).await.unwrap();
        assert_eq!(got.calls, 0);
        assert!(!got.found.is_empty() && got.found.iter().all(|d| d.label == "tree"), "{got:?}");
        assert_eq!(got.missing, 2 * bxs.len() - 1, "every (box, label) but the cached one");
        let _ = std::fs::remove_dir_all(&root);
    }
}
