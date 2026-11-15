//! Test-running a cost function before the agent uses or saves it: score an even sample of ~50 standpoints in
//! the brief's area and report the spread, so the agent can see whether its function separates good points from
//! bad (docs/metrics.md v2, `define_cost_function`).
//!
//! Pure. It uses the same core pieces search does (`site`, `sun_track`, `score::look`, the skyline ray march,
//! `features::*`, `expr::score_of`), so a sampled point scores what search would give it; a test checks that
//! against `search_with`. TODO(core): this belongs in core as `search::sample` (raised with burvy); it lives
//! here until then because the server can't edit core.

use chrono::NaiveDate;
use contracts::{Brief, DsmGrid, Standing};
use golden_core::expr::{score_of, Checked};
use golden_core::features::{self, Feature};
use golden_core::horizon::{ray_max_below, Eye};
use golden_core::score::{self, Look, SkyWindow, SunTrack, FLANK_LOOK_DEG, GAP_SEARCH_DEG};
use golden_core::search::{self, Site, EYE_M, SKYLINE_RADIUS_M, SKY_STEP_DEG};
use serde_json::{json, Value};

/// Candidates per side of the square laid over the area: 8 x 8, about 50 inside the circle.
pub const SAMPLE_SIDE: u32 = 8;
/// Scores that vary less than this across the sample don't separate anything.
pub const MIN_SPREAD: f64 = 0.02;

/// One sampled standpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
    pub standing: Standing,
    /// The function's `must` was 0 here.
    pub dropped: bool,
    pub score: f64,
    /// Every feature the function reads, with its value here.
    pub values: Vec<(&'static str, f64)>,
}

/// What a trial found.
#[derive(Debug, Clone, PartialEq)]
pub struct Trial {
    /// Sample points inside the area, and how many of them someone can stand on (for the brief's standing).
    pub tried: usize,
    pub points: Vec<Point>,
}

impl Trial {
    pub fn dropped(&self) -> usize {
        self.points.iter().filter(|p| p.dropped).count()
    }

    /// Kept points, worst to best.
    fn kept(&self) -> Vec<&Point> {
        let mut k: Vec<&Point> = self.points.iter().filter(|p| !p.dropped).collect();
        k.sort_by(|a, b| a.score.total_cmp(&b.score));
        k
    }

