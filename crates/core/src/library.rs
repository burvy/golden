//! Turning a brief's goals into checked cost functions (docs/metrics.md v2).
//!
//! The seed library ships in `library/*.json` at the repo root and is compiled in here, so a
//! search with no library at hand (and every brief without goals) behaves as before. The server
//! adds saved entries on top and passes the whole list to `search::search_with`.

use contracts::{Brief, CostFn, Framing, Goal, GoalFn, ObjArg, ObjRef};

use crate::expr::{check, Checked, ExprError};

/// The seed entries, as shipped in `library/`.
const SEEDS: &[(&str, &str)] = &[
    ("sun_clear.json", include_str!("../../../library/sun_clear.json")),
    ("sun_between.json", include_str!("../../../library/sun_between.json")),
    ("sun_behind.json", include_str!("../../../library/sun_behind.json")),
    ("in_front.json", include_str!("../../../library/in_front.json")),
    ("lined_up.json", include_str!("../../../library/lined_up.json")),
    ("between.json", include_str!("../../../library/between.json")),
    ("beside.json", include_str!("../../../library/beside.json")),
    ("fills.json", include_str!("../../../library/fills.json")),
    // Proven in the library lab (docs/library-lab.md, promote list): 100% recall@10 in every tested run.
    ("open_horizon.json", include_str!("../../../library/open_horizon.json")),
    ("sun_wide_gap.json", include_str!("../../../library/sun_wide_gap.json")),
    ("sunset_behind.json", include_str!("../../../library/sunset_behind.json")),
    ("sunset_beside.json", include_str!("../../../library/sunset_beside.json")),
    ("sunset_between_two.json", include_str!("../../../library/sunset_between_two.json")),
    ("tower_lit_at_sunset.json", include_str!("../../../library/tower_lit_at_sunset.json")),
    ("sunrise_between_two.json", include_str!("../../../library/sunrise_between_two.json")),
    ("tower_lit_at_sunrise.json", include_str!("../../../library/tower_lit_at_sunrise.json")),
    ("sunrise_beside.json", include_str!("../../../library/sunrise_beside.json")),
    ("sunrise_behind.json", include_str!("../../../library/sunrise_behind.json")),
    ("towers_over.json", include_str!("../../../library/towers_over.json")),
    ("sun_down_the_street.json", include_str!("../../../library/sun_down_the_street.json")),
    ("sun_keyhole.json", include_str!("../../../library/sun_keyhole.json")),
    ("skyline_glow.json", include_str!("../../../library/skyline_glow.json")),
    ("skyline_glow_at_sunrise.json", include_str!("../../../library/skyline_glow_at_sunrise.json")),
];

/// The built-in cost functions.
pub fn seeds() -> Vec<CostFn> {
    SEEDS
        .iter()
        .map(|(file, text)| serde_json::from_str(text).unwrap_or_else(|e| panic!("library/{file}: {e}")))
        .collect()
}

/// The library entry a framing used to mean: what a brief without goals asks for.
pub fn sun_goal_name(framing: Framing) -> &'static str {
    match framing {
        Framing::Clear => "sun_clear",
        Framing::Between => "sun_between",
        Framing::Behind => "sun_behind",
    }
}

/// The brief's goals, or `[sun_<framing>]` when it has none.
pub fn goals_of(brief: &Brief) -> Vec<Goal> {
    if brief.goals.is_empty() {
        vec![Goal::using(sun_goal_name(brief.framing))]
    } else {
        brief.goals.clone()
    }
}

/// A goal ready for the funnel.
#[derive(Debug, Clone, PartialEq)]
pub struct Ready {
    pub name: String,
    pub cost: Checked,
    /// `cost.refs` with the goal's `args` filled in: the object reference for each slot.
    pub refs: Vec<ObjRef>,
    pub weight: f64,
    pub must: bool,
}

