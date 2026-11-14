//! Oblique (angled aerial) photos: `get_oblique(lat, lon, az_deg) -> JPEG bytes`.
//!
//! What the live API does (measured, see docs/Nube-Lube/DEVLOG.md):
//! - Obliques come in 4 directions, `product-type=oblique-{north,east,south,west}`, and the name is the
//!   direction the camera LOOKS: for `oblique-west` the camera was ~3 km east of what it shot.
//! - `GET /v2/oriented/extract` with a point needs `bbox-dimensions` (metres); `destination-dimensions`
//!   resamples to our pixel size. One image per call (~400 KB at 1024 px, ~1 s).
//! - The JPEG has no EXIF rotation tag: it is upright as delivered (far side of the view at the top).
//! - Vexcel's attribution is burned into the bottom-right corner. Leave it there (licence).

use std::path::{Path, PathBuf};

use contracts::photo::MAX_PHOTO_PX;

use crate::auth::ApiKey;
use crate::{cache, client, Error};

/// The four directions Vexcel shoots obliques in. Named by where the camera looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    North,
    East,
    South,
    West,
}

impl Facing {
    /// The camera direction closest to `az_deg` (clockwise from north, any value; 45° goes to the next one clockwise).
    pub fn nearest(az_deg: f64) -> Facing {
        match ((az_deg.rem_euclid(360.0) / 90.0).round() as u32) % 4 {
            0 => Facing::North,
            1 => Facing::East,
            2 => Facing::South,
            _ => Facing::West,
        }
    }

    pub fn az_deg(self) -> f64 {
        match self {
            Facing::North => 0.0,
            Facing::East => 90.0,
            Facing::South => 180.0,
            Facing::West => 270.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Facing::North => "north",
            Facing::East => "east",
            Facing::South => "south",
            Facing::West => "west",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ObliqueOptions {
    /// Ground area to show around the point, metres (width, height).
    pub view_m: (u32, u32),
    /// Output image size, pixels (width, height). Each side at most 2048 (phone budget).
    pub size_px: (u32, u32),
    pub cache_root: PathBuf,
}

impl Default for ObliqueOptions {
    fn default() -> Self {
        ObliqueOptions { view_m: (300, 200), size_px: (1024, 683), cache_root: cache::default_root() }
    }
}

/// An oblique photo plus what we know about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Oblique {
    pub jpeg: Vec<u8>,
    /// Direction the camera looks (0, 90, 180 or 270).
    pub look_az_deg: f64,
    pub width: u32,
    pub height: u32,
}

/// JPEG of the view around (lat, lon), from the Vexcel camera looking closest to `az_deg`.
/// E.g. pass the sunset azimuth (~297° in June in LA) to see the buildings the sun sets behind.
pub async fn get_oblique(lat: f64, lon: f64, az_deg: f64) -> Result<Vec<u8>, Error> {
    Ok(get_oblique_with(lat, lon, az_deg, &ObliqueOptions::default()).await?.jpeg)
}

pub async fn get_oblique_with(lat: f64, lon: f64, az_deg: f64, opts: &ObliqueOptions) -> Result<Oblique, Error> {
    let req = ObliqueRequest::new(lat, lon, az_deg, opts)?;
    let path = req.cache_path(&opts.cache_root);
    let jpeg = match cache::read(&path).await {
        Some(bytes) => bytes,
        None => {
            let key = ApiKey::from_env()?;
            let pairs = req.query_pairs();
            let query: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let bytes = client::get_bytes(&client::http()?, &key, client::OBLIQUE_URL, &query).await?;
            if !is_jpeg(&bytes) {
                return Err(Error::BadData("Vexcel oblique was not a JPEG".into()));
            }
            cache::write(&path, &bytes).await?;
            bytes
        }
    };
    Ok(Oblique { jpeg, look_az_deg: req.facing.az_deg(), width: req.size_px.0, height: req.size_px.1 })
}

/// A validated request. Coordinates are rounded to 1e-5° (~1 m) so nearby calls share a cache entry.
#[derive(Debug, Clone, PartialEq)]
pub struct ObliqueRequest {
    pub lat: String,
    pub lon: String,
    pub facing: Facing,
    pub view_m: (u32, u32),
    pub size_px: (u32, u32),
}

impl ObliqueRequest {
    pub fn new(lat: f64, lon: f64, az_deg: f64, opts: &ObliqueOptions) -> Result<ObliqueRequest, Error> {
        let bad = |m: &str| Err(Error::BadData(m.to_string()));
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) || !az_deg.is_finite() {
            return bad("lat/lon out of range or azimuth not a number");
        }
        let (w, h) = opts.size_px;
        if w == 0 || h == 0 || w > MAX_PHOTO_PX || h > MAX_PHOTO_PX {
            return bad("image size must be 1..=2048 px per side");
        }
        let (vw, vh) = opts.view_m;
        if !(10..=2000).contains(&vw) || !(10..=2000).contains(&vh) {
            return bad("view must be 10..=2000 m per side");
        }
        Ok(ObliqueRequest {
            lat: format!("{lat:.5}"),
            lon: format!("{lon:.5}"),
            facing: Facing::nearest(az_deg),
            view_m: opts.view_m,
            size_px: opts.size_px,
        })
    }

    /// Query string for `/v2/oriented/extract`.
    pub fn query_pairs(&self) -> Vec<(&'static str, String)> {
        vec![
            ("wkt", format!("POINT({} {})", self.lon, self.lat)),
            ("srid", "4326".into()),
            ("layer", "urban".into()),
            ("product-type", format!("oblique-{}", self.facing.name())),
            ("bbox-dimensions", format!("{},{}", self.view_m.0, self.view_m.1)),
            ("destination-dimensions", format!("{},{},fit", self.size_px.0, self.size_px.1)),
            ("image-format", "jpeg".into()),
            // newest flight first, then the photo whose centre is closest to our point (least distorted)
            ("order-by", "collection-last-capture-date-desc,image-center-distance-asc".into()),
        ]
    }

    /// `<root>/vexcel/oblique/<facing>/<lat>_<lon>_<w>x<h>m_<pw>x<ph>.jpg`
    pub fn cache_path(&self, root: &Path) -> PathBuf {
        root.join("vexcel").join("oblique").join(self.facing.name()).join(format!(
            "{}_{}_{}x{}m_{}x{}.jpg",
            self.lat, self.lon, self.view_m.0, self.view_m.1, self.size_px.0, self.size_px.1
        ))
    }
}

pub fn is_jpeg(b: &[u8]) -> bool {
    b.starts_with(&[0xFF, 0xD8, 0xFF])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_facing() {
        assert_eq!(Facing::nearest(297.0), Facing::West); // LA sunset in June
        assert_eq!(Facing::nearest(240.0), Facing::West); // LA sunset in December
        assert_eq!(Facing::nearest(60.0), Facing::East); // sunrise
        assert_eq!(Facing::nearest(350.0), Facing::North);
        assert_eq!(Facing::nearest(-10.0), Facing::North);
        assert_eq!(Facing::nearest(720.0 + 180.0), Facing::South);
        assert_eq!(Facing::nearest(45.0), Facing::East);
    }

    #[test]
    fn query_asks_for_the_right_image() {
        let r = ObliqueRequest::new(34.049, -118.251, 297.0, &ObliqueOptions::default()).unwrap();
        let q = r.query_pairs();
        let get = |k: &str| q.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("wkt"), Some("POINT(-118.25100 34.04900)"));
        assert_eq!(get("product-type"), Some("oblique-west"));
        assert_eq!(get("destination-dimensions"), Some("1024,683,fit"));
        assert_eq!(get("image-format"), Some("jpeg"));
    }

