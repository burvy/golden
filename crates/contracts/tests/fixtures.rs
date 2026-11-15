//! Every fixture in `fixtures/` must parse into its contract struct, with no unknown fields,
//! and survive a JSON round trip. If this fails, the fixture and docs/contracts.md disagree.

use std::path::PathBuf;

use contracts::scene::{decode_dm, SCENE_ENCODING};
use contracts::*;
use serde::de::DeserializeOwned;
use serde::Serialize;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
}

fn load<T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug>(name: &str) -> T {
    let text = std::fs::read_to_string(fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let v: T = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} does not match its contract: {e}"));
    let back: T = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
    assert_eq!(back, v, "{name} changes on a round trip");
    v
}

#[test]
fn sun_path() {
    let s: SunPath = load("sun_path.json");
    assert_eq!(s.sunset.unwrap().t, "2026-06-21T20:07:41-07:00");
    assert!(s.points.windows(2).all(|w| w[0].t < w[1].t), "points in time order");
}

#[test]
fn horizon_profile() {
    let h: HorizonProfile = load("horizon_profile.json");
    assert_eq!(h.points.len(), (360.0 / h.step_deg) as usize);
    assert!((h.observer.eye_m - h.observer.ground_m - 1.7).abs() < 1e-9);
    assert!(h.points.iter().any(|p| p.dist_m.is_none() && p.elev_deg == 0.0));
    assert!(h.sun.is_some());
}

#[test]
fn horizon_without_date_omits_sun() {
    let mut h: HorizonProfile = load("horizon_profile.json");
    h.sun = None;
    assert!(!serde_json::to_string(&h).unwrap().contains("\"sun\""));
}

