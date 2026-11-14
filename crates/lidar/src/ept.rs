//! Pure: the Entwine Point Tile (EPT) octree that USGS publishes its LiDAR in.
//!
//! A project is a cube (`bounds`, EPSG:3857 metres) split in 8 again and again. Node `D-X-Y-Z` is at depth D,
//! with cube size `size0 / 2^D`. Every node holds a thinned sample of the points in its cube (about
//! `span` points across), so a shallow node is a coarse view and the deepest nodes are full detail.
//! `ept-hierarchy/<key>.json` maps keys to point counts; a count of -1 means "the rest of this branch is in
//! another hierarchy file named after this key".

use std::collections::HashMap;

/// `D-X-Y-Z`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub d: u32,
    pub x: u64,
    pub y: u64,
    pub z: u64,
}

impl Key {
    pub const ROOT: Key = Key { d: 0, x: 0, y: 0, z: 0 };

    pub fn name(&self) -> String {
        format!("{}-{}-{}-{}", self.d, self.x, self.y, self.z)
    }

    pub fn children(&self) -> impl Iterator<Item = Key> + '_ {
        (0..8u64).map(move |i| Key {
            d: self.d + 1,
            x: 2 * self.x + (i & 1),
            y: 2 * self.y + ((i >> 1) & 1),
            z: 2 * self.z + ((i >> 2) & 1),
        })
    }
}

/// A box on the map in EPSG:3857 metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Rect {
    pub fn intersects(&self, o: &Rect) -> bool {
        self.min_x < o.max_x && o.min_x < self.max_x && self.min_y < o.max_y && o.min_y < self.max_y
    }
}

/// The project cube, from `ept.json` (`bounds` = [minx, miny, minz, maxx, maxy, maxz]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cube {
    pub min_x: f64,
    pub min_y: f64,
    pub size: f64,
}

impl Cube {
    pub fn from_bounds(b: &[f64]) -> Option<Cube> {
        (b.len() == 6).then(|| Cube { min_x: b[0], min_y: b[1], size: b[3] - b[0] })
    }

    /// Footprint of a node on the map.
    pub fn rect(&self, k: Key) -> Rect {
        let s = self.size / f64::powi(2.0, k.d as i32);
        let (x0, y0) = (self.min_x + k.x as f64 * s, self.min_y + k.y as f64 * s);
        Rect { min_x: x0, min_y: y0, max_x: x0 + s, max_y: y0 + s }
    }

    /// Rough point spacing (3857 metres) of the sample stored at depth `d`.
    pub fn spacing(&self, d: u32, span: f64) -> f64 {
        self.size / f64::powi(2.0, d as i32) / span
    }
}

/// What to download for an area.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Nodes with points to fetch.
    pub nodes: Vec<Key>,
    /// Hierarchy pages (-1 entries) we must fetch before the plan is complete.
    pub pages: Vec<Key>,
}

/// Walk the hierarchy: every node touching `near` (any depth), plus nodes touching `far` down to `far_depth`.
/// Call again after fetching `pages` into `hier` until `pages` is empty.
pub fn plan(hier: &HashMap<String, i64>, cube: &Cube, near: &Rect, far: &Rect, far_depth: u32) -> Plan {
    let mut out = Plan::default();
    let mut todo = vec![Key::ROOT];
    while let Some(k) = todo.pop() {
        let Some(&n) = hier.get(&k.name()) else { continue };
        let r = cube.rect(k);
        let wanted = r.intersects(near) || (k.d <= far_depth && r.intersects(far));
        if !wanted {
            continue;
        }
        if n == -1 {
            // the page for this key lists the node itself and its subtree
            out.pages.push(k);
            continue;
        }
        if n > 0 {
            out.nodes.push(k);
        }
        todo.extend(k.children());
    }
    out.nodes.sort();
    out.pages.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> Cube {
        Cube { min_x: 0.0, min_y: 0.0, size: 1024.0 }
    }

    #[test]
    fn key_names_and_children() {
        assert_eq!(Key::ROOT.name(), "0-0-0-0");
        let c: Vec<String> = Key { d: 1, x: 1, y: 0, z: 1 }.children().map(|k| k.name()).collect();
        assert_eq!(c.len(), 8);
        assert!(c.contains(&"2-2-0-2".to_string()) && c.contains(&"2-3-1-3".to_string()));
    }

    #[test]
    fn node_rects_halve_each_level() {
        let r = cube().rect(Key { d: 2, x: 3, y: 1, z: 0 });
        assert_eq!(r, Rect { min_x: 768.0, min_y: 256.0, max_x: 1024.0, max_y: 512.0 });
        assert_eq!(cube().spacing(2, 256.0), 1.0);
    }

    #[test]
    fn plan_goes_deep_near_and_shallow_far() {
        let mut hier = HashMap::new();
        // a full tree to depth 3, with points everywhere
        let mut todo = vec![Key::ROOT];
        while let Some(k) = todo.pop() {
            hier.insert(k.name(), 10);
            if k.d < 3 {
                todo.extend(k.children());
            }
        }
        let near = Rect { min_x: 10.0, min_y: 10.0, max_x: 20.0, max_y: 20.0 };
        let far = Rect { min_x: 0.0, min_y: 0.0, max_x: 600.0, max_y: 600.0 };
        let p = plan(&hier, &cube(), &near, &far, 1);
        assert!(p.pages.is_empty());
        // near: one column of cubes per depth (8 z-slices... only those with keys), far: depths 0-1
        assert!(p.nodes.iter().any(|k| k.d == 3 && k.x == 0 && k.y == 0));
        assert!(!p.nodes.iter().any(|k| k.d == 3 && k.x == 2));
        assert!(p.nodes.iter().any(|k| k.d == 1 && k.x == 1 && k.y == 1));
    }

    #[test]
    fn plan_asks_for_missing_pages() {
        let hier = HashMap::from([("0-0-0-0".to_string(), 5), ("1-0-0-0".to_string(), -1)]);
        let all = Rect { min_x: 0.0, min_y: 0.0, max_x: 1024.0, max_y: 1024.0 };
        let p = plan(&hier, &cube(), &all, &all, 9);
        assert_eq!(p.nodes, vec![Key::ROOT]);
        assert_eq!(p.pages, vec![Key { d: 1, x: 0, y: 0, z: 0 }]);
    }
}