    #[test]
    fn rejects_bad_requests() {
        let o = ObliqueOptions::default();
        assert!(ObliqueRequest::new(95.0, 0.0, 0.0, &o).is_err());
        assert!(ObliqueRequest::new(34.0, -118.0, f64::NAN, &o).is_err());
        let huge = ObliqueOptions { size_px: (4096, 4096), ..ObliqueOptions::default() };
        assert!(ObliqueRequest::new(34.0, -118.0, 0.0, &huge).is_err());
        let tiny_view = ObliqueOptions { view_m: (1, 1), ..ObliqueOptions::default() };
        assert!(ObliqueRequest::new(34.0, -118.0, 0.0, &tiny_view).is_err());
    }

    #[test]
    fn nearby_points_share_a_cache_entry() {
        let o = ObliqueOptions::default();
        let a = ObliqueRequest::new(34.049_001, -118.251_002, 290.0, &o).unwrap();
        let b = ObliqueRequest::new(34.049_002, -118.251_001, 300.0, &o).unwrap();
        assert_eq!(a.cache_path(Path::new("c")), b.cache_path(Path::new("c")));
        let p = a.cache_path(Path::new("c"));
        assert!(p.ends_with(Path::new("west").join("34.04900_-118.25100_300x200m_1024x683.jpg")));
    }

    #[tokio::test]
    async fn serves_from_cache_without_a_key() {
        let root = std::env::temp_dir().join(format!("vexcel_oblique_test_{}", std::process::id()));
        let opts = ObliqueOptions { cache_root: root.clone(), ..ObliqueOptions::default() };
        let req = ObliqueRequest::new(34.049, -118.251, 297.0, &opts).unwrap();
        let fake = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
        cache::write(&req.cache_path(&root), &fake).await.unwrap();
        let got = get_oblique_with(34.049, -118.251, 297.0, &opts).await.unwrap();
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(got.jpeg, fake);
        assert_eq!(got.look_az_deg, 270.0);
    }

    /// Live: hits Vexcel. `cargo test -p vexcel --lib -- --ignored oblique::tests::live`
    #[tokio::test]
    #[ignore]
    async fn live_west_view_downtown_la() {
        let jpeg = get_oblique(34.0490, -118.2510, 297.0).await.expect("fetch");
        assert!(is_jpeg(&jpeg));
        assert!(jpeg.len() > 50_000, "suspiciously small: {} bytes", jpeg.len());
    }
}
