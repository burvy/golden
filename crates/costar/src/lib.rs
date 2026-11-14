//! costar: the answer to "who do I call to get on this roof".
//!
//! Loads the CoStar hackathon kit from the folder in `COSTAR_KIT_DIR` (kept out of git) and finds the
//! buildings near a point, as contract `Building`s: height, floors, contacts and Matterport tours.
//!
//! ```no_run
//! let near = costar::buildings_near(34.0490, -118.2510, 150.0)?;
//! for b in &near {
//!     println!("{:.0} m  {:?}  {} contacts", b.dist_m.unwrap(), b.name, b.contacts.len());
//! }
//! # Ok::<(), costar::Error>(())
//! ```
//!
//! What it reads from the kit (only these files; see the kit's `index.md`):
//! - `data/commercial-property.jsonl`: the buildings (required).
//! - `data/commercial-listing-summary.jsonl`: brokers on active listings, and names/emails for the
//!   building's own contacts (optional).
//! - `data/company-detail.jsonl`: company names and phones for the building's contacts (optional).
//! - `commercial-properties.csv`: Matterport model ids (optional).
//!
//! NEVER commit kit data, not even a few rows. Tests build a tiny made-up kit in a temp folder.

pub mod convert;
pub mod raw;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use contracts::{Building, BuildingList, Contact, ContactRole, Matterport};
use serde::de::DeserializeOwned;

/// Env var holding the path of the unzipped CoStar kit (the folder with `index.md` and `data/`).
pub const KIT_DIR_VAR: &str = "COSTAR_KIT_DIR";

/// Mean Earth radius, for distances between two lat/lon points.
const EARTH_MEAN_R_M: f64 = 6_371_008.8;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no CoStar kit: set {KIT_DIR_VAR} to the folder of the unzipped kit (the one with index.md and data/)")]
    NoKitDir,
    #[error("CoStar kit file {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("CoStar kit file {path}, line {line}: {source}")]
    Json { path: PathBuf, line: usize, source: serde_json::Error },
}

/// The loaded kit: every building with coordinates, ready to search.
#[derive(Debug, Clone)]
pub struct Kit {
    buildings: Vec<Building>,
}

impl Kit {
    /// Load the kit from the folder in `COSTAR_KIT_DIR`.
    pub fn from_env() -> Result<Kit, Error> {
        let dir = std::env::var_os(KIT_DIR_VAR).filter(|v| !v.is_empty()).ok_or(Error::NoKitDir)?;
        Kit::load(Path::new(&dir))
    }

    /// Load the kit from `dir`. Takes a second or two on the real kit (a 190 MB file), so load once.
    pub fn load(dir: &Path) -> Result<Kit, Error> {
        let data = dir.join("data");

        let companies: HashMap<u64, raw::Company> =
            read_jsonl_optional(&data.join("company-detail.jsonl"))?.into_iter().map(|c: raw::Company| (c.id, c)).collect();

        let mut people = HashMap::new();
        let mut brokers: HashMap<u64, Vec<raw::ListingContact>> = HashMap::new();
        for listing in read_jsonl_optional::<raw::Listing>(&data.join("commercial-listing-summary.jsonl"))? {
            for p in listing.property_contacts {
                people.insert(p.id, p);
            }
            if let (Some(pid), Some(raw::LISTING_ACTIVE)) = (listing.property_id, listing.listing_status) {
                brokers.entry(pid).or_default().extend(listing.listing_contacts);
            }
        }

        let matterport = read_matterport_csv(&dir.join("commercial-properties.csv"))?;

        let look = convert::Lookups { companies: &companies, people: &people, brokers: &brokers, matterport: &matterport };
        let buildings = read_jsonl::<raw::Property>(&data.join("commercial-property.jsonl"))?
            .iter()
            .filter_map(|p| convert::building(p, &look))
            .collect();
        Ok(Kit { buildings })
    }

    /// Buildings within `radius_m` metres of the point, nearest first, with `dist_m` set.
    pub fn buildings_near(&self, lat: f64, lon: f64, radius_m: f64) -> Vec<Building> {
        let mut near: Vec<Building> = self
            .buildings
            .iter()
            .filter_map(|b| {
                let d = distance_m(lat, lon, b.lat, b.lon);
                (d <= radius_m).then(|| Building { dist_m: Some((d * 10.0).round() / 10.0), ..b.clone() })
            })
            .collect();
        near.sort_by(|a, b| a.dist_m.partial_cmp(&b.dist_m).expect("distances are finite"));
        near
    }

    /// One building by contract id (`costar:<id>`), with `dist_m: None`.
    pub fn building(&self, id: &str) -> Option<Building> {
        self.buildings.iter().find(|b| b.id == id).cloned()
    }

    /// Number of buildings loaded.
    pub fn len(&self) -> usize {
        self.buildings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buildings.is_empty()
    }
}

/// Buildings within `radius_m` of the point, nearest first, from the kit in `COSTAR_KIT_DIR`.
/// The kit is loaded on the first call and kept for the life of the process.
pub fn buildings_near(lat: f64, lon: f64, radius_m: f64) -> Result<Vec<Building>, Error> {
    Ok(shared_kit()?.buildings_near(lat, lon, radius_m))
}

/// The kit from `COSTAR_KIT_DIR`, loaded once per process.
pub fn shared_kit() -> Result<&'static Kit, Error> {
    static KIT: OnceLock<Kit> = OnceLock::new();
    if let Some(kit) = KIT.get() {
        return Ok(kit);
    }
    let kit = Kit::from_env()?;
    Ok(KIT.get_or_init(|| kit))
}

/// Great-circle distance in metres (haversine).
pub fn distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_MEAN_R_M * a.sqrt().asin()
}

fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, Error> {
    let io = |source| Error::Io { path: path.to_path_buf(), source };
    let reader = BufReader::new(File::open(path).map_err(io)?);
    let mut out = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(io)?;
        if line.trim().is_empty() {
            continue;
        }
        let item = serde_json::from_str(&line)
            .map_err(|source| Error::Json { path: path.to_path_buf(), line: i + 1, source })?;
        out.push(item);
    }
    Ok(out)
}

/// Like `read_jsonl`, but a missing file is empty (the kit still works without contacts).
fn read_jsonl_optional<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, Error> {
    if path.exists() { read_jsonl(path) } else { Ok(Vec::new()) }
}

/// `commercial-properties.csv`: `PropertyId,MatterportModelId,Address,City,State`, one row per model.
/// Only the first two columns are read, and neither contains commas.
fn read_matterport_csv(path: &Path) -> Result<HashMap<u64, Vec<String>>, Error> {
    let mut out: HashMap<u64, Vec<String>> = HashMap::new();
    if !path.exists() {
        return Ok(out);
    }
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })?;
    for line in text.lines().skip(1) {
        let mut cols = line.split(',');
        let (Some(id), Some(model)) = (cols.next(), cols.next()) else { continue };
        let (Ok(id), model) = (id.trim().parse::<u64>(), model.trim()) else { continue };
        if !model.is_empty() {
            let models = out.entry(id).or_default();
            if !models.iter().any(|m| m == model) {
                models.push(model.to_string());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_of_one_degree_of_latitude() {
        let d = distance_m(34.0, -118.0, 35.0, -118.0);
        assert!((d - 111_195.0).abs() < 5.0, "{d}");
        assert_eq!(distance_m(34.05, -118.25, 34.05, -118.25), 0.0);
    }
}
