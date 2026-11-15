//! Test-running a cost function before the agent uses or saves it: score every candidate in the brief's area
//! and report the spread, so the agent can see whether its function separates good points from bad
//! (docs/metrics.md v2, `define_cost_function`).
//!
//! Pure. It runs core's own search (`search_with`) with the function as the only goal and reads that goal's
//! layer (its raw score at each cell's candidate, and the cells its `must` drops), so a trial scores exactly
//! what search would: objects, lens and the funnel included. The best, median and worst points are then
//! searched again on their own (a 1 m area) for the goal's one-line reason, which carries the feature numbers
//! behind the score.

use std::collections::BTreeMap;

use contracts::{Brief, CostFn, DsmGrid, Goal, GoalFn, Object, ObjRef, Standing, EARTH_R};
use golden_core::search::{self, Inputs};
use serde_json::{json, Value};

/// Scores that vary less than this across the sample don't separate anything.
pub const MIN_SPREAD: f64 = 0.02;

/// One scored candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
    pub standing: Standing,
    /// The function's `must` was 0 here.
    pub dropped: bool,
    pub score: f64,
    /// The goal's reason at this point (feature numbers included), for the examples only.
    pub why: Option<String>,
}

/// What a trial found.
#[derive(Debug, Clone, PartialEq)]
pub struct Trial {
    /// Candidates in the area; `points` are the ones someone can stand on (for the brief's standing).
    pub tried: usize,
    pub points: Vec<Point>,
}

impl Trial {
    pub fn dropped(&self) -> usize {
        self.points.iter().filter(|p| p.dropped).count()
    }

    /// Indexes of the kept points, worst to best.
    fn kept_idx(&self) -> Vec<usize> {
        let mut k: Vec<usize> = (0..self.points.len()).filter(|&i| !self.points[i].dropped).collect();
        k.sort_by(|&a, &b| self.points[a].score.total_cmp(&self.points[b].score));
        k
    }

    fn kept(&self) -> Vec<&Point> {
        self.kept_idx().into_iter().map(|i| &self.points[i]).collect()
    }

    /// (worst, median, best) kept points, as indexes.
    fn examples(&self) -> Option<(usize, usize, usize)> {
        let k = self.kept_idx();
        Some((*k.first()?, k[k.len() / 2], *k.last()?))
    }

    /// Why this function shouldn't be saved, judged on this sample; None if it's fine.
    pub fn degenerate(&self) -> Option<String> {
        let kept = self.kept();
        if self.points.is_empty() {
            return Some("no point in the area can be stood on; nothing to judge".into());
        }
        if kept.is_empty() {
            return Some(format!("its must drops all {} points in the area", self.points.len()));
        }
        // A gate that drops some points separates them, even if the score is flat among the rest.
        let (lo, hi) = (kept[0].score, kept[kept.len() - 1].score);
        if self.dropped() == 0 && kept.len() >= 3 && hi - lo < MIN_SPREAD {
            return Some(format!("every point scores about {hi:.2} and none is dropped, so it doesn't separate good points from bad"));
        }
        None
    }

    /// Worth telling the agent, but not a reason to refuse it.
    fn caution(&self) -> Option<String> {
        let kept = self.kept();
        let spread = kept.last().map_or(0.0, |b| b.score) - kept.first().map_or(0.0, |w| w.score);
        (self.dropped() > 0 && kept.len() >= 3 && spread < MIN_SPREAD)
            .then(|| "only the must separates points: the kept ones all score the same, so they can't be ranked by it".into())
    }

