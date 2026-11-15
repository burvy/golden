//! Turning a brief's goals into checked cost functions (docs/metrics.md v2).
//!
//! The seed library ships in `library/*.json` at the repo root and is compiled in here, so a
//! search with no library at hand (and every brief without goals) behaves as before. The server
//! adds saved entries on top and passes the whole list to `search::search_with`.

use contracts::{Brief, CostFn, Framing, Goal, GoalFn};

use crate::expr::{check, Checked, ExprError};

/// The seed entries, as shipped in `library/`.
const SEEDS: &[(&str, &str)] = &[
    ("sun_clear.json", include_str!("../../../library/sun_clear.json")),
    ("sun_between.json", include_str!("../../../library/sun_between.json")),
    ("sun_behind.json", include_str!("../../../library/sun_behind.json")),
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
            Ok(cost) => out.push(Ready { name: goal.name().to_string(), cost, weight: goal.weight_or_default(), must: goal.must }),
            Err(mut e) => errors.append(&mut e),
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

fn ready_one(goal: &Goal, library: &[CostFn], at: &str) -> Result<Checked, Vec<ExprError>> {
    let err = |path: String, msg: String| vec![ExprError { at: path, msg }];
    let (cost, path) = match &goal.cost {
        GoalFn::Def(f) => (f, format!("{at}.def")),
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
            (f, at.to_string())
        }
    };
    check(cost).map_err(|errs| errs.into_iter().map(|e| ExprError { at: format!("{path}.{}", e.at), msg: e.msg }).collect())
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
            assert_eq!(check(f).unwrap().tier(), Tier::Skyline);
        }
        for framing in [Framing::Clear, Framing::Between, Framing::Behind] {
            assert!(lib.iter().any(|f| f.name == sun_goal_name(framing)));
        }
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
    fn weights_default_by_kind() {
        let goals: Vec<Goal> = serde_json::from_str(r#"[{"use":"sun_clear"},{"use":"sun_behind","must":true},{"use":"sun_between","weight":0.5}]"#).unwrap();
        let r = ready(&goals, &seeds()).unwrap();
        assert_eq!(r.iter().map(|g| (g.weight, g.must)).collect::<Vec<_>>(), [(1.0, false), (0.0, true), (0.5, false)]);
    }
}
