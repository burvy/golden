//! Pure: pick the USGS LiDAR project that covers a point, from the public index
//! (`https://usgs.entwine.io/boundaries/resources.geojson`, one polygon per project).

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub name: String,
    /// `.../ept.json`
    pub url: String,
    pub points: u64,
}

impl Project {
    /// Survey year: the first 4-digit 19xx/20xx in the name ("DE_DelawareValley_HD_2015_LAS_2017" -> 2015).
    pub fn year(&self) -> Option<u32> {
        self.name
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| s.len() == 4)
            .filter_map(|s| s.parse().ok())
            .find(|y| (1990..2100).contains(y))
    }

    /// The folder holding ept.json, hierarchy and data.
    pub fn base_url(&self) -> &str {
        self.url.strip_suffix("ept.json").unwrap_or(&self.url)
    }
}

#[derive(Deserialize)]
struct Fc {
    features: Vec<Feature>,
}
#[derive(Deserialize)]
struct Feature {
    properties: Props,
    geometry: Geometry,
}
#[derive(Deserialize)]
struct Props {
    name: String,
    url: String,
    #[serde(default)]
    count: u64,
}
#[derive(Deserialize)]
#[serde(tag = "type", content = "coordinates")]
enum Geometry {
    Polygon(Vec<Vec<[f64; 2]>>),
    MultiPolygon(Vec<Vec<Vec<[f64; 2]>>>),
    #[serde(other)]
    Other,
}

/// Projects whose outline contains (lat, lon): newest survey first, then the densest.
pub fn covering(geojson: &str, lat: f64, lon: f64) -> Result<Vec<Project>, serde_json::Error> {
    let fc: Fc = serde_json::from_str(geojson)?;
    let mut out: Vec<Project> = fc
        .features
        .into_iter()
        .filter(|f| match &f.geometry {
            Geometry::Polygon(p) => in_polygon(p, lon, lat),
            Geometry::MultiPolygon(ps) => ps.iter().any(|p| in_polygon(p, lon, lat)),
            Geometry::Other => false,
        })
        .map(|f| Project { name: f.properties.name, url: f.properties.url, points: f.properties.count })
        .collect();
    out.sort_by(|a, b| b.year().cmp(&a.year()).then(b.points.cmp(&a.points)));
    Ok(out)
}

/// Inside the outer ring and outside every hole.
fn in_polygon(rings: &[Vec<[f64; 2]>], x: f64, y: f64) -> bool {
    match rings.split_first() {
        Some((outer, holes)) => in_ring(outer, x, y) && !holes.iter().any(|h| in_ring(h, x, y)),
        None => false,
    }
}

/// Even-odd ray casting.
fn in_ring(ring: &[[f64; 2]], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut j = ring.len().wrapping_sub(1);
    for i in 0..ring.len() {
        let ([xi, yi], [xj, yj]) = (ring[i], ring[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","properties":{"name":"NJ_Old_2010","url":"https://x/NJ_Old_2010/ept.json","count":5},
       "geometry":{"type":"Polygon","coordinates":[[[-75,40],[-74,40],[-74,41],[-75,41],[-75,40]]]}},
      {"type":"Feature","properties":{"name":"DE_Valley_HD_2015_LAS_2017","url":"https://x/DE/ept.json","count":9},
       "geometry":{"type":"MultiPolygon","coordinates":[[[[-75,40],[-74,40],[-74,41],[-75,41],[-75,40]],
                                                         [[-74.7,40.3],[-74.6,40.3],[-74.6,40.4],[-74.7,40.4],[-74.7,40.3]]]]}},
      {"type":"Feature","properties":{"name":"CA_Far_2020","url":"https://x/CA/ept.json","count":1},
       "geometry":{"type":"Polygon","coordinates":[[[-119,34],[-118,34],[-118,35],[-119,35],[-119,34]]]}}
    ]}"#;

    #[test]
    fn newest_covering_project_first() {
        let p = covering(INDEX, 40.8, -74.5).unwrap();
        let names: Vec<&str> = p.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["DE_Valley_HD_2015_LAS_2017", "NJ_Old_2010"]);
        assert_eq!(p[0].year(), Some(2015));
        assert_eq!(p[0].base_url(), "https://x/DE/");
    }

    #[test]
    fn holes_are_outside() {
        let p = covering(INDEX, 40.35, -74.65).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "NJ_Old_2010");
    }

    #[test]
    fn nothing_in_the_ocean() {
        assert!(covering(INDEX, 0.0, 0.0).unwrap().is_empty());
    }
}