/// Check every goal against `library`. Errors are prefixed with the goal's place (`goals[1].score...`).
pub fn ready(goals: &[Goal], library: &[CostFn]) -> Result<Vec<Ready>, Vec<ExprError>> {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (i, goal) in goals.iter().enumerate() {
        let at = format!("goals[{i}]");
        match ready_one(goal, library, &at) {
            Ok((cost, refs)) => {
                out.push(Ready { name: goal.name().to_string(), cost, refs, weight: goal.weight_or_default(), must: goal.must })
            }
            Err(mut e) => errors.append(&mut e),
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

fn ready_one(goal: &Goal, library: &[CostFn], at: &str) -> Result<(Checked, Vec<ObjRef>), Vec<ExprError>> {
    let err = |path: String, msg: String| vec![ExprError { at: path, msg }];
    let none = std::collections::BTreeMap::new();
    let (cost, path, args) = match &goal.cost {
        GoalFn::Def(f) if !f.params.is_empty() => {
            return Err(err(format!("{at}.def.params"), "an inline cost function gets no args, so it can't have params; write the objects into the expression, like {\"kind\": \"tree\"}".into()));
        }
        GoalFn::Def(f) => (f, format!("{at}.def"), &none),
        GoalFn::Use { name, args } => {
            let Some(f) = library.iter().find(|f| &f.name == name) else {
                let names: Vec<&str> = library.iter().map(|f| f.name.as_str()).collect();
                return Err(err(format!("{at}.use"), format!("no cost function \"{name}\" in the library; it has {}", names.join(", "))));
            };
            if let Some(extra) = args.keys().find(|k| !f.params.contains(k)) {
                return Err(err(format!("{at}.args"), format!("\"{name}\" has no parameter \"{extra}\" (params: {:?})", f.params)));
            }
            if let Some(missing) = f.params.iter().find(|p| !args.contains_key(*p)) {
                return Err(err(format!("{at}.args"), format!("\"{name}\" needs an object for \"{missing}\"")));
            }
            (f, at.to_string(), args)
        }
    };
    let checked = check(cost).map_err(|errs| errs.into_iter().map(|e| ExprError { at: format!("{path}.{}", e.at), msg: e.msg }).collect::<Vec<_>>())?;
    let refs = checked
        .refs
        .iter()
        .map(|r| match r {
            ObjArg::Param(p) => args[p].clone(), // check() and the args check above guarantee it
            ObjArg::Ref(o) => o.clone(),
        })
        .collect();
    Ok((checked, refs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::Tier;

    #[test]
    fn seeds_parse_check_and_match_their_file_names() {
        let lib = seeds();
        assert_eq!(lib.len(), SEEDS.len());
        for (f, (file, _)) in lib.iter().zip(SEEDS) {
            assert_eq!(format!("{}.json", f.name), *file);
            let c = check(f).unwrap_or_else(|e| panic!("{file}: {e:?}"));
            // Sun-only seeds read the skyline; relation seeds need at least a ray (line of sight).
            if f.params.is_empty() {
                assert_eq!(c.tier(), Tier::Skyline, "{file}");
            } else {
                assert!(c.tier() >= Tier::Ray, "{file}: {:?}", c.tier());
            }
            assert_eq!(c.refs.len(), f.params.len(), "{file}: relation seeds use every param, and only params");
        }
        for framing in [Framing::Clear, Framing::Between, Framing::Behind] {
            assert!(lib.iter().any(|f| f.name == sun_goal_name(framing)));
        }
    }

    #[test]
    fn the_lab_promote_list_is_seeded_and_each_about_names_its_brief() {
        // docs/library-lab.md, promote list: tasks 070 (the first 11) and 068 (the last 4).
        let promoted = [
            "open_horizon", "sun_wide_gap", "sun_down_the_street", "sun_keyhole",
            "sunset_behind", "sunset_beside", "sunset_between_two", "tower_lit_at_sunset",
            "sunrise_between_two", "tower_lit_at_sunrise", "sunrise_beside", "sunrise_behind",
            "skyline_glow", "skyline_glow_at_sunrise", "towers_over",
        ];
        let lib = seeds();
        for name in promoted {
            let f = lib.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{name} is not a seed"));
            assert!(f.about.contains("For briefs like '"), "{name}: {}", f.about);
        }
    }

    #[test]
    fn the_street_and_keyhole_shots_are_sun_only_and_the_glow_needs_two_objects() {
        let lib = seeds();
        let get = |n: &str| lib.iter().find(|f| f.name == n).unwrap();
        for n in ["sun_down_the_street", "sun_keyhole"] {
            assert!(get(n).params.is_empty(), "{n}");
            assert!(get(n).must.is_some(), "{n}");
        }
        for n in ["skyline_glow", "skyline_glow_at_sunrise"] {
            assert_eq!(get(n).params, ["a", "b"], "{n}");
        }
        // down the street's must is Point tier (no roof), so it drops points before any trace.
        let street = check(get("sun_down_the_street")).unwrap();
        assert_eq!(street.must.as_ref().map(|m| m.tier()), Some(Tier::Point));
    }

    #[test]
    fn goal_errors_name_the_goal() {
        let lib = seeds();
        let goals: Vec<Goal> = serde_json::from_str(
            r#"[{"use":"sun_clear"},{"use":"sun_sideways"},{"use":"sun_clear","args":{"a":{"id":"x"}}},
                {"def":{"name":"x","about":"x","score":{"f":"nope"}}}]"#,
        )
        .unwrap();
        let errs = ready(&goals, &lib).unwrap_err();
        let text: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        assert_eq!(text.len(), 3, "{text:?}");
        assert!(text[0].starts_with("goals[1].use: no cost function \"sun_sideways\""), "{text:?}");
        assert!(text[1].starts_with("goals[2].args: \"sun_clear\" has no parameter \"a\""), "{text:?}");
        assert!(text[2].starts_with("goals[3].def.score: unknown feature \"nope\""), "{text:?}");
    }

    #[test]
    fn args_fill_the_slots() {
        let goals: Vec<Goal> = serde_json::from_str(
            r#"[{"use":"in_front","args":{"a":{"kind":"tree"},"b":{"id":"costar:1"}}},
                {"def":{"name":"x","about":"x","params":["a"],"score":{"f":"dist","obj":"a"}}}]"#,
        )
        .unwrap();
        let errs = ready(&goals, &seeds()).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].to_string().starts_with("goals[1].def.params: an inline cost function gets no args"), "{errs:?}");
        let r = ready(&goals[..1], &seeds()).unwrap();
        let want: Vec<ObjRef> = serde_json::from_str(r#"[{"kind":"tree"},{"id":"costar:1"}]"#).unwrap();
        assert_eq!(r[0].refs, want);
    }

    #[test]
    fn weights_default_by_kind() {
        let goals: Vec<Goal> = serde_json::from_str(r#"[{"use":"sun_clear"},{"use":"sun_behind","must":true},{"use":"sun_between","weight":0.5}]"#).unwrap();
        let r = ready(&goals, &seeds()).unwrap();
        assert_eq!(r.iter().map(|g| (g.weight, g.must)).collect::<Vec<_>>(), [(1.0, false), (0.0, true), (0.5, false)]);
    }
}
