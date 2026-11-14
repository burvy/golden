//! Vexcel's textured 3D city mesh (`/v2/3d/...`, OGC 3D Tiles: tileset JSON + glTF tiles with Draco geometry and
//! KTX2 textures). We pass it through to the browser so the key never leaves the server, and keep every response on
//! disk (`<cache>/vexcel/mesh/`), since the same tiles are asked for again and again.
//!
//! Coverage for our key (checked 11/14): LA (Downey 2020-21) and Houston 2023. `covering` finds the collection at a
//! point from `/v2/3d/collections`.

use std::path::{Path, PathBuf};

use crate::auth::ApiKey;
use crate::client::get_bytes;
use crate::Error;

pub const MESH_URL: &str = "https://api.vexcelgroup.com/v2/3d/";

/// What the browser may ask for under `/api/mesh/`: tileset JSON, a glTF tile, or the coverage list.
pub fn allowed(kind: &str) -> bool {
    matches!(kind, "tileset" | "tile" | "collections")
}

/// Tilesets point at their children with absolute Vexcel URLs at the top level; send those to our proxy instead.
/// Relative ones (`./tile?...`) already resolve against `/api/mesh/...`. Viewers pick a loader by file extension and
/// Vexcel's URLs have none, so children become `tileset.json?...` and `tile.glb?...` (the proxy strips it again).
pub fn rewrite_json(body: &[u8], proxy_prefix: &str) -> Vec<u8> {
    String::from_utf8_lossy(body)
        .replace(MESH_URL, proxy_prefix)
        .replace("tileset?", "tileset.json?")
        .replace("tile?", "tile.glb?")
        .into_bytes()
}

/// `tileset.json` -> `tileset`, `tile.glb` -> `tile`.
pub fn strip_ext(kind: &str) -> &str {
    kind.trim_end_matches(".json").trim_end_matches(".glb")
}

/// `<root>/vexcel/mesh/<kind>/<hash of the query>.bin`
pub fn cache_path(root: &Path, kind: &str, query: &str) -> PathBuf {
    root.join("vexcel").join("mesh").join(kind).join(format!("{:016x}.bin", fnv1a(query.as_bytes())))
}

/// One mesh response, from the disk cache or Vexcel (with backoff on 429/5xx). JSON comes back rewritten.
pub async fn get(http: &reqwest::Client, key: &ApiKey, root: &Path, kind: &str, query: &str, proxy_prefix: &str) -> Result<Vec<u8>, Error> {
    if !allowed(kind) {
        return Err(Error::BadData(format!("not a mesh resource: {kind}")));
    }
    let path = cache_path(root, kind, query);
    let raw = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            let url = if query.is_empty() { format!("{MESH_URL}{kind}") } else { format!("{MESH_URL}{kind}?{query}") };
            let b = get_bytes(http, key, &url, &[]).await?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("part");
            std::fs::write(&tmp, &b)?;
            std::fs::rename(&tmp, &path)?;
            b
        }
    };
    Ok(if kind == "tile" { raw } else { rewrite_json(&raw, proxy_prefix) })
}

/// Keep only the parts of a tileset within `radius_m` of (lat, lon). Standing in a city, the view takes in the
/// whole basin; without this the viewer loads coarse tiles for all of it before refining near the camera.
/// Regions are 3D Tiles `[west, south, east, north, minh, maxh]` in radians. Nodes without a region are kept.
pub fn trim_tileset(body: &[u8], lat: f64, lon: f64, radius_m: f64) -> Vec<u8> {
    let Ok(mut j) = serde_json::from_slice::<serde_json::Value>(body) else { return body.to_vec() };
    if let Some(root) = j.get_mut("root") {
        trim_node(root, lat, lon, radius_m);
    }
    serde_json::to_vec(&j).unwrap_or_else(|_| body.to_vec())
}

fn trim_node(n: &mut serde_json::Value, lat: f64, lon: f64, r: f64) {
    if let Some(kids) = n.get_mut("children").and_then(|c| c.as_array_mut()) {
        kids.retain(|k| region_dist_m(k, lat, lon).is_none_or(|d| d <= r));
        for k in kids.iter_mut() {
            trim_node(k, lat, lon, r);
        }
    }
}

/// Ground distance (m) from (lat, lon) to a node's bounding region, 0 inside; `None` without a region.
fn region_dist_m(n: &serde_json::Value, lat: f64, lon: f64) -> Option<f64> {
    let r = n["boundingVolume"]["region"].as_array()?;
    let g = |i: usize| r.get(i).and_then(|v| v.as_f64()).map(f64::to_degrees);
    let (w, s, e, no) = (g(0)?, g(1)?, g(2)?, g(3)?);
    let dlat = if lat < s { s - lat } else if lat > no { lat - no } else { 0.0 };
    let dlon = if lon < w { w - lon } else if lon > e { lon - e } else { 0.0 };
    Some((dlat * 111_320.0).hypot(dlon * 111_320.0 * lat.to_radians().cos()))
}

