//! Ortho (straight-down aerial) map tiles on the standard XYZ / Web Mercator grid, for the map view.
//!
//! What the live API does (measured 11/14/26, see docs/Nube-Lube/DEVLOG.md):
//! - `GET /v2/ortho/tile?layer=..&zoom=z&tile-x=x&tile-y=y` returns a 256 px tile (JPEG, or PNG where
//!   part of it has no imagery, so the edge of coverage is transparent).
//! - Our hackathon key is geofenced to greater LA, greater Houston and (since 11/14) Princeton / Trenton
//!   (`n40w74-us-nj-trenton-2026`). Everywhere else is HTTP 403 (NYC, Chicago, San Diego, London...).
//!   Outside coverage at low zoom the tile is black/transparent.
//! So Vexcel can only be an overlay on top of a global basemap. "No tile" answers are cached too, so
//! panning outside coverage doesn't hit Vexcel again and again.

use std::path::{Path, PathBuf};

use crate::auth::ApiKey;
use crate::{cache, client, Error};

pub const ORTHO_TILE_URL: &str = "https://api.vexcelgroup.com/v2/ortho/tile";
/// Deepest zoom we ask for (Vexcel urban imagery is ~7.5 cm, i.e. about z21).
pub const MAX_ZOOM: u8 = 21;

#[derive(Debug, Clone, PartialEq)]
pub enum OrthoTile {
    Image { bytes: Vec<u8>, content_type: &'static str },
    /// No imagery we may use here (outside the key's area, or none flown).
    Empty,
}

/// Fetches and caches tiles. Make one and share it (it holds a connection pool).
#[derive(Debug, Clone)]
pub struct OrthoTiles {
    http: reqwest::Client,
    /// None: serve only what's cached; everything else is Empty.
    key: Option<ApiKey>,
    cache_root: PathBuf,
}

impl OrthoTiles {
    /// Key from the environment if there is one; without it, cached tiles still work.
    pub fn from_env() -> Result<OrthoTiles, Error> {
        Ok(OrthoTiles { http: client::http()?, key: ApiKey::from_env().ok(), cache_root: cache::default_root() })
    }

    pub fn new(key: Option<ApiKey>, cache_root: PathBuf) -> Result<OrthoTiles, Error> {
        Ok(OrthoTiles { http: client::http()?, key, cache_root })
    }

    pub fn has_key(&self) -> bool {
        self.key.is_some()
    }

    pub async fn get(&self, z: u8, x: u32, y: u32) -> Result<OrthoTile, Error> {
        check_xyz(z, x, y)?;
        let base = tile_base(&self.cache_root, z, x, y);
        if let Some(t) = read_cached(&base).await {
            return Ok(t);
        }
        let Some(key) = &self.key else { return Ok(OrthoTile::Empty) };
        let (zs, xs, ys) = (z.to_string(), x.to_string(), y.to_string());
        let query = [
            ("layer", "urban,wide-area"),
            ("zoom", zs.as_str()),
            ("tile-x", xs.as_str()),
            ("tile-y", ys.as_str()),
            ("order-by", "collection-last-capture-date-desc"),
        ];
        match client::get_bytes(&self.http, key, ORTHO_TILE_URL, &query).await {
            Ok(bytes) if bytes.is_empty() => Ok(OrthoTile::Empty), // 204 No Content
            Ok(bytes) => match content_type(&bytes) {
                Some(ct) => {
                    cache::write(&base.with_extension(ext(ct)), &bytes).await?;
                    Ok(OrthoTile::Image { bytes, content_type: ct })
                }
                None => Err(Error::BadData("Vexcel tile is neither JPEG nor PNG".into())),
            },
            // 403: outside the key's area. 404 / 204: nothing flown here. Both mean "show the basemap".
            Err(Error::Api { status: 403 | 404 | 204, .. }) => {
                cache::write(&base.with_extension("none"), b"").await?;
                Ok(OrthoTile::Empty)
            }
            Err(e) => Err(e),
        }
    }
}

pub fn check_xyz(z: u8, x: u32, y: u32) -> Result<(), Error> {
    let n = 1u64 << z.min(MAX_ZOOM);
    if z > MAX_ZOOM || u64::from(x) >= n || u64::from(y) >= n {
        return Err(Error::BadData(format!("no tile {z}/{x}/{y} (zoom 0..={MAX_ZOOM})")));
    }
    Ok(())
}

/// `<root>/vexcel/ortho/<z>/<x>/<y>` (extension added: .jpg, .png, or .none for "no imagery").
pub fn tile_base(root: &Path, z: u8, x: u32, y: u32) -> PathBuf {
    root.join("vexcel").join("ortho").join(z.to_string()).join(x.to_string()).join(y.to_string())
}

async fn read_cached(base: &Path) -> Option<OrthoTile> {
    for (e, ct) in [("jpg", "image/jpeg"), ("png", "image/png")] {
        if let Some(bytes) = cache::read(&base.with_extension(e)).await {
            return Some(OrthoTile::Image { bytes, content_type: ct });
        }
    }
    base.with_extension("none").exists().then_some(OrthoTile::Empty)
}

pub fn content_type(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else {
        None
    }
}

fn ext(content_type: &str) -> &'static str {
    if content_type == "image/png" {
        "png"
    } else {
        "jpg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("vexcel_ortho_{tag}_{}", std::process::id()))
    }

    #[test]
    fn xyz_bounds() {
        assert!(check_xyz(0, 0, 0).is_ok());
        assert!(check_xyz(3, 7, 7).is_ok());
        assert!(check_xyz(3, 8, 0).is_err());
        assert!(check_xyz(22, 0, 0).is_err());
    }

    #[test]
    fn sniffs_image_type() {
        assert_eq!(content_type(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(content_type(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(content_type(b"{\"status\":403}"), None);
    }

    #[tokio::test]
    async fn cached_image_and_cached_empty_need_no_key() {
        let r = root("cache");
        let t = OrthoTiles::new(None, r.clone()).unwrap();
        cache::write(&tile_base(&r, 18, 1, 2).with_extension("png"), b"\x89PNG\r\n\x1a\nx").await.unwrap();
        cache::write(&tile_base(&r, 18, 1, 3).with_extension("none"), b"").await.unwrap();
        let png = t.get(18, 1, 2).await.unwrap();
        let none = t.get(18, 1, 3).await.unwrap();
        let unknown = t.get(18, 1, 4).await.unwrap(); // not cached, no key: empty, no network
        std::fs::remove_dir_all(&r).ok();
        assert!(matches!(png, OrthoTile::Image { content_type: "image/png", .. }));
        assert_eq!(none, OrthoTile::Empty);
        assert_eq!(unknown, OrthoTile::Empty);
    }

    /// Live: LA has imagery, Princeton is outside the key's area. `cargo test -p vexcel --lib -- --ignored ortho`
    #[tokio::test]
    #[ignore]
    async fn live_la_has_tiles_princeton_does_not() {
        let r = root("live");
        let t = OrthoTiles::new(Some(ApiKey::from_env().expect("key")), r.clone()).unwrap();
        let la = t.get(18, 44_964, 104_675).await.unwrap(); // downtown LA
        let princeton = t.get(18, 76_706, 98_909).await.unwrap(); // Nassau Hall
        std::fs::remove_dir_all(&r).ok();
        assert!(matches!(la, OrthoTile::Image { .. }), "LA should have imagery");
        assert_eq!(princeton, OrthoTile::Empty);
    }
}