#[test]
fn brief_and_defaults() {
    let b: Brief = load("brief.json");
    assert_eq!((b.standing, b.framing, b.event), (Standing::Roof, Framing::Between, Event::Sunset));
    let minimal: Brief =
        serde_json::from_str(r#"{"area":{"lat":34.049,"lon":-118.251,"radius_m":300},"date":"2026-06-24"}"#).unwrap();
    assert_eq!(minimal.tz, "America/Los_Angeles");
    assert_eq!((minimal.standing, minimal.framing, minimal.max_spots), (Standing::Any, Framing::Clear, 10));
}

#[test]
fn brief_rejects_typos() {
    let typo = r#"{"area":{"lat":34,"lon":-118,"radius_m":300},"date":"2026-06-24","framming":"between"}"#;
    assert!(serde_json::from_str::<Brief>(typo).is_err());
}

#[test]
fn search_result() {
    let r: SearchResult = load("search_result.json");
    assert!(r.spots.windows(2).all(|w| w[0].rank < w[1].rank && w[0].score >= w[1].score));
    assert!(r.spots.iter().all(|s| s.standing != Standing::Any));
    assert_eq!(r.heat.values.len(), (r.heat.width * r.heat.height) as usize);
    assert!(r.heat.width <= search::MAX_HEAT_SIDE);
}

#[test]
fn scene_meta_and_binary_agree() {
    let m: SceneMeta = load("scene_meta.json");
    assert_eq!(m.encoding, SCENE_ENCODING);
    let bin = std::fs::read(fixture("scene.bin")).unwrap();
    assert_eq!(bin.len() as u64, m.bytes);
    let mut next = 0;
    for r in &m.rings {
        assert_eq!(r.byte_offset, next, "rings are back to back");
        next += r.byte_len();
    }
    assert_eq!(next, m.bytes);
    let first = u16::from_le_bytes([bin[0], bin[1]]);
    assert_eq!(decode_dm(first, m.base_m), Some(90.0));
}

#[test]
fn photo_meta() {
    let p: PhotoMeta = load("photo_meta.json");
    assert_eq!(p.kind, PhotoKind::Oblique);
    assert!(p.width <= photo::MAX_PHOTO_PX && p.height <= photo::MAX_PHOTO_PX);
}

#[test]
fn buildings() {
    let b: BuildingList = load("buildings.json");
    assert!(b.buildings.windows(2).all(|w| w[0].dist_m <= w[1].dist_m), "nearest first");
    assert!(b.buildings.iter().all(|x| x.id.starts_with("costar:")));
}

#[test]
fn error() {
    let e: ApiError = load("error.json");
    assert!(!e.error.is_empty());
}

#[test]
fn agent_request() {
    let r: AgentRequest = load("agent_request.json");
    assert!(r.date.is_none() && r.area.is_some());
    let minimal: AgentRequest = serde_json::from_str(r#"{"text":"sunset from a roof"}"#).unwrap();
    assert_eq!((minimal.area, minimal.date, minimal.tz.as_str()), (None, None, "America/Los_Angeles"));
}

#[test]
fn call_sheet() {
    let c: CallSheet = load("call_sheet.json");
    assert_eq!(c.spots[0].role, SpotRole::Primary, "primary comes first");
    assert_eq!(c.spots.iter().filter(|s| s.role == SpotRole::Primary).count(), 1, "exactly one primary");
    assert!(c.timeline.windows(2).all(|w| w[0].t < w[1].t), "timeline in time order");
    let roof_ids: Vec<_> = c.spots.iter().filter_map(|s| s.spot.building_id.as_ref()).collect();
    assert!(c.access.iter().all(|a| roof_ids.contains(&&a.building_id)), "access only for sheet buildings");
}

#[test]
fn chat_request() {
    let r: ChatRequest = load("chat_request.json");
    assert_eq!(r.messages[0].role, ChatRole::User, "the user speaks first");
    let minimal: ChatRequest = serde_json::from_str(r#"{"messages":[{"role":"user","text":"sunset"}]}"#).unwrap();
    assert_eq!((minimal.area, minimal.date, minimal.tz.as_str(), minimal.go), (None, None, "America/Los_Angeles", false));
}

#[test]
fn chat_reply() {
    let r: ChatReply = load("chat_reply.json");
    assert!(r.ask.is_some() && r.brief.is_none() && r.sheet.is_none(), "a question, or a search, never both");
    // the done form: the brief it settled on plus the call sheet
    let sheet: CallSheet = load("call_sheet.json");
    let mut brief = sheet.brief.clone();
    brief.goals = vec![Goal::using("sun_between"), serde_json::from_str(r#"{"use":"in_front","args":{"a":{"kind":"fountain"},"b":{"kind":"tower"}}}"#).unwrap()];
    let note = Some("I can't place the fountain yet; searching for the sunset part.".to_string());
    let done = ChatReply { understood: vec!["fountain in front of a tower (not checked yet)".into()], ask: None, brief: Some(brief), sheet: Some(sheet), model: "mock".into(), note };
    let back: ChatReply = serde_json::from_str(&serde_json::to_string(&done).unwrap()).unwrap();
    assert_eq!(back, done);
    assert!(!serde_json::to_string(&r).unwrap().contains("note"), "no note: left out");
}

// ---- goals (docs/metrics.md v2) -------------------------------------------------------------

#[test]
fn old_briefs_still_parse_and_print_unchanged() {
    // brief.json predates goals: it parses, gets no goals and the default lens, and printing it
    // adds only `lens_deg` (goals are left out while empty).
    let b: Brief = load("brief.json");
    assert!(b.goals.is_empty());
    assert_eq!(b.lens_deg, DEFAULT_LENS_DEG);
    let printed = floats(serde_json::to_value(&b).unwrap());
    let mut old: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(fixture("brief.json")).unwrap()).unwrap();
    old["lens_deg"] = serde_json::json!(60.0);
    assert_eq!(printed, floats(old));
    // An old SearchResult (spots without `scores`) parses too.
    let mut r: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(fixture("search_result.json")).unwrap()).unwrap();
    for s in r["spots"].as_array_mut().unwrap() {
        s.as_object_mut().unwrap().remove("scores");
    }
    let r: SearchResult = serde_json::from_value(r).unwrap();
    assert!(r.spots.iter().all(|s| s.scores.is_empty()));
}

#[test]
fn brief_with_goals() {
    let b: Brief = load("brief_goals.json");
    assert_eq!(b.lens_deg, 50.0);
    let names: Vec<&str> = b.goals.iter().map(Goal::name).collect();
    assert_eq!(names, ["sun_between", "in_front", "tree_framing_the_tower"]);
    assert_eq!(b.goals.iter().map(Goal::weight_or_default).collect::<Vec<_>>(), [1.0, 0.0, 0.5]);
    assert!(b.goals[1].must);
    let GoalFn::Use { args, .. } = &b.goals[1].cost else { panic!("in_front is a library goal") };
    assert_eq!(args["b"], ObjRef::Id("costar:1234567".into()));
    let GoalFn::Def(f) = &b.goals[2].cost else { panic!("third goal is inline") };
    assert!(f.must.is_some());
    let Expr::List { op: ListOp::And, of } = &f.score else { panic!("score is an and") };
    assert_eq!(of.len(), 3);
}

#[test]
fn search_result_scores_follow_goals() {
    let r: SearchResult = load("search_result.json");
    assert!(r.spots.iter().all(|s| s.scores.len() == 1 && s.scores[0].goal == "sun_between"));
}

/// Every number as a float, so `300` and `300.0` compare equal.
fn floats(v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Number(n) => serde_json::json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.into_iter().map(floats).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, floats(v))).collect()),
        other => other,
    }
}