    /// The summary the agent reads: the distribution, and the best, median and worst kept points.
    pub fn summary(&self) -> Value {
        let kept = self.kept();
        let scores: Vec<f64> = kept.iter().map(|p| p.score).collect();
        let pct = |q: f64| scores.get(((scores.len().saturating_sub(1)) as f64 * q).round() as usize).copied();
        let example = |p: &Point| {
            json!({
                "lat": round5(p.lat), "lon": round5(p.lon),
                "standing": p.standing,
                "score": round3(p.score),
                "why": p.why,
            })
        };
        let ex = self.examples();
        json!({
            "candidates": self.tried,
            "standable": self.points.len(),
            "dropped_by_must": self.dropped(),
            "dropped_share": if self.points.is_empty() { 0.0 } else { round3(self.dropped() as f64 / self.points.len() as f64) },
            "score": {"min": pct(0.0).map(round3), "median": pct(0.5).map(round3), "max": pct(1.0).map(round3)},
            "best": ex.map(|(_, _, b)| example(&self.points[b])),
            "median_point": ex.map(|(_, m, _)| example(&self.points[m])),
            "worst": ex.map(|(w, _, _)| example(&self.points[w])),
            "a_dropped_point": self.points.iter().find(|p| p.dropped).map(example),
            "verdict": self.degenerate().unwrap_or_else(|| "separates points: OK to use or save".into()),
            "caution": self.caution(),
        })
    }
}

/// The goal a trial searches with: inline when the function takes no objects, else by name with `args`.
fn goal_for(cost: &CostFn, args: &BTreeMap<String, ObjRef>) -> Goal {
    let fun = if cost.params.is_empty() { GoalFn::Def(cost.clone()) } else { GoalFn::Use { name: cost.name.clone(), args: args.clone() } };
    Goal { cost: fun, weight: None, must: false }
}

/// Score `cost` at every candidate of the brief's area, with `library` (for `use`) and `objects`.
/// `args` fill the function's params, if it has any. Errors are worded for the agent.
pub fn trial(
    grid: &DsmGrid,
    brief: &Brief,
    cost: &CostFn,
    args: &BTreeMap<String, ObjRef>,
    library: &[CostFn],
    objects: &[Object],
) -> Result<Trial, String> {
    let mut lib = library.to_vec();
    lib.retain(|f| f.name != cost.name);
    lib.push(cost.clone());
    let inputs = Inputs { library: &lib, objects };
    let b = Brief { goals: vec![goal_for(cost, args)], max_spots: 1, ..brief.clone() };
    // the times in a spot don't matter here, only scores
    let out = search::search_with(grid, &b, &inputs, 0).map_err(|e| e.to_string())?;
    let layer = out.layers.first().ok_or("the search returned no layer for the function")?;
    let heat = &layer.heat;
    let mut points = Vec::new();
    for (i, v) in heat.values.iter().enumerate() {
        let dropped = layer.gate.contains(&(i as u32));
        if v.is_none() && !dropped {
            continue;
        }
        let (lat, lon) = cell_centre(heat, i);
        let standing = search::site(grid, lat, lon).map_or(Standing::Any, |s| s.standing);
        let score = if dropped { 0.0 } else { f64::from(v.unwrap_or(0.0)) };
        points.push(Point { lat, lon, standing, dropped, score, why: None });
    }
    let tried = out.stages.first().map_or(points.len(), |s| s.n as usize);
    let mut t = Trial { tried, points };
    // the examples' reasons: each searched again in its own 1 m area
    if let Some((w, m, best)) = t.examples() {
        for i in [w, m, best] {
            if t.points[i].why.is_some() {
                continue;
            }
            let p = &t.points[i];
            let one = Brief { area: contracts::Area { lat: p.lat, lon: p.lon, radius_m: 1.0, ..b.area.clone() }, ..b.clone() };
            let why = search::search_with(grid, &one, &inputs, 0).ok().and_then(|o| o.spots.first().and_then(|s| s.scores.first()).map(|r| r.why.clone()));
            t.points[i].why = why;
        }
    }
    Ok(t)
}