    /// Why this function shouldn't be saved, judged on this sample; None if it's fine.
    pub fn degenerate(&self) -> Option<String> {
        let kept = self.kept();
        if self.points.is_empty() {
            return Some("no point in the sample can be stood on; nothing to judge".into());
        }
        if kept.is_empty() {
            return Some(format!("its must drops all {} sampled points", self.points.len()));
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
        let example = |p: &&Point| {
            json!({
                "lat": round5(p.lat), "lon": round5(p.lon),
                "standing": p.standing,
                "score": round3(p.score),
                "features": p.values.iter().map(|(n, v)| (n.to_string(), json!(round3(*v)))).collect::<serde_json::Map<_, _>>()
            })
        };
        let dropped_example = self.points.iter().find(|p| p.dropped);
        json!({
            "sampled": self.tried,
            "standable": self.points.len(),
            "dropped_by_must": self.dropped(),
            "dropped_share": if self.points.is_empty() { 0.0 } else { round3(self.dropped() as f64 / self.points.len() as f64) },
            "score": {"min": pct(0.0).map(round3), "median": pct(0.5).map(round3), "max": pct(1.0).map(round3)},
            "best": kept.last().map(example),
            "median_point": kept.get(kept.len() / 2).map(example),
            "worst": kept.first().map(example),
            "a_dropped_point": dropped_example.map(|p| example(&p)),
            "verdict": self.degenerate().unwrap_or_else(|| "separates points: OK to use or save".into()),
            "caution": self.caution(),
        })
    }
}

/// Score `cost` on an even sample of the brief's area. Errors are worded for the agent.
pub fn trial(grid: &DsmGrid, brief: &Brief, cost: &Checked) -> Result<Trial, String> {
    let date = NaiveDate::parse_from_str(&brief.date, "%Y-%m-%d").map_err(|_| format!("bad brief date {:?}", brief.date))?;
    let track = search::sun_track(brief.area.lat, brief.area.lon, date, brief.event).ok_or("the sun doesn't rise or set there that day")?;
    let (mut sky, n_az) = window(&track);
    let top_m = grid_top(grid);
    let mut used: Vec<Feature> = Vec::new();
    for n in cost.must.iter().chain([&cost.score]) {
        n.each_feature(&mut |f| {
            if !used.contains(&f) {
                used.push(f);
            }
        });
    }

    let mut tried = 0;
    let mut points = Vec::new();
    for (lat, lon) in sample(brief.area.lat, brief.area.lon, brief.area.radius_m) {
        tried += 1;
        let Some(site) = search::site(grid, lat, lon) else { continue };
        if brief.standing != Standing::Any && brief.standing != site.standing {
            continue;
        }
        let mut look: Option<Look> = None;
        let mut value = |f: Feature| feature(f, &site, &mut look, || trace(grid, lat, lon, site.ground_m + EYE_M, top_m, n_az, &mut sky, &track));
        let dropped = cost.must.as_ref().is_some_and(|m| score_of(m, &mut value) <= 0.0);
        let score = if dropped { 0.0 } else { score_of(&cost.score, &mut value) };
        let values = used.iter().map(|&f| (f.name(), value(f))).collect();
        points.push(Point { lat, lon, standing: site.standing, dropped, score, values });
    }
    Ok(Trial { tried, points })
}

/// Even grid of points inside the circle.
fn sample(lat: f64, lon: f64, radius_m: f64) -> Vec<(f64, f64)> {
    let step = 2.0 * radius_m / f64::from(SAMPLE_SIDE);
    let mut out = Vec::new();
    for row in 0..SAMPLE_SIDE {
        for col in 0..SAMPLE_SIDE {
            let east = (f64::from(col) + 0.5) * step - radius_m;
            let north = radius_m - (f64::from(row) + 0.5) * step;
            if east.hypot(north) <= radius_m {
                let plat = lat + (north / contracts::EARTH_R).to_degrees();
                out.push((plat, lon + (east / (contracts::EARTH_R * lat.to_radians().cos())).to_degrees()));
            }
        }
    }
    out
}

fn feature(f: Feature, site: &Site, look: &mut Option<Look>, trace: impl FnOnce() -> Look) -> f64 {
    if let Some(v) = features::point_value(f, site, EYE_M) {
        return v;
    }
    features::sun_value(f, look.get_or_insert_with(trace)).unwrap_or(0.0)
}

/// An empty skyline window over the sun's track plus room for a gap's sides, and how many azimuths it holds
/// (as search builds it).
fn window(track: &SunTrack) -> (SkyWindow, usize) {
    let center_az = track.samples.first().map_or(270.0, |s| s.az_deg);
    let (lo, hi) = track.samples.iter().map(|s| score::wrap180(s.az_deg - center_az)).fold((0.0f64, 0.0f64), |(lo, hi), r| (lo.min(r), hi.max(r)));
    let side = GAP_SEARCH_DEG + FLANK_LOOK_DEG + SKY_STEP_DEG;
    let (lo_deg, hi_deg) = (lo.min(0.0) - side, hi.max(0.0) + side);
    let n = ((hi_deg - lo_deg) / SKY_STEP_DEG).ceil() as usize + 1;
    (SkyWindow { center_az, lo_deg, step_deg: SKY_STEP_DEG, elev: Vec::with_capacity(n), dist: Vec::with_capacity(n) }, n)
}

/// The candidate's `Look`: its skyline traced into `sky`, then the sun's track checked against it.
#[allow(clippy::too_many_arguments)]
fn trace(grid: &DsmGrid, lat: f64, lon: f64, eye_m: f64, top_m: f64, n: usize, sky: &mut SkyWindow, track: &SunTrack) -> Look {
    let (col, row) = grid.cell_of(lat, lon);
    let eye = Eye { col, row, eye_m };
    sky.elev.clear();
    sky.dist.clear();
    for i in 0..n {
        let m = ray_max_below(grid, eye, sky.az_of(i), SKYLINE_RADIUS_M, top_m);
        sky.elev.push(m.elev_deg);
        sky.dist.push(m.dist_m);
    }
    score::look(track, sky)
}

fn grid_top(grid: &DsmGrid) -> f64 {
    f64::from(grid.data.iter().filter(|&&h| h != grid.meta.nodata).fold(f32::MIN, |a, &h| a.max(h)))
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
    use contracts::{Area, CostFn, Event, Framing};
    use golden_core::expr::check;

    fn brief(radius_m: f64, date: &str) -> Brief {
        Brief {
            text: String::new(),
            area: Area { lat: testkit::LAT, lon: testkit::LON, radius_m },
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

    fn checked(json: &str) -> Checked {
        check(&serde_json::from_str::<CostFn>(json).unwrap()).unwrap()
    }

    #[test]
    fn samples_about_fifty_points() {
        let n = sample(testkit::LAT, testkit::LON, 300.0).len();
        assert!((44..=56).contains(&n), "{n}");
    }

    #[test]
    fn a_roof_gate_drops_street_points_and_says_how_many() {
        let g = testkit::city();
        let t = trial(&g, &brief(300.0, "2026-06-21"), &checked(r#"{"name":"r","about":"r","must":{"f":"roof"},"score":{"f":"sun_clear"}}"#)).unwrap();
        assert!(t.dropped() > 0 && t.dropped() < t.points.len(), "{} of {}", t.dropped(), t.points.len());
        assert!(t.points.iter().all(|p| p.dropped == (p.standing == Standing::Ground)));
        let s = t.summary();
        assert_eq!(s["dropped_by_must"], t.dropped());
        assert_eq!(s["best"]["features"]["roof"], 1.0);
        assert!(s["a_dropped_point"]["features"]["roof"] == 0.0);
    }

    #[test]
    fn flat_functions_are_flagged() {
        let g = testkit::city();
        let t = trial(&g, &brief(300.0, "2026-06-21"), &checked(r#"{"name":"c","about":"c","score":{"num":0.5}}"#)).unwrap();
        assert!(t.degenerate().unwrap().contains("doesn't separate"), "{:?}", t.degenerate());
        let t = trial(&g, &brief(300.0, "2026-06-21"), &checked(r#"{"name":"n","about":"n","must":{"num":0},"score":{"num":1}}"#)).unwrap();
        assert!(t.degenerate().unwrap().contains("drops all"));
        // a gate that separates (drops the few roofs), with a flat score behind it: accepted, with a caution
        let t = trial(&g, &brief(300.0, "2026-06-21"), &checked(r#"{"name":"g","about":"g","must":{"op":"not","x":{"f":"roof"}},"score":{"num":1}}"#)).unwrap();
        assert_eq!(t.degenerate(), None);
        assert!(t.summary()["caution"].as_str().unwrap().contains("only the must separates"));
        // the wall hides the June sun at different heights for different points: this one separates
        let t = trial(&g, &brief(300.0, "2026-06-21"), &checked(r#"{"name":"s","about":"s","score":{"op":"ramp","x":{"f":"sun_visible_min"},"lo":0,"hi":30}}"#)).unwrap();
        assert_eq!(t.degenerate(), None, "{}", t.summary());
    }

    #[test]
    fn a_sampled_point_scores_what_search_gives_it() {
        // Search's best spot, re-scored through the trial's own feature path, must match search's per-goal score.
        let g = testkit::city();
        let def = r#"{"name":"sunny_roof","about":"Sun in view on a roof","score":{"op":"mul","of":[{"f":"roof"},{"f":"sun_clear"}]}}"#;
        let b = Brief { goals: serde_json::from_str(&format!(r#"[{{"def":{def}}}]"#)).unwrap(), ..brief(300.0, "2026-06-21") };
        let out = search::search(&g, &b, -7 * 3600).unwrap();
        let top = &out.spots[0];
        // a 1 m "area" round the spot: every sampled point is in the spot's own cell or its neighbour
        let one = Brief { area: Area { lat: top.lat, lon: top.lon, radius_m: 1.0 }, ..b.clone() };
        let t = trial(&g, &one, &checked(def)).unwrap();
        assert!(!t.points.is_empty());
        for p in &t.points {
            assert!((p.score - top.scores[0].score).abs() < 0.02, "trial {} vs search {}", p.score, top.scores[0].score);
        }
    }
}