fn goal(json: &str) -> Result<Goal, String> {
    serde_json::from_str::<Goal>(json).map_err(|e| e.to_string())
}

fn expr(json: &str) -> Result<Expr, String> {
    serde_json::from_str::<Expr>(json).map_err(|e| e.to_string())
}

#[test]
fn goal_needs_exactly_one_source() {
    assert!(goal(r#"{"use":"sun_clear"}"#).is_ok());
    assert!(goal(r#"{"weight":1}"#).unwrap_err().contains("needs \"use\""));
    let both = r#"{"use":"sun_clear","def":{"name":"x","about":"x","score":{"num":1}}}"#;
    assert!(goal(both).unwrap_err().contains("not both"));
    assert!(goal(r#"{"use":"sun_clear","weight":-1}"#).unwrap_err().contains(">= 0"));
    assert!(goal(r#"{"use":"sun_clear","wieght":1}"#).is_err(), "typos are rejected");
}

#[test]
fn every_op_round_trips() {
    let all = [
        r#"{"f":"sun_visible_min"}"#,
        r#"{"f":"dist","obj":"a"}"#,
        r#"{"f":"bearing_diff","a":"a","b":{"id":"costar:1"}}"#,
        r#"{"num":0.5}"#,
        r#"{"op":"ramp","x":{"f":"gap_width_deg"},"lo":20,"hi":5}"#,
        r#"{"op":"bump","x":{"num":3},"at":0,"width":10}"#,
        r#"{"op":"and","of":[{"num":1},{"num":0}]}"#,
        r#"{"op":"or","of":[{"num":1}]}"#,
        r#"{"op":"min","of":[{"num":1}]}"#,
        r#"{"op":"max","of":[{"num":1}]}"#,
        r#"{"op":"mul","of":[{"num":1}]}"#,
        r#"{"op":"add","of":[{"num":1}]}"#,
        r#"{"op":"avg","of":[{"num":1}]}"#,
        r#"{"op":"not","x":{"f":"roof"}}"#,
        r#"{"op":"lt","x":{"num":1},"than":{"num":2}}"#,
        r#"{"op":"gt","x":{"num":1},"than":{"num":2}}"#,
    ];
    for text in all {
        let e = expr(text).unwrap_or_else(|err| panic!("{text}: {err}"));
        let back = floats(serde_json::to_value(&e).unwrap());
        let want = floats(serde_json::from_str(text).unwrap());
        assert_eq!(back, want, "{text} prints differently");
    }
    assert_eq!(EXPR_OPS.len(), all.len() - 4, "one case per op, plus 3 features and a number");
}

#[test]
fn bad_expressions_say_what_to_fix() {
    let cases = [
        (r#"{"op":"ramp","x":{"num":1},"lo":5}"#, "needs \"hi\""),
        (r#"{"op":"ramp","x":{"num":1},"lo":5,"hi":5}"#, "must differ"),
        (r#"{"op":"bump","x":{"num":1},"at":0,"width":0}"#, "width"),
        (r#"{"op":"pow","x":{"num":1}}"#, "unknown op \"pow\""),
        (r#"{"op":"not","x":{"num":1},"of":[]}"#, "doesn't take \"of\""),
        (r#"{"op":"and","of":[]}"#, "at least one"),
        (r#"{"num":1,"x":{"num":1}}"#, "doesn't take \"x\""),
        (r#"{"f":"dist","lo":1}"#, "doesn't take \"lo\""),
        (r#"{}"#, "needs \"f\""),
        (r#"{"op":"ramp","x":{"op":"nope"},"lo":0,"hi":1}"#, "unknown op \"nope\""),
        (r#"{"num":1,"colour":"red"}"#, "colour"),
    ];
    for (text, says) in cases {
        let err = expr(text).expect_err(text);
        assert!(err.contains(says), "{text}: error {err:?} should mention {says:?}");
    }
}

#[test]
fn object_refs() {
    let r: ObjRef = serde_json::from_str(r#"{"kind":"tree","near":{"id":"costar:1"},"within_m":300}"#).unwrap();
    assert!(matches!(&r, ObjRef::Kind { kind, within_m: Some(m), .. } if kind == "tree" && *m == 300.0));
    for (text, says) in [
        (r#"{"id":"a","kind":"tree"}"#, "not both"),
        (r#"{"id":"a","within_m":3}"#, "takes no"),
        (r#"{"kind":"tree","within_m":0}"#, "> 0"),
        (r#"{}"#, "needs \"id\" or \"kind\""),
    ] {
        let err = serde_json::from_str::<ObjRef>(text).unwrap_err().to_string();
        assert!(err.contains(says), "{text}: {err}");
    }
    assert!(OBJECT_KINDS.contains(&"building"));
}

// ---- goal layers and funnel stages (frontend v2, task 057/059) ------------------------------

/// The browser's rule (docs/contracts.md#goal-layers-and-stages): rebuild the combined heat and the
/// picked spots from the goal layers alone. Core must give the same answer for the same weights.
fn rebuild(searched: &HeatGrid, layers: &[GoalLayer], spacing_m: f64, n: usize) -> (Vec<Option<f64>>, Vec<usize>) {
    let cells = searched.values.len();
    let dropped = |i: usize| layers.iter().any(|l| l.gate.contains(&(i as u32)) || (l.must && l.heat.values[i].unwrap_or(0.0) <= 0.0));
    let mut ranked: Vec<(&GoalLayer, f64)> = layers.iter().filter(|l| l.weight > 0.0).map(|l| (l, l.weight)).collect();
    if ranked.is_empty() {
        ranked = layers.iter().map(|l| (l, 1.0 / layers.len() as f64)).collect();
    }
    let raw: Vec<Option<f64>> = (0..cells)
        .map(|i| {
            searched.values[i]?;
            Some(if dropped(i) { 0.0 } else { ranked.iter().map(|(l, w)| w * f64::from(l.heat.values[i].unwrap())).sum() })
        })
        .collect();
    let best = raw.iter().flatten().fold(0.0, |a: f64, &b| a.max(b));
    let heat = raw.iter().map(|v| v.map(|r| if best > 0.0 { r / best } else { 0.0 })).collect();
    let mut order: Vec<usize> = (0..cells).filter(|&i| raw[i].unwrap_or(0.0) > 0.0).collect();
    order.sort_by(|&a, &b| raw[b].unwrap().total_cmp(&raw[a].unwrap()).then(a.cmp(&b)));
    let h = &layers[0].heat;
    let mut picked: Vec<usize> = vec![];
    for i in order {
        if picked.len() == n {
            break;
        }
        if picked.iter().all(|&p| metres(cell_latlon(h, p), cell_latlon(h, i)) >= spacing_m) {
            picked.push(i);
        }
    }
    (heat, picked)
}

/// Centre of heat cell `i` (row 0 north), with the flat formula `DsmGrid` uses.
fn cell_latlon(h: &HeatGrid, i: usize) -> (f64, f64) {
    let (col, row) = ((i as u32 % h.width) as f64, (i as u32 / h.width) as f64);
    let east = (col + 0.5 - f64::from(h.width) / 2.0) * h.cell_m;
    let north = (f64::from(h.height) / 2.0 - row - 0.5) * h.cell_m;
    (h.center_lat + (north / EARTH_R).to_degrees(), h.center_lon + (east / (EARTH_R * h.center_lat.to_radians().cos())).to_degrees())
}

fn metres(a: (f64, f64), b: (f64, f64)) -> f64 {
    let north = (b.0 - a.0).to_radians() * EARTH_R;
    let east = (b.1 - a.1).to_radians() * EARTH_R * a.0.to_radians().cos();
    east.hypot(north)
}

#[test]
fn search_layers() {
    let r: SearchResult = load("search_layers.json");
    assert_eq!(r.brief.area.name.as_deref(), Some("Downtown LA, around Grand Tower"));
    let names: Vec<&str> = r.brief.goals.iter().map(Goal::name).collect();
    assert_eq!(r.layers.iter().map(|l| l.goal.as_str()).collect::<Vec<_>>(), names, "one layer per goal, same order");
    for (l, g) in r.layers.iter().zip(&r.brief.goals) {
        assert_eq!((l.weight, l.must), (g.weight_or_default(), g.must));
        assert_eq!((l.heat.width, l.heat.height, l.heat.cell_m), (r.heat.width, r.heat.height, r.heat.cell_m), "same grid as heat");
        assert!(l.heat.values.iter().zip(&r.heat.values).all(|(a, b)| b.is_some() || a.is_none()), "null where nothing was searched");
        assert!(r.heat.values.iter().zip(&l.heat.values).any(|(b, a)| b.is_some() && a.is_none()) || !r.stages.iter().any(|s| s.goal.is_some()), "null where an earlier gate stopped the funnel");
        assert!(l.heat.values.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
    }
    assert!(r.stages.windows(2).all(|w| w[0].n >= w[1].n), "the funnel only narrows");
    assert!(r.objects.iter().any(|o| o.id == "costar:52227" && o.name.as_deref() == Some("Grand Tower")), "the tower the goals name is on the map");
    assert_eq!(r.stages[1].n, r.searched, "'somewhere to stand' is what was searched");
    assert!(r.stages.iter().filter_map(|s| s.goal.as_deref()).all(|g| names.contains(&g)));
    let notes: Vec<Verdict> = r.spots.iter().filter_map(|s| s.investigation.as_ref().map(|i| i.verdict)).collect();
    assert_eq!(notes, [Verdict::Confirmed, Verdict::Caution, Verdict::Confirmed, Verdict::Caution, Verdict::Confirmed]);

    // the browser's rebuild gives back the server's heat and spots
    for s in &r.spots {
        let at_spot = |&i: &usize| { let (a, b) = cell_latlon(&r.heat, i); (a - s.lat).abs() < 1e-5 && (b - s.lon).abs() < 1e-5 };
        let i = (0..r.heat.values.len()).find(at_spot).expect("each spot is a cell centre");
        assert!(r.layers.iter().all(|l| l.heat.values[i].is_some()), "every goal is scored at spot {}", s.rank);
    }
    let (heat, picked) = rebuild(&r.heat, &r.layers, r.spacing_m.unwrap(), r.brief.max_spots as usize);
    for (mine, theirs) in heat.iter().zip(&r.heat.values) {
        assert_eq!(mine.is_some(), theirs.is_some());
        if let (Some(a), Some(b)) = (mine, theirs) {
            assert!((a - f64::from(*b)).abs() < 1e-3, "heat {a} vs {b}");
        }
    }
    let at: Vec<(f64, f64)> = picked.iter().map(|&i| cell_latlon(&r.heat, i)).collect();
    assert_eq!(at.len(), r.spots.len());
    for (s, (lat, lon)) in r.spots.iter().zip(at) {
        assert!((s.lat - lat).abs() < 1e-5 && (s.lon - lon).abs() < 1e-5, "spot {} is the cell the browser picks", s.rank);
    }

    // weight edits only (a must switched off needs a new search): turn the roof preference off
    let mut sun_only = r.layers.clone();
    sun_only[3].weight = 0.0;
    let (_, picked2) = rebuild(&r.heat, &sun_only, r.spacing_m.unwrap(), 3);
    assert_eq!(picked2.len(), 3);
    assert_ne!(picked2, picked[..3], "re-weighting changes the top spots");
}

#[test]
fn call_sheet_carries_its_search() {
    let mut sheet: CallSheet = load("call_sheet.json");
    assert!(sheet.search.is_none(), "older sheets have no search");
    sheet.search = Some(load("search_layers.json"));
    let back: CallSheet = serde_json::from_str(&serde_json::to_string(&sheet).unwrap()).unwrap();
    assert_eq!(back, sheet);
    let old: SearchResult = load("search_result.json");
    assert!(old.layers.is_empty() && old.stages.is_empty() && old.spacing_m.is_none(), "older results still parse");
    assert!(!serde_json::to_string(&old).unwrap().contains("layers"), "left out when empty");
    let bad = r#"{"verdict":"maybe","note":"x"}"#;
    assert!(serde_json::from_str::<Investigation>(bad).is_err(), "verdicts are a fixed set");
}
