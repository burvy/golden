//! Library lab: which cheap gates cut candidates before the expensive work, without losing the
//! spots an ungated search ranks best? (task 062, docs/library-lab.md)
//!
//! `cargo run -p golden-core --release --example library_lab -- [--write | --report] [--place NAME] [--scenario KEY] [--gate NAME] [--radius M]`
//! (`LAB_THREADS=N`: places run N at a time, default 6; `--report` rebuilds the doc from the last `--write`.)
//!
//! For every place (real height maps, cache only) and day (Nov 14, Jun 21), each scenario (a brief's
//! goals) runs once ungated, then once per gate with the gate added as a `must`. A must goal has
//! weight 0, so it never changes the scores of the candidates it keeps. That makes recall exact: if
//! every spot the ungated search picked survives the gate, the gated search picks the same spots.
//!
//! Per (place, day, scenario, gate) it measures:
//! - **prune**: share of the heat-grid candidates the gate drops (from a gate-only search). In a
//!   scenario scored on the skyline, every dropped candidate is one skyline trace saved.
//! - **recall@10**: share of the ungated top 10 spots still picked with the gate.
//! - **near-best**: share of the ungated candidates scoring >= 0.8 of the best that survive.
//! - **time**: ungated vs gated search, wall clock.
//!
//! Objects: CoStar buildings (`COSTAR_KIT_DIR`) where the kit has them (LA), else buildings found
//! in the height map itself (local peaks standing >= 8 m above the street), so the same scenarios
//! run in Princeton. "The tower" of a scenario is the tallest building object inside the area.
//!
//! Gates come in three kinds:
//! - **entry**: a cost function in `library/*.json`, reusable by any brief (these are the ones we keep);
//! - **sweep**: the same idea at several thresholds, to find the largest cut at full recall;
//! - **what-if**: needs a number fixed per brief (the sunset azimuth), which no Point feature gives
//!   yet. It shows what such a feature would buy; it is not saved.
//!
//! Cache only: a place whose height-map tiles aren't all on disk is skipped, never fetched.
//! `--write` replaces the generated part of docs/library-lab.md and writes the raw rows to
//! `cache/library_lab/rows.json`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use contracts::{Area, Brief, CostFn, DsmGrid, Event, Framing, Goal, Object, Standing};
use golden_core::objects::offset;
use golden_core::horizon::{ray_max_below, Eye};
use golden_core::score::{wrap180, SunTrack, FLANK_LOOK_DEG, GAP_SEARCH_DEG, MIN_IN_VIEW_S};
use golden_core::search::{search_with, site, skyline_need_m, sun_track, Inputs, SearchOutput, EYE_M, SKYLINE_RADIUS_M, SKY_STEP_DEG};

/// Spots compared for recall.
const K: u32 = 10;
/// Search area radius, metres (`--radius` changes it for a one-off run).
const RADIUS_M: f64 = 300.0;
/// A candidate this close to the best (normalised score) counts as near-best.
const NEAR_BEST: f32 = 0.8;
/// An entry is kept if its recall@K is 1.0 everywhere it ran and its median prune is at least this.
const KEEP_PRUNE: f64 = 0.30;

struct Place {
    name: &'static str,
    city: &'static str,
    lat: f64,
    lon: f64,
}

const PLACES: &[Place] = &[
    Place { name: "pershing", city: "LA", lat: 34.0490, lon: -118.2510 },
    Place { name: "bunker-hill", city: "LA", lat: 34.0530, lon: -118.2560 },
    Place { name: "south-park", city: "LA", lat: 34.0430, lon: -118.2620 },
    Place { name: "little-tokyo", city: "LA", lat: 34.0500, lon: -118.2400 },
    Place { name: "nassau-hall", city: "Princeton", lat: 40.3487, lon: -74.6593 },
    Place { name: "witherspoon", city: "Princeton", lat: 40.3540, lon: -74.6640 },
    Place { name: "dinky", city: "Princeton", lat: 40.3420, lon: -74.6590 },
    // round 2: inside the same cached height maps (LA and Princeton far tiles), nothing fetched
    Place { name: "macarthur-park", city: "LA", lat: 34.0590, lon: -118.2770 },
    Place { name: "grad-college", city: "Princeton", lat: 40.3430, lon: -74.6715 },
    // round 3: more places inside the same cached height maps, nothing fetched (skipped if not cached)
    Place { name: "civic-center", city: "LA", lat: 34.0545, lon: -118.2430 },
    Place { name: "fashion-district", city: "LA", lat: 34.0405, lon: -118.2505 },
    Place { name: "chinatown", city: "LA", lat: 34.0625, lon: -118.2385 },
    Place { name: "westlake", city: "LA", lat: 34.0560, lon: -118.2700 },
    Place { name: "echo-park", city: "LA", lat: 34.0700, lon: -118.2600 },
    Place { name: "convention-center", city: "LA", lat: 34.0400, lon: -118.2690 },
    Place { name: "princeton-stadium", city: "Princeton", lat: 40.3460, lon: -74.6440 },
    Place { name: "mercer-west", city: "Princeton", lat: 40.3560, lon: -74.6760 },
    Place { name: "carnegie-north", city: "Princeton", lat: 40.3400, lon: -74.6500 },
];

/// (date, UTC offset in LA, UTC offset in Princeton). The offset only changes printed times.
/// Round 4 added Dec 21, the winter extreme of the sun track (Jun 21 is the summer one).
const DAYS: [(&str, i32, i32); 3] = [("2026-11-14", -8 * 3600, -5 * 3600), ("2026-06-21", -7 * 3600, -4 * 3600), ("2026-12-21", -8 * 3600, -5 * 3600)];

/// What a scenario's goals and gates can refer to at one place and day.
struct Ctx {
    /// `{"id": ...}` of the tallest building in the area.
    tower: String,
    /// `{"id": ...}` of the next tallest, at least `TOWER2_APART_M` from the tower.
    tower2: String,
    /// The sun's azimuth range over the track (sunset back to 20 deg up).
    track_az: (f64, f64),
}

struct Scenario {
    key: &'static str,
    about: &'static str,
    standing: Standing,
    event: Event,
    goals: fn(&Ctx) -> String,
    /// Run on the second tower instead: "the tower" (and every entry's first param) is the next tallest.
    second: bool,
}