/// Lat/lon of the centre of heat cell `i` (row 0 is the north edge), as core lays the grid out.
fn cell_centre(h: &contracts::HeatGrid, i: usize) -> (f64, f64) {
    let (col, row) = ((i as u32 % h.width) as f64, (i as u32 / h.width) as f64);
    let east = (col + 0.5 - f64::from(h.width) / 2.0) * h.cell_m;
    let north = (f64::from(h.height) / 2.0 - row - 0.5) * h.cell_m;
    (h.center_lat + (north / EARTH_R).to_degrees(), h.center_lon + (east / (EARTH_R * h.center_lat.to_radians().cos())).to_degrees())
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn round5(x: f64) -> f64 {
    (x * 1e5).round() / 1e5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;
    use contracts::{Area, Event, Framing};

    fn brief(radius_m: f64, date: &str) -> Brief {
        Brief {
            text: String::new(),
            area: Area { lat: testkit::LAT, lon: testkit::LON, radius_m, name: None },
            date: date.into(),
            tz: "America/Los_Angeles".into(),
            event: Event::Sunset,
            standing: Standing::Any,
            framing: Framing::Clear,
            max_spots: 10,
            goals: vec![],
            lens_deg: contracts::DEFAULT_LENS_DEG,
        }
    }

    fn cost(json: &str) -> CostFn {
        serde_json::from_str(json).unwrap()
    }

    fn run(c: &str) -> Trial {
        trial(&testkit::city(), &brief(300.0, "2026-06-21"), &cost(c), &BTreeMap::new(), &golden_core::library::seeds(), &[]).unwrap()
    }

    #[test]
    fn a_roof_gate_drops_street_points_and_says_how_many() {
        let t = run(r#"{"name":"r","about":"r","must":{"f":"roof"},"score":{"f":"sun_clear"}}"#);
        assert!(t.dropped() > 0 && t.dropped() < t.points.len(), "{} of {}", t.dropped(), t.points.len());
        assert!(t.points.iter().filter(|p| !p.dropped).all(|p| p.standing == Standing::Roof));
        let s = t.summary();
        assert_eq!(s["dropped_by_must"], t.dropped());
        assert_eq!(s["best"]["standing"], "roof");
        assert!(s["best"]["why"].as_str().is_some_and(|w| !w.is_empty()), "{s}");
    }

    #[test]
    fn flat_functions_are_flagged() {
        let t = run(r#"{"name":"c","about":"c","score":{"num":0.5}}"#);
        assert!(t.degenerate().unwrap().contains("doesn't separate"), "{:?}", t.degenerate());
        let t = run(r#"{"name":"n","about":"n","must":{"num":0},"score":{"num":1}}"#);
        assert!(t.degenerate().unwrap().contains("drops all"), "{:?}", t.degenerate());
        // a gate that separates (drops the few roofs), with a flat score behind it: accepted, with a caution
        let t = run(r#"{"name":"g","about":"g","must":{"op":"not","x":{"f":"roof"}},"score":{"num":1}}"#);
        assert_eq!(t.degenerate(), None);
        assert!(t.summary()["caution"].as_str().unwrap().contains("only the must separates"));
        // the wall hides the June sun at different heights for different points: this one separates
        let t = run(r#"{"name":"s","about":"s","score":{"op":"ramp","x":{"f":"sun_visible_min"},"lo":0,"hi":30}}"#);
        assert_eq!(t.degenerate(), None, "{}", t.summary());
    }

    #[test]
    fn the_best_point_scores_what_search_gives_it() {
        let g = testkit::city();
        let def = r#"{"name":"sunny_roof","about":"Sun in view on a roof","score":{"op":"mul","of":[{"f":"roof"},{"f":"sun_clear"}]}}"#;
        let b = Brief { goals: serde_json::from_str(&format!(r#"[{{"def":{def}}}]"#)).unwrap(), ..brief(300.0, "2026-06-21") };
        let out = search::search(&g, &b, -7 * 3600).unwrap();
        let t = trial(&g, &brief(300.0, "2026-06-21"), &cost(def), &BTreeMap::new(), &golden_core::library::seeds(), &[]).unwrap();
        let best = t.summary()["score"]["max"].as_f64().unwrap();
        assert!((best - out.spots[0].scores[0].score).abs() < 0.002, "trial {best} vs search {}", out.spots[0].scores[0].score);
        assert!(t.points.len() > 50, "every candidate, not a sample: {}", t.points.len());
    }

    #[test]
    fn cells_map_back_to_their_centres() {
        let h = contracts::HeatGrid { center_lat: 34.0, center_lon: -118.0, cell_m: 10.0, width: 4, height: 4, values: vec![None; 16] };
        let (lat, lon) = cell_centre(&h, 0);
        assert!(lat > 34.0 && lon < -118.0, "row 0 col 0 is north-west");
        let north_m = (lat - 34.0).to_radians() * EARTH_R;
        assert!((north_m - 15.0).abs() < 1e-6);
    }
}