/// The mesh collection whose footprint contains (lat, lon), from the `/v2/3d/collections` GeoJSON.
pub fn covering(collections: &serde_json::Value, lat: f64, lon: f64) -> Option<String> {
    collections["features"].as_array()?.iter().find_map(|f| {
        let name = f["properties"]["collection"].as_str()?;
        let g = &f["geometry"];
        let polys: Vec<&serde_json::Value> = match g["type"].as_str()? {
            "Polygon" => vec![&g["coordinates"]],
            "MultiPolygon" => g["coordinates"].as_array()?.iter().collect(),
            _ => return None,
        };
        polys.iter().any(|p| p[0].as_array().is_some_and(|ring| inside(ring, lon, lat))).then(|| name.to_string())
    })
}

fn inside(ring: &[serde_json::Value], x: f64, y: f64) -> bool {
    let pts: Vec<(f64, f64)> = ring.iter().filter_map(|p| Some((p[0].as_f64()?, p[1].as_f64()?))).collect();
    let mut c = false;
    for i in 0..pts.len() {
        let (x1, y1) = pts[i];
        let (x2, y2) = pts[(i + pts.len() - 1) % pts.len()];
        if (y1 > y) != (y2 > y) && x < (x2 - x1) * (y - y1) / (y2 - y1) + x1 {
            c = !c;
        }
    }
    c
}

fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_child_urls_go_through_the_proxy() {
        let j = br#"{"uri":"https://api.vexcelgroup.com/v2/3d/tileset?collection=x&layer=3d-cities","b":"./tile?zoom=14"}"#;
        let out = String::from_utf8(rewrite_json(j, "/api/mesh/")).unwrap();
        assert_eq!(out, r#"{"uri":"/api/mesh/tileset.json?collection=x&layer=3d-cities","b":"./tile.glb?zoom=14"}"#);
        assert_eq!(strip_ext("tileset.json"), "tileset");
        assert_eq!(strip_ext("tile.glb"), "tile");
    }

    #[test]
    fn trims_far_children_and_keeps_near_ones() {
        let reg = |w: f64, so: f64, e: f64, n: f64| serde_json::json!({"region": [w.to_radians(), so.to_radians(), e.to_radians(), n.to_radians(), 0, 100]});
        let t = serde_json::json!({"root": {"children": [
            {"boundingVolume": reg(-118.26, 34.04, -118.24, 34.06), "children": [
                {"boundingVolume": reg(-118.26, 34.04, -118.255, 34.045)},
                {"boundingVolume": reg(-118.2505, 34.0495, -118.2495, 34.0505)}]},
            {"boundingVolume": reg(-118.0, 33.8, -117.9, 33.9)},
            {"content": {"uri": "./x"}}
        ]}});
        let out: serde_json::Value = serde_json::from_slice(&trim_tileset(&serde_json::to_vec(&t).unwrap(), 34.05, -118.25, 300.0)).unwrap();
        let kids = out["root"]["children"].as_array().unwrap();
        assert_eq!(kids.len(), 2); // the far one went; the one without a region stays
        assert_eq!(kids[0]["children"].as_array().unwrap().len(), 1); // only the near grandchild
    }

    #[test]
    fn only_mesh_resources() {
        assert!(allowed("tileset") && allowed("tile") && allowed("collections"));
        assert!(!allowed("../dsm/extract") && !allowed("oriented"));
    }

    #[test]
    fn finds_the_collection_at_a_point() {
        let fc = serde_json::json!({"features": [
            {"properties": {"collection": "la"}, "geometry": {"type": "MultiPolygon",
                "coordinates": [[[[-119.0, 33.0], [-117.0, 33.0], [-117.0, 35.0], [-119.0, 35.0], [-119.0, 33.0]]]]}},
            {"properties": {"collection": "hou"}, "geometry": {"type": "Polygon",
                "coordinates": [[[-96.0, 29.0], [-95.0, 29.0], [-95.0, 30.0], [-96.0, 30.0], [-96.0, 29.0]]]}}
        ]});
        assert_eq!(covering(&fc, 34.05, -118.25).as_deref(), Some("la"));
        assert_eq!(covering(&fc, 29.76, -95.37).as_deref(), Some("hou"));
        assert_eq!(covering(&fc, 40.35, -74.66), None);
    }

    #[test]
    fn same_query_same_file() {
        let r = Path::new("c");
        assert_eq!(cache_path(r, "tile", "a=1"), cache_path(r, "tile", "a=1"));
        assert_ne!(cache_path(r, "tile", "a=1"), cache_path(r, "tile", "a=2"));
    }
}