fn scenarios() -> Vec<Scenario> {
    let plain = |key, about, standing, goals: fn(&Ctx) -> String| Scenario { key, about, standing, event: Event::Sunset, goals, second: false };
    vec![
        plain("clear", "sun in view down to the horizon, roof or street", Standing::Any, |_| r#"[{"use":"sun_clear"}]"#.into()),
        plain("between", "sun sets into a gap between buildings, roof or street", Standing::Any, |_| r#"[{"use":"sun_between"}]"#.into()),
        plain("behind", "sun drops behind a building, roof or street", Standing::Any, |_| r#"[{"use":"sun_behind"}]"#.into()),
        plain("street-between", "street level, sun sets into a gap", Standing::Ground, |_| r#"[{"use":"sun_between"}]"#.into()),
        plain("street-behind", "street level, sun drops behind a building", Standing::Ground, |_| r#"[{"use":"sun_behind"}]"#.into()),
        plain("tower-and-sunset", "sunset in view, and the tower must fill part of the frame (fills is a must)", Standing::Any, |c| {
            format!(r#"[{{"use":"sun_clear"}},{{"use":"fills","args":{{"a":{t}}},"must":true}}]"#, t = c.tower)
        }),
        plain("soft-tower", "as tower-and-sunset, but fills only weighted 0.5 (the agent's default shape)", Standing::Any, |c| {
            format!(r#"[{{"use":"sun_clear"}},{{"use":"fills","args":{{"a":{t}}},"weight":0.5}}]"#, t = c.tower)
        }),
        plain("sun-by-tower", "the sun must set within 15 deg of the tower; clear sky weighted 0.5", Standing::Any, |c| {
            format!(
                r#"[{{"def":{{"name":"sun_by_tower","about":"Sun sets within 15 deg of the tower",
                    "score":{{"op":"bump","x":{{"f":"sun_bearing_diff","obj":{t}}},"at":0,"width":15}}}},"must":true,"weight":1}},
                    {{"use":"sun_clear","weight":0.5}}]"#,
                t = c.tower
            )
        }),
        plain("tower-framing-sunset", "the tower must fill part of the frame and the sun must set within 15 deg of it", Standing::Any, |c| {
            format!(
                r#"[{{"use":"sun_clear"}},{{"use":"fills","args":{{"a":{t}}},"must":true}},
                    {{"def":{{"name":"sun_by_tower","about":"Sun sets within 15 deg of the tower",
                    "score":{{"op":"bump","x":{{"f":"sun_bearing_diff","obj":{t}}},"at":0,"width":15}}}},"must":true}}]"#,
                t = c.tower
            )
        }),
        Scenario {
            event: Event::Sunrise,
            ..plain("sunrise-by-tower", "sunrise: the sun must come up within 15 deg of the tower; clear sky weighted 0.5", Standing::Any, |c| {
                format!(
                    r#"[{{"def":{{"name":"sun_by_tower","about":"Sun rises within 15 deg of the tower",
                        "score":{{"op":"bump","x":{{"f":"sun_bearing_diff","obj":{t}}},"at":0,"width":15}}}},"must":true,"weight":1}},
                        {{"use":"sun_clear","weight":0.5}}]"#,
                    t = c.tower
                )
            })
        },
        // ---- round 2 shot classes ----
        plain("silhouette", "the sun drops behind the tower: sun_behind, with the sun within 5 deg of the tower (a must)", Standing::Any, |c| {
            format!(
                r#"[{{"def":{{"name":"sun_behind_tower","about":"Sun drops behind the tower",
                    "score":{{"op":"mul","of":[{{"op":"bump","x":{{"f":"sun_bearing_diff","obj":{t}}},"at":0,"width":5}},{{"f":"sun_behind"}}]}}}},"must":true,"weight":1}}]"#,
                t = c.tower
            )
        }),
        plain("between-towers", "the sun sets between the two tallest buildings, both in one frame, in front of you (a must); clear sky weighted 0.5", Standing::Any, |c| {
            format!(
                r#"[{{"def":{{"name":"sun_between_towers","about":"Sun sets between the two towers",
                    "score":{{"op":"and","of":[
                        {{"op":"lt","x":{{"op":"mul","of":[{{"f":"sun_bearing_diff","obj":{t}}},{{"f":"sun_bearing_diff","obj":{u}}}]}},"than":{{"num":0}}}},
                        {{"op":"gt","x":{{"op":"bump","x":{{"f":"sun_bearing_diff","obj":{t}}},"at":0,"width":90}},"than":{{"num":0}}}},
                        {{"f":"in_frame","a":{t},"b":{u}}}]}}}},"must":true,"weight":1}},
                    {{"use":"sun_clear","weight":0.5}}]"#,
                t = c.tower,
                u = c.tower2
            )
        }),
        plain("looking-up", "street level, low angle: the tower looms over you (25+ deg tall, best 35+) and in view (a must); sun in view weighted 0.5", Standing::Ground, |c| {
            format!(
                r#"[{{"def":{{"name":"tower_overhead","about":"The tower looms overhead",
                    "score":{{"op":"mul","of":[{{"op":"ramp","x":{{"f":"size_deg","obj":{t}}},"lo":25,"hi":35}},{{"f":"visible","obj":{t}}}]}}}},"must":true,"weight":1}},
                    {{"use":"sun_clear","weight":0.5}}]"#,
                t = c.tower
            )
        }),
        plain(
            "rooftop-skyline",
            "on a roof, the tower reads as skyline (2-20 deg tall, best 4-12, in view; a must) with the sun setting within 15 deg of it (a must)",
            Standing::Roof,
            |c| {
                format!(
                    r#"[{{"def":{{"name":"skyline","about":"The tower reads as skyline",
                        "score":{{"op":"mul","of":[{{"op":"and","of":[{{"op":"ramp","x":{{"f":"size_deg","obj":{t}}},"lo":2,"hi":4}},
                            {{"op":"ramp","x":{{"f":"size_deg","obj":{t}}},"lo":20,"hi":12}}]}},{{"f":"visible","obj":{t}}}]}}}},"must":true,"weight":0.5}},
                        {{"def":{{"name":"sun_by_tower","about":"Sun sets within 15 deg of the tower",
                        "score":{{"op":"bump","x":{{"f":"sun_bearing_diff","obj":{t}}},"at":0,"width":15}}}},"must":true,"weight":1}},
                        {{"use":"sun_clear","weight":0.5}}]"#,
                    t = c.tower
                )
            },
        ),
        plain("open-view", "an open view: the sun stays in view to within 1 deg of the horizon (a must), ranked on clear sky", Standing::Any, |_| {
            r#"[{"def":{"name":"open_view","about":"Sun in view to the horizon","score":{"op":"lt","x":{"f":"sun_hidden_elev"},"than":{"num":1}}},"must":true},
                {"use":"sun_clear"}]"#
                .into()
        }),
        // ---- round 3 shot classes ----
        sunrise(plain("sunrise-silhouette", "dawn silhouette: the sun comes up from behind the tower (sun_behind, sun within 5 deg of the tower; a must)", Standing::Any, |c| {
            format!(
                r#"[{{"def":{{"name":"sun_behind_tower","about":"Sun rises from behind the tower",
                    "score":{{"op":"mul","of":[{{"op":"bump","x":{{"f":"sun_bearing_diff","obj":{t}}},"at":0,"width":5}},{{"f":"sun_behind"}}]}}}},"must":true,"weight":1}}]"#,
                t = c.tower
            )
        })),
        sunrise(plain(
            "sunrise-between-towers",
            "the sun rises between the two tallest buildings, both in one frame, in front of you (a must); clear sky weighted 0.5",
            Standing::Any,
            |c| {
                format!(
                    r#"[{{"def":{{"name":"sun_between_towers","about":"Sun rises between the two towers",
                    "score":{{"op":"and","of":[
                        {{"op":"lt","x":{{"op":"mul","of":[{{"f":"sun_bearing_diff","obj":{t}}},{{"f":"sun_bearing_diff","obj":{u}}}]}},"than":{{"num":0}}}},
                        {{"op":"gt","x":{{"op":"bump","x":{{"f":"sun_bearing_diff","obj":{t}}},"at":0,"width":90}},"than":{{"num":0}}}},
                        {{"f":"in_frame","a":{t},"b":{u}}}]}}}},"must":true,"weight":1}},
                    {{"use":"sun_clear","weight":0.5}}]"#,
                    t = c.tower,
                    u = c.tower2
                )
            },
        )),
        plain("lit-tower", "golden light on the tower: the setting sun behind you (within 30 deg of straight behind), the tower in view (a must); fills weighted 0.5", Standing::Any, lit_tower),
        sunrise(plain("sunrise-lit-tower", "morning light on the tower: the rising sun behind you (within 30 deg of straight behind), the tower in view (a must); fills weighted 0.5", Standing::Any, lit_tower)),
        sunrise(plain("sunrise-clear", "sunrise in view from the horizon up, roof or street (sun only)", Standing::Any, |_| r#"[{"use":"sun_clear"}]"#.into())),
        sunrise(plain("sunrise-open-view", "the library's open_horizon at sunrise: in view from within 1 deg of the horizon (a must), ranked on clear sky (sun only)", Standing::Any, |_| {
            r#"[{"use":"open_horizon","must":true,"weight":1}]"#.into()
        })),
        // ---- round 4: the same entries on another building, and the sun seeds at sunrise ----
        second(plain("sun-by-tower2", "as sun-by-tower, on the second tallest building", Standing::Any, |c| {
            format!(
                r#"[{{"def":{{"name":"sun_by_tower","about":"Sun sets within 15 deg of the tower",
                    "score":{{"op":"bump","x":{{"f":"sun_bearing_diff","obj":{t}}},"at":0,"width":15}}}},"must":true,"weight":1}},
                    {{"use":"sun_clear","weight":0.5}}]"#,
                t = c.tower
            )
        })),
        second(plain("silhouette-tower2", "as silhouette, on the second tallest building", Standing::Any, |c| {
            format!(
                r#"[{{"def":{{"name":"sun_behind_tower","about":"Sun drops behind the tower",
                    "score":{{"op":"mul","of":[{{"op":"bump","x":{{"f":"sun_bearing_diff","obj":{t}}},"at":0,"width":5}},{{"f":"sun_behind"}}]}}}},"must":true,"weight":1}}]"#,
                t = c.tower
            )
        })),
        second(plain("lit-tower2", "as lit-tower, on the second tallest building", Standing::Any, lit_tower)),
        sunrise(plain("sunrise-between", "the sun rises out of a gap between buildings, roof or street (sun only)", Standing::Any, |_| r#"[{"use":"sun_between"}]"#.into())),
        sunrise(plain("sunrise-behind", "the sun comes up from behind a building, roof or street (sun only)", Standing::Any, |_| r#"[{"use":"sun_behind"}]"#.into())),
        sunrise(plain("sunrise-wide-gap", "the library's sun_wide_gap at sunrise (a must; sun only)", Standing::Any, |_| {
            r#"[{"use":"sun_wide_gap","must":true,"weight":1}]"#.into()
        })),
        plain("wide-gap", "the library's sun_wide_gap: the sun sets into a wide opening (6-20 deg) and stays in view in it (a must; sun only)", Standing::Any, |_| {
            r#"[{"use":"sun_wide_gap","must":true,"weight":1}]"#.into()
        }),
        // ---- round 5 shot classes: each is a library entry's score without its must (see `shot`) ----
        plain("foreground-sunset", "a building in front of the tower, the sun setting behind the tower (within 20 deg; a must); clear sky weighted 0.5", Standing::Any, |c| {
            shot_clear("in_front_toward_sunset", &building_then_tower(c))
        }),
        sunrise(plain("foreground-sunrise", "a building in front of the tower, the sun rising behind the tower (within 20 deg; a must); clear sky weighted 0.5", Standing::Any, |c| {
            shot_clear("in_front_toward_sunrise", &building_then_tower(c))
        })),
        plain("stacked-sunset", "a building stacked in front of the tower (same bearing), the sun setting right behind the stack (a must); clear sky weighted 0.5", Standing::Any, |c| {
            shot_clear("lined_up_toward_sunset", &building_then_tower(c))
        }),
        sunrise(plain("stacked-sunrise", "a building stacked in front of the tower (same bearing), the sun rising right behind the stack (a must); clear sky weighted 0.5", Standing::Any, |c| {
            shot_clear("lined_up_toward_sunrise", &building_then_tower(c))
        })),
        plain("sunburst", "street level: the tower looms over you (20+ deg tall) with the evening sun at its edge (within 10 deg; a must)", Standing::Ground, |c| {
            format!("[{}]", shot("sunburst_up", &towers(c)))
        }),
        sunrise(plain("sunburst-sunrise", "street level: the tower looms over you (20+ deg tall) with the rising sun at its edge (within 10 deg; a must)", Standing::Ground, |c| {
            format!("[{}]", shot("sunburst_up_sunrise", &towers(c)))
        })),
        plain("skyline-sunset", "on a roof: the tower reads as skyline (2-20 deg tall, best 4-12, in view) with the sun setting within 15 deg of it, as one goal (a must); clear sky weighted 0.5", Standing::Roof, |c| {
            shot_clear("skyline_at_sunset", &towers(c))
        }),
    ]
}

/// The same scenario on the second tower.
fn second(s: Scenario) -> Scenario {
    Scenario { second: true, ..s }
}

/// A library entry's score as an inline must goal (weight 1), its params bound to `objs` in order, its
/// own `must` left out: the shot class as a brief would write it without the entry's speed gate. The
/// entry standing in for it (`replaces: Some(0)`) then has the same score, so recall is exact.
fn shot(name: &str, objs: &[String]) -> String {
    let path = repo().join("library").join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let params: Vec<String> = v["params"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    let bound: Vec<(String, serde_json::Value)> =
        params.iter().zip(objs).map(|(p, o)| (p.clone(), serde_json::from_str(o).expect("object ref"))).collect();
    let mut score = v["score"].take();
    bind(&mut score, &bound);
    serde_json::json!({ "def": { "name": format!("{name}_shot"), "about": v["about"], "score": score }, "must": true, "weight": 1 }).to_string()
}

/// `shot` plus clear sky weighted 0.5.
fn shot_clear(name: &str, objs: &[String]) -> String {
    format!(r#"[{},{{"use":"sun_clear","weight":0.5}}]"#, shot(name, objs))
}

/// The same scenario at sunrise.
fn sunrise(s: Scenario) -> Scenario {
    Scenario { event: Event::Sunrise, ..s }
}

/// Low sun on the tower: the sun behind you within 30 deg of straight behind (|sun_bearing_diff| near
/// 180), the tower in view; fills weighted 0.5. Same goals at sunset and sunrise.
fn lit_tower(c: &Ctx) -> String {
    format!(
        r#"[{{"def":{{"name":"tower_lit","about":"The low sun lights the tower",
            "score":{{"op":"mul","of":[
                {{"op":"bump","x":{{"op":"max","of":[{{"f":"sun_bearing_diff","obj":{t}}},{{"op":"mul","of":[{{"f":"sun_bearing_diff","obj":{t}}},{{"num":-1}}]}}]}},"at":180,"width":30}},
                {{"f":"visible","obj":{t}}}]}}}},"must":true,"weight":1}},
            {{"use":"fills","args":{{"a":{t}}},"weight":0.5}}]"#,
        t = c.tower
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GateKind {
    Entry,
    Sweep,
    WhatIf,
    WhatIfRay,
}

impl GateKind {
    fn name(self) -> &'static str {
        match self {
            GateKind::Entry => "entry",
            GateKind::Sweep => "sweep",
            GateKind::WhatIf => "what-if",
            GateKind::WhatIfRay => "ray what-if",
        }
    }
}

