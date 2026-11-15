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