struct Gate {
    name: String,
    kind: GateKind,
    /// The goal to add, as JSON (a `must`).
    goal: Box<dyn Fn(&Ctx, &[CostFn]) -> String + Send + Sync>,
    /// Scenario keys it is tried on.
    on: Vec<&'static str>,
    /// `Some(i)`: the gate replaces the scenario's goal `i` (a shot-class entry whose own cheap `must`
    /// is the speed gate) instead of being added, and its prune is measured with that `must` on its
    /// own. Recall is then against the same scenario without that `must`.
    replaces: Option<usize>,
    /// (a) a speed gate, implied by the score: judged by recall and prune. (b) a shot class, the
    /// user's intent, meant to change the result: judged by prune and a sane result.
    class: char,
    /// The objects an entry's params are bound to, in order.
    objs: fn(&Ctx) -> Vec<String>,
}

/// The tower, then the second tower (what an entry's params are bound to by default).
fn towers(c: &Ctx) -> Vec<String> {
    vec![c.tower.clone(), c.tower2.clone()]
}

/// Any building within 400 m of the tower, then the tower (round 5 foreground and stacked shots: a fixed
/// second tower almost never lines up with the tower, so "some building in front of it" is the real brief).
fn building_then_tower(c: &Ctx) -> Vec<String> {
    vec![format!(r#"{{"kind":"building","near":{},"within_m":400}}"#, c.tower), c.tower.clone()]
}

/// A library entry as a must goal, its params bound to the tower, then the second tower.
fn entry(name: &'static str, on: &[&'static str]) -> Gate {
    entry_on(name, on, towers)
}

/// A library entry as a must goal, its params bound to `objs(ctx)` in order.
fn entry_on(name: &'static str, on: &[&'static str], objs: fn(&Ctx) -> Vec<String>) -> Gate {
    Gate {
        name: name.into(),
        kind: GateKind::Entry,
        goal: Box::new(move |c, lib| {
            let params = lib.iter().find(|f| f.name == name).map(|f| f.params.clone()).unwrap_or_default();
            let o = objs(c);
            let args: Vec<String> = params.iter().zip(&o).map(|(p, x)| format!(r#""{p}":{x}"#)).collect();
            format!(r#"{{"use":"{name}","args":{{{}}},"must":true,"weight":1}}"#, args.join(","))
        }),
        on: on.to_vec(),
        replaces: None,
        class: 'a',
        objs,
    }
}

/// An inline gate: `def(ctx)` is the cost function's JSON.
fn inline(name: String, kind: GateKind, on: &[&'static str], def: impl Fn(&Ctx) -> String + Send + Sync + 'static) -> Gate {
    Gate { name, kind, goal: Box::new(move |c, _| format!(r#"{{"def":{},"must":true}}"#, def(c))), on: on.to_vec(), replaces: None, class: 'a', objs: towers }
}

/// An entry's own `must` on its own, as a must goal with its params bound to `objs(c)`: what the
/// entry drops before its score is computed (its prune when it stands in for a scenario's goal).
fn must_alone(name: &str, c: &Ctx, lib: &[CostFn], objs: fn(&Ctx) -> Vec<String>) -> Result<Goal, String> {
    let f = lib.iter().find(|f| f.name == name).ok_or(format!("no library entry {name}"))?;
    let must = f.must.as_ref().ok_or(format!("{name} has no must"))?;
    let mut v = serde_json::to_value(must).map_err(|e| e.to_string())?;
    let objs: Vec<(String, serde_json::Value)> = f
        .params
        .iter()
        .zip(objs(c))
        .map(|(p, o)| Ok((p.clone(), serde_json::from_str(&o).map_err(|e| e.to_string())?)))
        .collect::<Result<_, String>>()?;
    bind(&mut v, &objs);
    let def = serde_json::json!({ "def": { "name": format!("{name}_must"), "about": "the entry's must", "score": v }, "must": true });
    serde_json::from_value(def).map_err(|e| format!("{name}: {e}"))
}

/// Replace every object slot (`"obj"`, `"a"`, `"b"`) naming a param with that param's object.
fn bind(v: &mut serde_json::Value, objs: &[(String, serde_json::Value)]) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                let named = matches!(k.as_str(), "obj" | "a" | "b").then(|| x.as_str()).flatten();
                match named.and_then(|s| objs.iter().find(|(p, _)| p == s)) {
                    Some((_, o)) => *x = o.clone(),
                    None => bind(x, objs),
                }
            }
        }
        serde_json::Value::Array(xs) => xs.iter_mut().for_each(|x| bind(x, objs)),
        _ => {}
    }
}

fn gates() -> Vec<Gate> {
    let skyline_scenes: &[&str] = &["between", "street-between", "behind", "street-behind"];
    // a size or visibility test on the tower, to see which part of fills does the pruning
    let tower_test = |name: &str, score: &'static str| {
        inline(name.into(), GateKind::Sweep, &["tower-and-sunset"], move |c| {
            format!(r#"{{"name":"x","about":"sweep","score":{}}}"#, score.replace("TOWER", &c.tower))
        })
    };
    let mut out = vec![
        entry("tower_framable", &["tower-and-sunset", "soft-tower", "tower-framing-sunset"]),
        entry("tower_toward_sunset", &["sun-by-tower", "tower-framing-sunset", "silhouette", "rooftop-skyline"]),
        entry("tower_toward_sunrise", &["sunrise-by-tower", "sunrise-silhouette"]),
        entry("tower_framed_toward_sunset", &["tower-framing-sunset"]),
        Gate { replaces: Some(0), ..entry("sunset_beside", &["sun-by-tower", "sun-by-tower2"]) },
        Gate { replaces: Some(0), ..entry("sunrise_beside", &["sunrise-by-tower"]) },
        Gate { replaces: Some(1), ..entry("tower_fills", &["tower-and-sunset"]) },
        // round 2: shot-class entries whose own must is the speed gate
        Gate { replaces: Some(0), ..entry("sunset_behind", &["silhouette", "silhouette-tower2"]) },
        Gate { replaces: Some(0), ..entry("sunset_between_two", &["between-towers"]) },
        Gate { replaces: Some(0), ..entry("towers_over", &["looking-up"]) },
        Gate { replaces: Some(0), ..entry("skyline_view", &["rooftop-skyline"]) },
        // round 3: sunrise mirrors and the low sun on the tower
        Gate { replaces: Some(0), ..entry("sunrise_behind", &["sunrise-silhouette"]) },
        Gate { replaces: Some(0), ..entry("sunrise_between_two", &["sunrise-between-towers"]) },
        Gate { replaces: Some(0), ..entry("tower_lit_at_sunset", &["lit-tower", "lit-tower2"]) },
        Gate { replaces: Some(0), ..entry("tower_lit_at_sunrise", &["sunrise-lit-tower"]) },
        // round 5: foreground, stacked, sunburst and skyline shots with the sun
        Gate { replaces: Some(0), ..entry_on("in_front_toward_sunset", &["foreground-sunset"], building_then_tower) },
        Gate { replaces: Some(0), ..entry_on("in_front_toward_sunrise", &["foreground-sunrise"], building_then_tower) },
        Gate { replaces: Some(0), ..entry_on("lined_up_toward_sunset", &["stacked-sunset"], building_then_tower) },
        Gate { replaces: Some(0), ..entry_on("lined_up_toward_sunrise", &["stacked-sunrise"], building_then_tower) },
        Gate { replaces: Some(0), ..entry("sunburst_up", &["sunburst"]) },
        Gate { replaces: Some(0), ..entry("sunburst_up_sunrise", &["sunburst-sunrise"]) },
        Gate { replaces: Some(0), ..entry("skyline_at_sunset", &["skyline-sunset"]) },
        // why framing gates prune so little: the two halves of the size range, and the line of sight
        tower_test("tower_over_2_deg", r#"{"op":"gt","x":{"f":"size_deg","obj":TOWER},"than":{"num":2}}"#),
        tower_test("tower_under_40_deg", r#"{"op":"lt","x":{"f":"size_deg","obj":TOWER},"than":{"num":40}}"#),
        tower_test("tower_in_view", r#"{"op":"gt","x":{"f":"visible","obj":TOWER},"than":{"num":0}}"#),
        // Standing on a roof: a user constraint, tried as a precheck for the sun scenes.
        Gate {
            class: 'b',
            ..inline("on_roof".into(), GateKind::Sweep, &["clear", "between", "behind", "tower-and-sunset", "soft-tower", "sun-by-tower"], |_| {
                r#"{"name":"x","about":"on a roof","score":{"f":"roof"}}"#.into()
            })
        },
    ];
    // How tall must some building look (anywhere, or to the west) before a gap or a blocker is possible?
    let looms = |deg: f64, west: bool| {
        let obj = r#"{"kind":"building"}"#;
        let tall = format!(r#"{{"op":"gt","x":{{"f":"size_deg","obj":{obj}}},"than":{{"num":{deg}}}}}"#);
        let west_of = format!(r#"{{"op":"gt","x":{{"op":"bump","x":{{"f":"bearing","obj":{obj}}},"at":270,"width":70}},"than":{{"num":0}}}}"#);
        let score = if west { format!(r#"{{"op":"and","of":[{tall},{west_of}]}}"#) } else { tall };
        format!(r#"{{"name":"x","about":"sweep","score":{score}}}"#)
    };
    for deg in [3.0, 8.0] {
        let any = looms(deg, false);
        out.push(inline(format!("looms_{deg}"), GateKind::Sweep, skyline_scenes, move |_| any.clone()));
        let west = looms(deg, true);
        out.push(inline(format!("looms_west_{deg}"), GateKind::Sweep, skyline_scenes, move |_| west.clone()));
    }
    // What a Point-tier sunset azimuth would buy: the same gates aimed at this day's sun track
    // (+-18 deg, the gap search's reach), a number fixed per brief here.
    out.push(inline("sunward_looms_3".into(), GateKind::WhatIf, skyline_scenes, |c| {
        let (lo, hi) = c.track_az;
        let (at, width) = ((lo + hi) / 2.0, (hi - lo) / 2.0 + 18.0);
        format!(
            r#"{{"name":"x","about":"a building toward the sun looks at least 3 deg tall",
                "score":{{"op":"and","of":[
                    {{"op":"gt","x":{{"f":"size_deg","obj":{{"kind":"building"}}}},"than":{{"num":3}}}},
                    {{"op":"gt","x":{{"op":"bump","x":{{"f":"bearing","obj":{{"kind":"building"}}}},"at":{at:.1},"width":{width:.1}}},"than":{{"num":0}}}}]}}}}"#
        )
    }));
    out.push(inline("tower_on_sun_track".into(), GateKind::WhatIf, &["sun-by-tower", "sunrise-by-tower", "tower-framing-sunset", "silhouette", "rooftop-skyline", "sunrise-silhouette"], |c| {
        let (lo, hi) = c.track_az;
        let (at, width) = ((lo + hi) / 2.0, (hi - lo) / 2.0 + 15.0);
        format!(
            r#"{{"name":"x","about":"the tower is within 15 deg of this day's sun track",
                "score":{{"op":"gt","x":{{"op":"bump","x":{{"f":"bearing","obj":{t}}},"at":{at:.1},"width":{width:.1}}},"than":{{"num":0}}}}}}"#,
            t = c.tower
        )
    }));
    // The lit-tower mirror: the tower within 30 deg of straight opposite this day's sun track.
    out.push(inline("tower_opposite_sun_track".into(), GateKind::WhatIf, &["lit-tower", "sunrise-lit-tower"], |c| {
        let (lo, hi) = c.track_az;
        let (at, width) = (((lo + hi) / 2.0 + 180.0) % 360.0, (hi - lo) / 2.0 + 30.0);
        format!(
            r#"{{"name":"x","about":"the tower is within 30 deg of straight opposite this day's sun track",
                "score":{{"op":"or","of":[
                    {{"op":"gt","x":{{"op":"bump","x":{{"f":"bearing","obj":{t}}},"at":{at:.1},"width":{width:.1}}},"than":{{"num":0}}}},
                    {{"op":"gt","x":{{"op":"bump","x":{{"f":"bearing","obj":{t}}},"at":{a2:.1},"width":{width:.1}}},"than":{{"num":0}}}},
                    {{"op":"gt","x":{{"op":"bump","x":{{"f":"bearing","obj":{t}}},"at":{a3:.1},"width":{width:.1}}},"than":{{"num":0}}}}]}}}}"#,
            t = c.tower,
            a2 = at + 360.0,
            a3 = at - 360.0
        )
    }));
    out
}

// ---- ray what-ifs ----------------------------------------------------------------------
//
// Not expressible yet: one ray through the height map at the azimuth where the sun is at a given
// elevation (a possible Ray-tier feature). Measured here with core's public horizon functions, at
// every heat-grid candidate and every ungated top spot, without running a gated search: a must
// never rescales survivors, so the gated picks are the ungated ones iff every top spot passes.

struct RayGate {
    name: &'static str,
    on: &'static [&'static str],
    /// Does a standpoint with this eye pass, given the day's sun track and the grid's top height?
    pass: fn(&DsmGrid, Eye, &SunTrack, f64) -> bool,
}

/// Rays in one fast-pass skyline trace: the window `search` traces (the track's azimuths plus room for
/// a gap's sides), at `SKY_STEP_DEG`.
fn skyline_rays(t: &SunTrack) -> usize {
    let center = t.samples.first().map_or(0.0, |s| s.az_deg);
    let (lo, hi) = t.samples.iter().map(|s| wrap180(s.az_deg - center)).fold((0.0f64, 0.0f64), |(lo, hi), r| (lo.min(r), hi.max(r)));
    let side = GAP_SEARCH_DEG + FLANK_LOOK_DEG + SKY_STEP_DEG;
    (((hi + side) - (lo - side)) / SKY_STEP_DEG).ceil() as usize + 1
}

thread_local! {
    /// Rays marched by the ray what-ifs on this thread (to report rays per candidate).
    static RAYS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn ray(grid: &DsmGrid, eye: Eye, az: f64, top: f64) -> f64 {
    RAYS.with(|r| r.set(r.get() + 1));
    ray_max_below(grid, eye, az, SKYLINE_RADIUS_M, top).elev_deg
}

/// Does the sun come into view (for `MIN_IN_VIEW_S` in a row) before it is `top_deg` up, scanning
/// from the flat sunset as `score::look` does? That is `hidden_elev_deg < top_deg`, which `sun_between`
/// needs (`in_view` is 0 from 6 deg) and the best `sun_clear` spots have. Exact: rays at the fast pass's
/// own skyline azimuths (same window, same 0.5 deg steps, same interpolation as `score::blocks`), only
/// those the track crosses below `top_deg`.
fn seen_below(g: &DsmGrid, e: Eye, t: &SunTrack, top: f64, top_deg: f64) -> bool {
    let Some(first) = t.samples.first() else { return true };
    let center = first.az_deg;
    let lo = t.samples.iter().map(|s| wrap180(s.az_deg - center)).fold(0.0f64, f64::min);
    let lo_deg = lo - (GAP_SEARCH_DEG + FLANK_LOOK_DEG + SKY_STEP_DEG);
    let need = (MIN_IN_VIEW_S as f64 / t.step_s.max(1) as f64).ceil().max(1.0) as usize;
    let mut sky: Vec<(i64, f64)> = Vec::new();
    let mut elev_at = |i: i64| -> f64 {
        if let Some(&(_, v)) = sky.iter().find(|(j, _)| *j == i) {
            return v;
        }
        let v = ray(g, e, center + lo_deg + i as f64 * SKY_STEP_DEG, top);
        sky.push((i, v));
        v
    };
    let last = t.samples.len() - 1;
    let mut run = 0;
    for (k, s) in t.samples.iter().enumerate() {
        if run == 0 && s.elev_deg >= top_deg {
            return false; // no run started below top_deg
        }
        let x = (wrap180(s.az_deg - center) - lo_deg) / SKY_STEP_DEG;
        let i = x.floor() as i64;
        let sky_elev = elev_at(i) + (elev_at(i + 1) - elev_at(i)) * (x - i as f64);
        run = if sky_elev > s.elev_deg.max(0.0) { 0 } else { run + 1 };
        if run > 0 && (run >= need || k == last) {
            return t.samples[k + 1 - run].elev_deg < top_deg;
        }
    }
    false
}

const RAY_GATES: &[RayGate] = &[
    RayGate { name: "ray_sun_seen_below_6", on: &["clear", "between", "street-between", "open-view", "sunrise-clear", "sunrise-open-view", "wide-gap", "sunrise-between", "sunrise-wide-gap"], pass: |g, e, t, top| seen_below(g, e, t, top, 6.0) },
    // open-view's must is `sun_hidden_elev < 1`: this is that test, exactly, before any skyline
    RayGate { name: "ray_sun_seen_below_1", on: &["open-view", "sunrise-open-view"], pass: |g, e, t, top| seen_below(g, e, t, top, 1.0) },
    RayGate { name: "ray_sun_seen_below_3", on: &["clear", "between", "street-between"], pass: |g, e, t, top| seen_below(g, e, t, top, 3.0) },
    RayGate {
        name: "ray_blocked_at_sunset",
        on: &["behind", "street-behind"],
        pass: |g, e, t, top| {
            let az = t.samples.first().map_or(270.0, |s| s.az_deg);
            ray(g, e, az, top) > -0.25
        },
    },
];

fn eye_at(grid: &DsmGrid, lat: f64, lon: f64, eye_m: f64) -> Eye {
    let (col, row) = grid.cell_of(lat, lon);
    Eye { col, row, eye_m }
}

/// The heat grid's candidates that were searched: (lat, lon, eye_m).
fn heat_candidates(grid: &DsmGrid, out: &SearchOutput) -> Vec<(f64, f64, f64)> {
    let h = &out.heat;
    (0..h.values.len())
        .filter(|&i| h.values[i].is_some())
        .filter_map(|i| {
            let (col, row) = ((i as u32 % h.width) as f64, (i as u32 / h.width) as f64);
            let east = (col + 0.5 - f64::from(h.width) / 2.0) * h.cell_m;
            let north = (f64::from(h.height) / 2.0 - row - 0.5) * h.cell_m;
            let (lat, lon) = offset((h.center_lat, h.center_lon), east, north);
            site(grid, lat, lon).map(|s| (lat, lon, s.ground_m + EYE_M))
        })
        .collect()
}

// ---- one measurement ------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Row {
    place: String,
    city: String,
    date: String,
    scenario: String,
    gate: String,
    kind: String,
    class: char,
    /// Skyline traces in the ungated and the gated search (candidates that reach the first Skyline step).
    traces_base: usize,
    traces_gated: usize,
    gated_spots: usize,
    /// Ray what-ifs: rays marched per candidate, and rays in one fast-pass skyline (what a trace costs).
    rays: f64,
    sky_rays: usize,
    /// Heat-grid candidates searched, and how many the gate keeps.
    searched: usize,
    kept: usize,
    prune: f64,
    recall: f64,
    /// (survivors, total) of the ungated near-best candidates.
    near_best: (usize, usize),
    /// The ungated top 10 had this many spots (fewer if few scored above 0).
    top: usize,
    t_base_ms: f64,
    t_gated_ms: f64,
}

/// One ungated scenario run: is the shot class sane here (does it find spots), and what does it cost?
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Shot {
    place: String,
    scenario: String,
    searched: usize,
    traces: usize,
    spots: usize,
    ms: f64,
}

struct Run {
    out: SearchOutput,
    took: Duration,
}

/// Where and when a search runs.
struct At<'a> {
    place: &'a Place,
    radius_m: f64,
    date: &'a str,
    offset_s: i32,
}

fn run(grid: &DsmGrid, at: &At, (standing, event): (Standing, Event), goals: Vec<Goal>, max_spots: u32, inputs: &Inputs) -> Result<Run, String> {
    let At { place, radius_m, date, offset_s } = *at;
    let brief = Brief {
        text: String::new(),
        area: Area { lat: place.lat, lon: place.lon, radius_m, name: None },
        date: date.into(),
        tz: contracts::search::DEFAULT_TZ.into(),
        event,
        standing,
        framing: Framing::Clear,
        max_spots,
        goals,
        lens_deg: contracts::DEFAULT_LENS_DEG,
    };
    let t = Instant::now();
    let out = search_with(grid, &brief, inputs, offset_s).map_err(|e| e.to_string())?;
    Ok(Run { out, took: t.elapsed() })
}

/// Skyline traces a search made: candidates left when the funnel reaches its first Skyline-tier gate,
/// or all survivors if no gate is Skyline-tier (every scenario here ranks on a sun goal). Gate stages
/// run cheapest tier first. `skyline` names the search's Skyline-tier goals (`skyline_goals`).
fn traces(out: &SearchOutput, skyline: &[String]) -> usize {
    let mut n = out.stages.get(1).map_or(0, |s| s.n);
    for st in out.stages.iter().skip(2) {
        if st.goal.as_ref().is_some_and(|g| skyline.contains(g)) {
            break;
        }
        n = st.n;
    }
    n as usize
}

/// The goals that need the skyline: their cost function uses a `sun_*` feature (all Skyline tier).
fn skyline_goals(goals: &[Goal], lib: &[CostFn]) -> Vec<String> {
    let uses_sun = |f: &CostFn| serde_json::to_string(f).is_ok_and(|j| j.contains(r#""f":"sun_"#));
    goals
        .iter()
        .filter(|g| match &g.cost {
            contracts::GoalFn::Use { name, .. } => lib.iter().find(|f| &f.name == name).is_some_and(uses_sun),
            contracts::GoalFn::Def(f) => uses_sun(f),
        })
        .map(|g| g.name().to_string())
        .collect()
}

fn parse_goals(json: &str) -> Result<Vec<Goal>, String> {
    serde_json::from_str(json).map_err(|e| format!("bad goals JSON: {e}\n{json}"))
}

/// `LAB_DEBUG=1`: each search's funnel stages and picked spots with their goal scores.
fn print_spots(outs: &[(&str, &SearchOutput)]) {
    for (name, o) in outs {
        let stages: Vec<(&str, u32)> = o.stages.iter().map(|s| (s.label.as_str(), s.n as u32)).collect();
        println!("      {name}: {} searched, stages {stages:?}", o.searched);
        for sp in &o.spots {
            let scores: Vec<(&str, f64)> = sp.scores.iter().map(|x| (x.goal.as_str(), x.score as f64)).collect();
            println!("        {} {:.5},{:.5} {scores:?}", sp.id, sp.lat, sp.lon);
        }
    }
}

fn ids(o: &SearchOutput) -> Vec<&str> {
    o.spots.iter().map(|s| s.id.as_str()).collect()
}

// ---- objects --------------------------------------------------------------------------

/// Fractional cell (col, row) to lat/lon, inverse of `DsmGrid::cell_of`.
fn latlon_of(grid: &DsmGrid, col: f64, row: f64) -> (f64, f64) {
    let m = &grid.meta;
    let east = (col - f64::from(m.width) / 2.0) * m.cell_m;
    let north = (f64::from(m.height) / 2.0 - row) * m.cell_m;
    offset((m.center_lat, m.center_lon), east, north)
}

/// (min, max, argmax) of heights in the cell square [c0, c1) x [r0, r1).
fn square(grid: &DsmGrid, c0: i64, r0: i64, c1: i64, r1: i64) -> Option<(f32, f32, (i64, i64))> {
    let (w, h) = (i64::from(grid.meta.width), i64::from(grid.meta.height));
    let mut acc: Option<(f32, f32, (i64, i64))> = None;
    for r in r0.max(0)..r1.min(h) {
        for c in c0.max(0)..c1.min(w) {
            let v = grid.data[(r * w + c) as usize];
            if v == grid.meta.nodata {
                continue;
            }
            acc = Some(match acc {
                None => (v, v, (c, r)),
                Some((lo, hi, at)) => (lo.min(v), if v > hi { v } else { hi }, if v > hi { (c, r) } else { at }),
            });
        }
    }
    acc
}

/// Street level near (lat, lon): the lowest surface within 60 m.
fn street_m(grid: &DsmGrid, lat: f64, lon: f64) -> Option<f64> {
    let (c, r) = grid.cell_of(lat, lon);
    let n = (60.0 / grid.meta.cell_m) as i64;
    square(grid, c as i64 - n, r as i64 - n, c as i64 + n + 1, r as i64 + n + 1).map(|(lo, _, _)| f64::from(lo))
}

/// CoStar buildings within `reach_m`, measured on the height map: top = highest surface within 20 m
/// of the kit's point, base = street level. Buildings under 6 m above the street are left out.
fn costar_buildings(kit: &costar::Kit, grid: &DsmGrid, lat: f64, lon: f64, reach_m: f64) -> Vec<Object> {
    let n = (20.0 / grid.meta.cell_m) as i64;
    kit.buildings_near(lat, lon, reach_m)
        .into_iter()
        .filter_map(|b| {
            let (c, r) = grid.cell_of(b.lat, b.lon);
            let (_, top, _) = square(grid, c as i64 - n, r as i64 - n, c as i64 + n + 1, r as i64 + n + 1)?;
            let base = street_m(grid, b.lat, b.lon)?;
            (f64::from(top) - base >= 6.0).then(|| Object {
                id: b.id.clone(),
                kind: "building".into(),
                lat: b.lat,
                lon: b.lon,
                base_m: base,
                top_m: f64::from(top),
                radius_m: 15.0,
                source: "costar".into(),
                name: b.name.clone(),
            })
        })
        .collect()
}

/// Buildings found in the height map: 25 m blocks whose highest point stands >= 8 m above the street
/// (lowest surface within 2 blocks) and above its 8 neighbouring blocks.
fn dsm_buildings(grid: &DsmGrid, lat: f64, lon: f64, reach_m: f64) -> Vec<Object> {
    let block = (25.0 / grid.meta.cell_m).round() as i64;
    let (cc, cr) = grid.cell_of(lat, lon);
    let nb = (reach_m / 25.0).ceil() as i64;
    let side = (2 * nb + 1) as usize;
    let at = |bx: i64, by: i64| -> (i64, i64) { (cc as i64 + bx * block - block / 2, cr as i64 + by * block - block / 2) };
    let mut blocks: Vec<Option<(f32, f32, (i64, i64))>> = Vec::with_capacity(side * side);
    for by in -nb..=nb {
        for bx in -nb..=nb {
            let (c0, r0) = at(bx, by);
            blocks.push(square(grid, c0, r0, c0 + block, r0 + block));
        }
    }
    let get = |bx: i64, by: i64| -> Option<(f32, f32, (i64, i64))> {
        if bx < -nb || bx > nb || by < -nb || by > nb {
            return None;
        }
        blocks[((by + nb) as usize) * side + (bx + nb) as usize]
    };
    let mut out = Vec::new();
    for by in -nb..=nb {
        for bx in -nb..=nb {
            let Some((_, top, (c, r))) = get(bx, by) else { continue };
            if (bx.abs().max(by.abs()) * 25) as f64 > reach_m {
                continue;
            }
            let mut street = f32::MAX;
            let mut peak = true;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    if let Some((lo, hi, _)) = get(bx + dx, by + dy) {
                        street = street.min(lo);
                        let neighbour = dx.abs() <= 1 && dy.abs() <= 1 && (dx, dy) != (0, 0);
                        // ties go to the earlier block, so a flat roof gives one object
                        if neighbour && (hi > top || (hi == top && (dy, dx) < (0, 0))) {
                            peak = false;
                        }
                    }
                }
            }
            if !peak || f64::from(top - street) < 8.0 {
                continue;
            }
            let (olat, olon) = latlon_of(grid, c as f64 + 0.5, r as f64 + 0.5);
            out.push(Object {
                id: format!("dsm:{olat:.5},{olon:.5}"),
                kind: "building".into(),
                lat: olat,
                lon: olon,
                base_m: f64::from(street),
                top_m: f64::from(top),
                radius_m: 12.5,
                source: "dsm".into(),
                name: None,
            });
        }
    }
    out
}

/// Highest height in the grid (lets rays stop early).
fn grid_top(grid: &DsmGrid) -> f64 {
    f64::from(grid.data.iter().filter(|&&h| h != grid.meta.nodata).fold(f32::MIN, |a, &h| a.max(h)))
}

/// The second tower stands at least this far from the first, so the two read as two buildings.
const TOWER2_APART_M: f64 = 60.0;

/// The tallest building (top above base) within `radius_m` of the centre, not within `apart_m` of `not_near`.
fn tower_of<'a>(objects: &'a [Object], lat: f64, lon: f64, radius_m: f64, not_near: Option<(&Object, f64)>) -> Option<&'a Object> {
    objects
        .iter()
        .filter(|o| costar::distance_m(lat, lon, o.lat, o.lon) <= radius_m)
        .filter(|o| not_near.map_or(true, |(t, apart)| costar::distance_m(t.lat, t.lon, o.lat, o.lon) >= apart))
        .max_by(|a, b| (a.top_m - a.base_m).total_cmp(&(b.top_m - b.base_m)))
}

// ---- inputs ---------------------------------------------------------------------------

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every cost function in `library/*.json` (the seeds and the lab's entries), as they'd ship.
fn load_library() -> Result<Vec<CostFn>, Box<dyn std::error::Error>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(repo().join("library"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let f: CostFn = serde_json::from_str(&std::fs::read_to_string(&p)?).map_err(|e| format!("{}: {e}", p.display()))?;
        golden_core::expr::check(&f).map_err(|e| format!("{}: {e:?}", p.display()))?;
        out.push(f);
    }
    Ok(out)
}

/// The height map for a place, only if every tile it needs is already cached.
async fn cached_grid(place: &Place, radius_m: f64) -> Result<Option<DsmGrid>, Box<dyn std::error::Error>> {
    let opts = vexcel::DsmOptions { near_radius_m: 1.0, ..vexcel::DsmOptions::default() };
    let need = skyline_need_m(radius_m);
    let tiles = vexcel::plan_tiles(place.lat, place.lon, need, opts.near_radius_m);
    let all_cached = vexcel::cached_only(&tiles, |t| vexcel::cache::tile_path(&opts.cache_root, t).exists()).is_some();
    if !all_cached {
        return Ok(None);
    }
    Ok(Some(vexcel::get_dsm_with(place.lat, place.lon, need, &opts).await?))
}

// ---- main -----------------------------------------------------------------------------

fn arg(args: &[String], name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).cloned()
}

/// What one place's measurements produced: its printed log, rows, shots and its line for the report.
struct PlaceOut {
    log: String,
    rows: Vec<Row>,
    shots: Vec<Shot>,
    label: Option<String>,
}

/// What every place shares: the library, CoStar, the scenarios and gates, and the command-line filters.
struct Shared<'a> {
    library: &'a [CostFn],
    kit: Option<&'a costar::Kit>,
    scenes: &'a [Scenario],
    gates: &'a [Gate],
    only_scenario: Option<&'a str>,
    only_gate: Option<&'a str>,
    radius_m: f64,
}

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Every scenario and gate at one place, both days. Runs on its own thread (searches are single-threaded).
fn measure_place(place: &Place, grid: &DsmGrid, took: Duration, sh: &Shared) -> Res<PlaceOut> {
    use std::fmt::Write as _;
    let Shared { library, kit, scenes, gates, only_scenario, only_gate, radius_m } = *sh;
    let library = library.to_vec();
    let (mut log, mut rows, mut shots) = (String::new(), Vec::new(), Vec::new());
    let reach = radius_m + golden_core::objects::OBJECT_REACH_M;
    let mut objects = kit.as_ref().map(|k| costar_buildings(k, &grid, place.lat, place.lon, reach)).unwrap_or_default();
    // CoStar only where it has a real set of buildings, including one inside the area to be the tower.
    let source = if objects.len() >= 20 && tower_of(&objects, place.lat, place.lon, radius_m, None).is_some() { "costar" } else { "dsm" };
    if source == "dsm" {
        objects = dsm_buildings(&grid, place.lat, place.lon, reach);
    }
    let Some(tower) = tower_of(&objects, place.lat, place.lon, radius_m, None).cloned() else {
        let _ = writeln!(log, "== {}: no building objects, skipped", place.name);
        return Ok(PlaceOut { log, rows, shots, label: None });
    };
    let tower2 = tower_of(&objects, place.lat, place.lon, radius_m, Some((&tower, TOWER2_APART_M))).cloned().unwrap_or_else(|| tower.clone());
    let _ = writeln!(log, 
        "== {} ({}): grid {}x{} ({:.1?}), {} buildings from {source}, tower {} ({:.0} m tall), second {} ({:.0} m)",
        place.name,
        place.city,
        grid.meta.width,
        grid.meta.height,
        took,
        objects.len(),
        tower.id,
        tower.top_m - tower.base_m,
        tower2.id,
        tower2.top_m - tower2.base_m
    );
    let label = Some(format!("{} ({}, buildings from {source})", place.name, place.city));
    let inputs = Inputs { library: &library, objects: &objects };
    let top_m = grid_top(&grid);

    for (date, la_off, pr_off) in DAYS {
        let offset_s = if place.city == "LA" { la_off } else { pr_off };
        let day = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|e| e.to_string())?;
        let at = At { place, radius_m, date, offset_s };

        for scene in scenes.iter().filter(|s| only_scenario.as_deref().map_or(true, |k| k == s.key)) {
            let track = sun_track(place.lat, place.lon, day, scene.event).ok_or("no sun track")?;
            let azs = track.samples.iter().map(|s| s.az_deg);
            let track_az = azs.fold((f64::MAX, f64::MIN), |(lo, hi), a| (lo.min(a), hi.max(a)));
            let id = |o: &Object| format!(r#"{{"id":"{}"}}"#, o.id);
            let (a, b) = if scene.second { (&tower2, &tower) } else { (&tower, &tower2) };
                let ctx = Ctx { tower: id(a), tower2: id(b), track_az };
            let base_goals = parse_goals(&(scene.goals)(&ctx))?;
            let base = match run(&grid, &at, (scene.standing, scene.event), base_goals.clone(), K, &inputs) {
                Ok(r) => r,
                Err(e) => {
                    let _ = writeln!(log, "  {date} {}: {e}", scene.key);
                    continue;
                }
            };
            let top = ids(&base.out);
            let base_sky = skyline_goals(&base_goals, &library);
            shots.push(Shot {
                place: place.name.into(),
                scenario: scene.key.into(),
                searched: base.out.searched as usize,
                traces: traces(&base.out, &base_sky),
                spots: top.len(),
                ms: base.took.as_secs_f64() * 1e3,
            });
            let base_heat = &base.out.heat.values;
            let near: Vec<usize> = (0..base_heat.len()).filter(|&i| base_heat[i].is_some_and(|v| v >= NEAR_BEST)).collect();
            let _ = writeln!(log, 
                "  {date} {:<16} ungated: {} searched, {} spots, {:.0} ms",
                scene.key,
                base.out.searched,
                top.len(),
                base.took.as_secs_f64() * 1e3
            );
            for gate in gates.iter().filter(|g| g.on.contains(&scene.key) && only_gate.as_deref().map_or(true, |n| n == g.name)) {
                let gate_goal: Goal = serde_json::from_str(&(gate.goal)(&ctx, &library)).map_err(|e| format!("gate {}: {e}", gate.name))?;
                let goals: Vec<Goal> = match gate.replaces {
                    // same weight and must as the goal it stands in for, so only the entry's own must differs
                    Some(i) => base_goals
                        .iter()
                        .enumerate()
                        .map(|(j, g)| if j == i { Goal { cost: gate_goal.cost.clone(), ..g.clone() } } else { g.clone() })
                        .collect(),
                    None => base_goals.iter().cloned().chain([gate_goal.clone()]).collect(),
                };
                let prune_goal = match gate.replaces {
                    Some(_) => must_alone(&gate.name, &ctx, &library, gate.objs)?,
                    None => gate_goal,
                };
                let gated = match run(&grid, &at, (scene.standing, scene.event), goals.clone(), K, &inputs) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = writeln!(log, "    {:<22} error: {e}", gate.name);
                        continue;
                    }
                };
                let alone = run(&grid, &at, (scene.standing, scene.event), vec![prune_goal], 1, &inputs)?;
                let cells: Vec<&Option<f32>> = alone.out.heat.values.iter().filter(|v| v.is_some()).collect();
                let searched = cells.len();
                let kept = cells.iter().filter(|v| v.is_some_and(|x| x > 0.0)).count();
                let gated_ids = ids(&gated.out);
                if std::env::var("LAB_DEBUG").is_ok() {
                    print_spots(&[("ungated", &base.out), ("gated", &gated.out)]);
                }
                let found = top.iter().filter(|id| gated_ids.contains(id)).count();
                let near_kept = near.iter().filter(|&&i| gated.out.heat.values[i].is_some_and(|v| v > 0.0)).count();
                let traces_base = traces(&base.out, &base_sky);
                let traces_gated = match gate.replaces {
                    // the entry's own Point must runs before its Skyline score, inside one stage
                    Some(i) if base_sky.iter().any(|n| n == base_goals[i].name()) => traces_base.saturating_sub(searched - kept),
                    _ => traces(&gated.out, &skyline_goals(&goals, &library)),
                };
                let row = Row {
                    place: place.name.into(),
                    city: place.city.into(),
                    date: date.into(),
                    scenario: scene.key.into(),
                    gate: gate.name.clone(),
                    kind: gate.kind.name().into(),
                    class: gate.class,
                    traces_base,
                    traces_gated,
                    gated_spots: gated.out.spots.len(),
                    rays: 0.0,
                    sky_rays: skyline_rays(&track),
                    searched,
                    kept,
                    prune: if searched > 0 { 1.0 - kept as f64 / searched as f64 } else { 0.0 },
                    recall: if top.is_empty() { 1.0 } else { found as f64 / top.len() as f64 },
                    near_best: (near_kept, near.len()),
                    top: top.len(),
                    t_base_ms: base.took.as_secs_f64() * 1e3,
                    t_gated_ms: gated.took.as_secs_f64() * 1e3,
                };
                let _ = writeln!(log, 
                    "    {:<22} prune {:>3.0}%  recall@10 {:>3.0}%  near-best {}/{}  {:.0} -> {:.0} ms",
                    gate.name,
                    row.prune * 100.0,
                    row.recall * 100.0,
                    near_kept,
                    near.len(),
                    row.t_base_ms,
                    row.t_gated_ms
                );
                rows.push(row);
            }
            let ray_gates = RAY_GATES.iter().filter(|g| g.on.contains(&scene.key) && only_gate.as_deref().map_or(true, |n| n == g.name));
            for gate in ray_gates {
                let cands = heat_candidates(&grid, &base.out);
                RAYS.with(|r| r.set(0));
                let t = Instant::now();
                let kept = cands.iter().filter(|&&(la, lo, e)| (gate.pass)(&grid, eye_at(&grid, la, lo, e), &track, top_m)).count();
                let gate_ms = t.elapsed().as_secs_f64() * 1e3;
                let rays = RAYS.with(|r| r.get());
                // spots are rounded to 1e-5 deg; test the candidate each one came from
                let at = |s: &contracts::Spot| {
                    let d = |c: &&(f64, f64, f64)| costar::distance_m(s.lat, s.lon, c.0, c.1);
                    cands.iter().min_by(|a, b| d(a).total_cmp(&d(b))).copied().unwrap_or((s.lat, s.lon, s.eye_m))
                };
                let passed = base.out.spots.iter().map(at).filter(|&(la, lo, e)| (gate.pass)(&grid, eye_at(&grid, la, lo, e), &track, top_m)).count();
                let searched = cands.len();
                let prune = if searched > 0 { 1.0 - kept as f64 / searched as f64 } else { 0.0 };
                let t_base_ms = base.took.as_secs_f64() * 1e3;
                let row = Row {
                    place: place.name.into(),
                    city: place.city.into(),
                    date: date.into(),
                    scenario: scene.key.into(),
                    gate: gate.name.into(),
                    kind: GateKind::WhatIfRay.name().into(),
                    class: 'a',
                    traces_base: traces(&base.out, &base_sky),
                    traces_gated: traces(&base.out, &base_sky).saturating_sub(searched - kept),
                    gated_spots: top.len(),
                    rays: rays as f64 / searched.max(1) as f64,
                    sky_rays: skyline_rays(&track),
                    searched,
                    kept,
                    prune,
                    recall: if top.is_empty() { 1.0 } else { passed as f64 / top.len() as f64 },
                    near_best: (0, 0),
                    top: top.len(),
                    t_base_ms,
                    // estimate: the skyline work scales with the candidates left, plus the rays
                    t_gated_ms: t_base_ms * (1.0 - prune) + gate_ms,
                };
                let _ = writeln!(log, 
                    "    {:<22} prune {:>3.0}%  recall@10 {:>3.0}%  (ray what-if, {:.1} ms of rays)",
                    gate.name,
                    row.prune * 100.0,
                    row.recall * 100.0,
                    gate_ms
                );
                rows.push(row);
            }
        }
    }
    Ok(PlaceOut { log, rows, shots, label })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let write = args.iter().any(|a| a == "--write");
    let only_place = arg(&args, "--place");
    let only_scenario = arg(&args, "--scenario");
    let only_gate = arg(&args, "--gate");
    let radius_m: f64 = arg(&args, "--radius").map_or(Ok(RADIUS_M), |r| r.parse()).map_err(|e| format!("--radius: {e}"))?;
    if write && radius_m != RADIUS_M {
        return Err("--write only with the default radius (the doc's table is for one radius)".into());
    }

    let library = load_library()?;
    if args.iter().any(|a| a == "--report") {
        // rebuild the generated section from the last --write's saved rows, without searching
        let dir = repo().join("cache").join("library_lab");
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).map_err(|e| format!("{f}: {e} (run with --write first)"));
        let rows: Vec<Row> = serde_json::from_str(&read("rows.json")?)?;
        let shots: Vec<Shot> = serde_json::from_str(&read("shots.json")?)?;
        let places_run: Vec<String> = serde_json::from_str(&read("places.json")?)?;
        let gates = gates();
        let names: Vec<(String, GateKind)> =
            gates.iter().map(|g| (g.name.clone(), g.kind)).chain(RAY_GATES.iter().map(|g| (g.name.to_string(), GateKind::WhatIfRay))).collect();
        let report = report(&rows, &shots, &names, &scenarios(), &places_run, &library, RADIUS_M);
        write_report(&report)?;
        println!("rebuilt docs/library-lab.md from cache/library_lab");
        return Ok(());
    }
    let kit = match costar::Kit::from_env() {
        Ok(k) => Some(k),
        Err(e) => {
            eprintln!("no CoStar ({e}); using buildings from the height map everywhere");
            None
        }
    };
    let scenes = scenarios();
    let gates = gates();
    let mut rows: Vec<Row> = Vec::new();
    let mut shots: Vec<Shot> = Vec::new();
    let mut places_run: Vec<String> = Vec::new();

    let chosen: Vec<&Place> = PLACES.iter().filter(|p| only_place.as_deref().map_or(true, |n| n == p.name)).collect();
    // places run in parallel (each search is single-threaded); LAB_THREADS=1 for clean timings
    let threads: usize = std::env::var("LAB_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(6).max(1);
    let shared = Shared {
        library: &library,
        kit: kit.as_ref(),
        scenes: &scenes,
        gates: &gates,
        only_scenario: only_scenario.as_deref(),
        only_gate: only_gate.as_deref(),
        radius_m,
    };
    for batch in chosen.chunks(threads) {
        let mut loaded = Vec::new();
        for &place in batch {
            let t = Instant::now();
            match cached_grid(place, radius_m).await? {
                Some(grid) => loaded.push((place, grid, t.elapsed())),
                None => println!("== {} ({}): height map not fully cached, skipped", place.name, place.city),
            }
        }
        let outs: Vec<Res<PlaceOut>> = std::thread::scope(|s| {
            let handles: Vec<_> = loaded.iter().map(|(place, grid, took)| s.spawn(|| measure_place(place, grid, *took, &shared))).collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("a place's thread panicked".into()))).collect()
        });
        for out in outs {
            let out = out.map_err(|e| e.to_string())?;
            print!("{}", out.log);
            rows.extend(out.rows);
            shots.extend(out.shots);
            places_run.extend(out.label);
        }
    }

    let names: Vec<(String, GateKind)> =
        gates.iter().map(|g| (g.name.clone(), g.kind)).chain(RAY_GATES.iter().map(|g| (g.name.to_string(), GateKind::WhatIfRay))).collect();
    let report = report(&rows, &shots, &names, &scenes, &places_run, &library, radius_m);
    println!("\n{report}");
    if write {
        write_report(&report)?;
        let dir = repo().join("cache").join("library_lab");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("rows.json"), serde_json::to_string_pretty(&rows)?)?;
        std::fs::write(dir.join("shots.json"), serde_json::to_string_pretty(&shots)?)?;
        std::fs::write(dir.join("places.json"), serde_json::to_string_pretty(&places_run)?)?;
        println!("wrote docs/library-lab.md and cache/library_lab/{{rows,shots,places}}.json");
    }
    Ok(())
}

// ---- the report -----------------------------------------------------------------------

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Runs that tested recall (the ungated search found spots), and on how many places.
fn tested(r: &[&Row]) -> (usize, usize) {
    let t: Vec<&&Row> = r.iter().filter(|r| r.top > 0).collect();
    let mut places: Vec<&str> = t.iter().map(|r| r.place.as_str()).collect();
    places.sort();
    places.dedup();
    (t.len(), places.len())
}

fn pct(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

/// The control scenario: a gate on a goal that is only weighted. Shown, not used for verdicts.
const CONTROL: &str = "soft-tower";

/// The generated part of docs/library-lab.md: one summary row per (gate, scenario), then the verdict per gate.
fn report(rows: &[Row], shots: &[Shot], gates: &[(String, GateKind)], scenes: &[Scenario], places: &[String], library: &[CostFn], radius_m: f64) -> String {
    let mut s = String::new();
    let days: Vec<&str> = DAYS.iter().map(|d| d.0).collect();
    s.push_str(&format!("Places: {}. Days: {}. Area radius {radius_m} m, recall over the top {K}.\n\n", places.join(", "), days.join(", ")));
    s.push_str("Scenarios (shot classes), ungated. Runs with at least one spot, then medians over the runs: candidates searched, skyline traces, spots picked, time.\n\n");
    s.push_str("| scenario | standing | event | what it asks | runs with a spot | candidates | skyline traces | spots | time |\n");
    s.push_str("|---|---|---|---|---|---|---|---|---|\n");
    for sc in scenes {
        let when = if sc.event == Event::Sunrise { "sunrise" } else { "sunset" };
        let r: Vec<&Shot> = shots.iter().filter(|x| x.scenario == sc.key).collect();
        if r.is_empty() {
            continue;
        }
        let mut places: Vec<&str> = r.iter().filter(|x| x.spots > 0).map(|x| x.place.as_str()).collect();
        places.dedup();
        let med = |f: fn(&Shot) -> f64| median(r.iter().map(|x| f(x)).collect());
        s.push_str(&format!(
            "| `{}` | {:?} | {when} | {} | {} of {} ({} places) | {:.0} | {:.0} | {:.0} | {:.0} ms |\n",
            sc.key,
            sc.standing,
            sc.about,
            r.iter().filter(|x| x.spots > 0).count(),
            r.len(),
            places.len(),
            med(|x| x.searched as f64),
            med(|x| x.traces as f64),
            med(|x| x.spots as f64),
            med(|x| x.ms),
        ));
    }
    s.push_str("\nClass (a) = speed gate, judged by recall@10 and prune. Class (b) = shot class (changes the result on purpose), judged by prune and a sane result.\n");
    s.push_str("Skyline traces are summed over all runs of the row. Ray what-ifs: traces and time are estimates (the gate is evaluated in the lab, not in a search).\n\n");
    s.push_str("| gate | class | kind | scenario | runs | places | tested runs (places) | prune median (min-max) | recall@10 min | near-best kept | skyline traces ungated -> gated | rays/cand (one trace) | time ungated -> gated |\n");
    s.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for (name, kind) in gates {
        for sc in scenes {
            let r: Vec<&Row> = rows.iter().filter(|r| &r.gate == name && r.scenario == sc.key).collect();
            if r.is_empty() {
                continue;
            }
            let prunes: Vec<f64> = r.iter().map(|r| r.prune).collect();
            let (lo, hi) = prunes.iter().fold((1.0f64, 0.0f64), |(lo, hi), &p| (lo.min(p), hi.max(p)));
            let recall = r.iter().map(|r| r.recall).fold(1.0, f64::min);
            let (nk, nt) = r.iter().fold((0, 0), |(a, b), r| (a + r.near_best.0, b + r.near_best.1));
            let near = if *kind == GateKind::WhatIfRay { "-".to_string() } else { format!("{nk}/{nt}") };
            let mut places: Vec<&str> = r.iter().map(|r| r.place.as_str()).collect();
            places.dedup();
            let tb: usize = r.iter().map(|r| r.traces_base).sum();
            let tg: usize = r.iter().map(|r| r.traces_gated).sum();
            let rays = if *kind == GateKind::WhatIfRay {
                let n = r.len() as f64;
                format!("{:.1} ({:.0})", r.iter().map(|r| r.rays).sum::<f64>() / n, r.iter().map(|r| r.sky_rays as f64).sum::<f64>() / n)
            } else {
                "-".into()
            };
            let (tested, tested_places) = tested(&r);
            let msb: f64 = r.iter().map(|r| r.t_base_ms).sum::<f64>() / r.len() as f64;
            let msg: f64 = r.iter().map(|r| r.t_gated_ms).sum::<f64>() / r.len() as f64;
            s.push_str(&format!(
                "| `{name}` | ({}) | {} | {} | {} | {} | {tested} ({tested_places}) | {} ({}-{}) | {} | {near} | {tb} -> {tg} | {rays} | {msb:.0} -> {msg:.0} ms |\n",
                r[0].class,
                kind.name(),
                sc.key,
                r.len(),
                places.len(),
                pct(median(prunes.clone())),
                pct(lo),
                pct(hi),
                pct(recall),
            ));
        }
    }
    s.push_str("\nVerdicts. A class (a) gate is kept when recall@10 is 100% in every run, on at least 2 places, with median prune at least ");
    s.push_str(&format!("{}. Class (b) is reported, not judged on recall. The `{CONTROL}` rows (a gate on a goal that is only weighted) are left out of the verdicts.\n\n", pct(KEEP_PRUNE)));
    for (name, kind) in gates {
        // the control (a gate on a goal that is only weighted) is in the table but not judged
        let r: Vec<&Row> = rows.iter().filter(|r| &r.gate == name && r.scenario != CONTROL).collect();
        if r.is_empty() {
            continue;
        }
        let recall = r.iter().map(|r| r.recall).fold(1.0, f64::min);
        let prune = median(r.iter().map(|r| r.prune).collect());
        let mut places: Vec<&str> = r.iter().map(|r| r.place.as_str()).collect();
        places.sort();
        places.dedup();
        let verdict = if r[0].class == 'b' {
            let empty = r.iter().filter(|r| r.gated_spots == 0 && r.top > 0).count();
            format!("**shot class**: median prune {}, a result in {} of {} runs (ungated top 10 kept: {} at worst)", pct(prune), r.len() - empty, r.len(), pct(recall))
        } else if recall < 1.0 {
            let worst = r.iter().filter(|r| r.recall < 1.0).map(|r| format!("{} {} {} ({})", r.place, r.date, r.scenario, pct(r.recall))).collect::<Vec<_>>();
            format!("**drop**: loses top spots in {}", worst.join("; "))
        } else if places.len() < 2 || tested(&r).1 < 2 {
            format!("not enough places (recall tested in {} runs on {} places)", tested(&r).0, tested(&r).1)
        } else if prune < KEEP_PRUNE {
            format!("**weak**: full recall, but median prune only {}", pct(prune))
        } else {
            format!("**keep**: full recall (tested in {} runs on {} places), median prune {}", tested(&r).0, tested(&r).1, pct(prune))
        };
        let about = library.iter().find(|f| &f.name == name).map(|f| format!(" ({})", f.about)).unwrap_or_default();
        s.push_str(&format!("- `{name}` [{}]{about}: {verdict}\n", kind.name()));
    }
    s
}

const BEGIN: &str = "<!-- library-lab:begin (generated by `--write`, don't edit by hand) -->";
const END: &str = "<!-- library-lab:end -->";

fn write_report(report: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = repo().join("docs").join("library-lab.md");
    let doc = std::fs::read_to_string(&path).unwrap_or_else(|_| format!("# Library lab\n\n{BEGIN}\n{END}\n"));
    let (Some(a), Some(b)) = (doc.find(BEGIN), doc.find(END)) else {
        return Err(format!("{} has no {BEGIN} ... {END} markers", path.display()).into());
    };
    let out = format!("{}{BEGIN}\n\n{report}\n{}", &doc[..a], &doc[b..]);
    std::fs::write(&path, out)?;
    Ok(())
}
