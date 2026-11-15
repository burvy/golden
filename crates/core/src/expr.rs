//! Cost-function expressions: check, tier, evaluate (docs/metrics.md v2).
//!
//! `contracts::Expr` is the JSON shape (parsing already rejects unknown ops and missing keys).
//! `check` turns a `CostFn` into a `Checked` one, with every feature name resolved, or returns
//! every problem with its place in the expression, worded so the agent can fix it. A `Checked`
//! function knows its tier and evaluates on one candidate through a feature lookup the caller
//! provides, so this module needs no grid, no sun and no objects.
//!
//! The objects a function names (`"obj": "a"` for a parameter, or an inline `{"kind": ...}`) are
//! collected into `Checked::refs`, once each; a `Call` points into that list by index. Which real
//! object each one is gets decided later, per goal (`objects::bind`).

use contracts::{CmpOp, CostFn, Expr, ListOp, ObjArg};

use crate::features::{Call, Feature, Slot, Takes, Tier, FEATURES};
use crate::objects::MAX_OBJECTS_PER_FN;

/// A cost function whose features are all known. Evaluate with `eval`.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub name: String,
    pub about: String,
    /// The objects the expressions name, each once; `Call::objs` index this.
    pub refs: Vec<ObjArg>,
    pub must: Option<Node>,
    pub score: Node,
}

impl Checked {
    /// The most expensive tier any part of the function needs.
    pub fn tier(&self) -> Tier {
        self.must.as_ref().map_or(Tier::Point, Node::tier).max(self.score.tier())
    }

    /// How `refs[slot]` reads in a reason: the parameter name, the id, or the kind.
    pub fn label(&self, slot: Slot) -> String {
        match &self.refs[usize::from(slot)] {
            ObjArg::Param(p) => p.clone(),
            ObjArg::Ref(contracts::ObjRef::Id(id)) => id.clone(),
            ObjArg::Ref(contracts::ObjRef::Kind { kind, .. }) => kind.clone(),
        }
    }

    /// `dist(a)`, `bearing_diff(a, tree)`, `roof`: a call as it reads in a reason.
    pub fn call_text(&self, c: Call) -> String {
        let args: Vec<String> = c.objs.iter().flatten().map(|&s| self.label(s)).collect();
        if args.is_empty() {
            c.f.name().to_string()
        } else {
            format!("{}({})", c.f.name(), args.join(", "))
        }
    }
}

/// A checked expression: `contracts::Expr` with features resolved. Leaves are numbers or features;
/// a later AI leaf (`{"ai": {question, image}}`, task 058) would resolve to a feature-like key whose
/// value the caller's lookup supplies, so `eval` and `tier` stay as they are.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Feat(Call),
    Num(f64),
    Ramp { x: Box<Node>, lo: f64, hi: f64 },
    Bump { x: Box<Node>, at: f64, width: f64 },
    List { op: ListOp, of: Vec<Node> },
    Not(Box<Node>),
    Cmp { op: CmpOp, x: Box<Node>, than: Box<Node> },
}

/// One problem in a cost function. `at` is a path like `score.of[1].x`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprError {
    pub at: String,
    pub msg: String,
}

impl std::fmt::Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.at, self.msg)
    }
}

/// Check a cost function: its name, its params and every feature it uses.
pub fn check(cost: &CostFn) -> Result<Checked, Vec<ExprError>> {
    let mut errors = Vec::new();
    let err = |at: &str, msg: String| ExprError { at: at.to_string(), msg };
    if cost.name.is_empty() || !cost.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        errors.push(err("name", format!("{:?} should be snake_case: a-z, 0-9 and _", cost.name)));
    }
    if cost.about.trim().is_empty() {
        errors.push(err("about", "say in one line what the function looks for".into()));
    }
    for (i, p) in cost.params.iter().enumerate() {
        if p.is_empty() || cost.params[..i].contains(p) {
            errors.push(err(&format!("params[{i}]"), format!("{p:?} is empty or repeated")));
        }
    }
    let mut c = Compiler { params: &cost.params, refs: Vec::new(), errors };
    let must = cost.must.as_ref().map(|e| c.compile(e, "must"));
    let score = c.compile(&cost.score, "score");
    let Compiler { refs, errors, .. } = c;
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Checked { name: cost.name.clone(), about: cost.about.clone(), refs, must, score })
}

/// Walks an expression, resolving features and collecting the objects it names.
struct Compiler<'a> {
    params: &'a [String],
    refs: Vec<ObjArg>,
    errors: Vec<ExprError>,
}

impl Compiler<'_> {
    fn compile(&mut self, e: &Expr, at: &str) -> Node {
        let sub = |c: &mut Self, x: &Expr, key: &str| Box::new(c.compile(x, &format!("{at}.{key}")));
        match e {
            Expr::Feature { f, obj, a, b } => match self.feature(f, obj, a, b) {
                Ok(call) => Node::Feat(call),
                Err(msg) => {
                    self.errors.push(ExprError { at: at.to_string(), msg });
                    Node::Num(0.0)
                }
            },
            Expr::Num(n) => Node::Num(*n),
            Expr::Ramp { x, lo, hi } => Node::Ramp { x: sub(self, x, "x"), lo: *lo, hi: *hi },
            Expr::Bump { x, at: c, width } => Node::Bump { x: sub(self, x, "x"), at: *c, width: *width },
            Expr::List { op, of } => {
                let of = of.iter().enumerate().map(|(i, x)| self.compile(x, &format!("{at}.of[{i}]"))).collect();
                Node::List { op: *op, of }
            }
            Expr::Not(x) => Node::Not(sub(self, x, "x")),
            Expr::Cmp { op, x, than } => Node::Cmp { op: *op, x: sub(self, x, "x"), than: sub(self, than, "than") },
        }
    }

    /// Resolve a feature name and check (and collect) its objects.
    fn feature(&mut self, name: &str, obj: &Option<ObjArg>, a: &Option<ObjArg>, b: &Option<ObjArg>) -> Result<Call, String> {
        let Some(f) = Feature::by_name(name) else {
            return Err(format!("unknown feature \"{name}\"; features are {}", feature_names()));
        };
        let objs = match (f.info().takes, obj, a, b) {
            (Takes::Nothing, None, None, None) => [None, None],
            (Takes::Nothing, ..) => return Err(format!("feature \"{name}\" takes no object (obj, a, b)")),
            (Takes::One, Some(o), None, None) => [Some(self.slot(o)?), None],
            (Takes::One, ..) => {
                return Err(format!("feature \"{name}\" takes one object as \"obj\": a parameter like \"a\" or {{\"kind\": \"tree\"}}"))
            }
            (Takes::Two, None, Some(x), Some(y)) => [Some(self.slot(x)?), Some(self.slot(y)?)],
            (Takes::Two, ..) => return Err(format!("feature \"{name}\" takes two objects as \"a\" and \"b\"")),
        };
        Ok(Call { f, objs })
    }

    /// The slot of an object argument, adding it if it's new. A parameter must be declared.
    fn slot(&mut self, o: &ObjArg) -> Result<Slot, String> {
        if let ObjArg::Param(p) = o {
            if !self.params.contains(p) {
                return Err(if self.params.is_empty() {
                    format!("\"{p}\" isn't a parameter (this function has none); name the object inline, like {{\"kind\": \"tree\"}}")
                } else {
                    format!("\"{p}\" isn't a parameter (params: {})", self.params.join(", "))
                });
            }
        }
        if let Some(i) = self.refs.iter().position(|r| r == o) {
            return Ok(i as Slot);
        }
        if self.refs.len() == MAX_OBJECTS_PER_FN {
            return Err(format!("a cost function can name at most {MAX_OBJECTS_PER_FN} objects"));
        }
        self.refs.push(o.clone());
        Ok((self.refs.len() - 1) as Slot)
    }
}

fn feature_names() -> String {
    FEATURES.iter().map(|i| i.name).collect::<Vec<_>>().join(", ")
}

impl Node {
    /// The most expensive tier among the features in this expression (Point if it has none).
    pub fn tier(&self) -> Tier {
        let mut tier = Tier::Point;
        self.each_feature(&mut |c| tier = tier.max(c.f.tier()));
        tier
    }

    /// Every feature call in the expression, depth first, repeats included.
    pub fn each_feature(&self, visit: &mut impl FnMut(Call)) {
        match self {
            Node::Feat(f) => visit(*f),
            Node::Num(_) => {}
            Node::Ramp { x, .. } | Node::Bump { x, .. } | Node::Not(x) => x.each_feature(visit),
            Node::List { of, .. } => of.iter().for_each(|x| x.each_feature(visit)),
            Node::Cmp { x, than, .. } => {
                x.each_feature(visit);
                than.each_feature(visit);
            }
        }
    }
}

/// The value of `node` on one candidate, with `value(call)` giving each feature's number.
/// Not clamped; see `score_of` for a 0..1 score.
pub fn eval(node: &Node, value: &mut impl FnMut(Call) -> f64) -> f64 {
    match node {
        Node::Feat(f) => value(*f),
        Node::Num(n) => *n,
        Node::Ramp { x, lo, hi } => crate::score::ramp(eval(x, value), *lo, *hi),
        Node::Bump { x, at, width } => (1.0 - (eval(x, value) - at).abs() / width).clamp(0.0, 1.0),
        Node::List { op, of } => {
            let xs = of.iter().map(|x| eval(x, value));
            match op {
                ListOp::And | ListOp::Min => xs.fold(f64::INFINITY, f64::min),
                ListOp::Or | ListOp::Max => xs.fold(f64::NEG_INFINITY, f64::max),
                ListOp::Mul => xs.product(),
                ListOp::Add => xs.sum(),
                ListOp::Avg => xs.sum::<f64>() / of.len() as f64,
            }
        }
        Node::Not(x) => 1.0 - clamp01(eval(x, value)),
        Node::Cmp { op, x, than } => {
            let (a, b) = (eval(x, value), eval(than, value));
            let yes = match op {
                CmpOp::Lt => a < b,
                CmpOp::Gt => a > b,
            };
            if yes {
                1.0
            } else {
                0.0
            }
        }
    }
}

/// `eval`, clamped to 0..1 (anything not finite counts as 0).
pub fn score_of(node: &Node, value: &mut impl FnMut(Call) -> f64) -> f64 {
    clamp01(eval(node, value))
}

fn clamp01(x: f64) -> f64 {
    if x.is_finite() {
        x.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(json: &str) -> CostFn {
        serde_json::from_str(json).unwrap()
    }

    fn run(score: &str, value: impl Fn(Feature) -> f64) -> f64 {
        let c = check(&cost(&format!(r#"{{"name":"t","about":"t","score":{score}}}"#))).unwrap();
        score_of(&c.score, &mut |c: Call| value(c.f))
    }

    #[test]
    fn ops_evaluate() {
        let none = |_| 0.0;
        assert_eq!(run(r#"{"num":0.25}"#, none), 0.25);
        assert_eq!(run(r#"{"op":"ramp","x":{"num":15},"lo":20,"hi":10}"#, none), 0.5);
        assert_eq!(run(r#"{"op":"ramp","x":{"num":5},"lo":20,"hi":10}"#, none), 1.0);
        assert_eq!(run(r#"{"op":"ramp","x":{"num":99},"lo":20,"hi":10}"#, none), 0.0);
        assert_eq!(run(r#"{"op":"bump","x":{"num":-5},"at":0,"width":10}"#, none), 0.5);
        assert_eq!(run(r#"{"op":"bump","x":{"num":30},"at":0,"width":10}"#, none), 0.0);
        assert_eq!(run(r#"{"op":"and","of":[{"num":0.3},{"num":0.8}]}"#, none), 0.3);
        assert_eq!(run(r#"{"op":"or","of":[{"num":0.3},{"num":0.8}]}"#, none), 0.8);
        assert_eq!(run(r#"{"op":"mul","of":[{"num":0.5},{"num":0.5}]}"#, none), 0.25);
        assert_eq!(run(r#"{"op":"add","of":[{"num":0.5},{"num":0.75}]}"#, none), 1.0, "clamped");
        assert_eq!(run(r#"{"op":"avg","of":[{"num":0.5},{"num":1}]}"#, none), 0.75);
        assert_eq!(run(r#"{"op":"not","x":{"num":0.25}}"#, none), 0.75);
        assert_eq!(run(r#"{"op":"lt","x":{"num":1},"than":{"num":2}}"#, none), 1.0);
        assert_eq!(run(r#"{"op":"gt","x":{"num":1},"than":{"num":2}}"#, none), 0.0);
    }

    #[test]
    fn features_come_from_the_lookup() {
        let v = |f| if f == Feature::GapWidthDeg { 4.0 } else { 1.0 };
        assert_eq!(run(r#"{"op":"bump","x":{"f":"gap_width_deg"},"at":3,"width":2}"#, v), 0.5);
        assert_eq!(run(r#"{"op":"add","of":[{"f":"roof"},{"num":-2}]}"#, v), 0.0, "negative clamps to 0");
    }

    #[test]
    fn tier_is_the_most_expensive_feature() {
        let c = check(&cost(r#"{"name":"t","about":"t","must":{"f":"roof"},"score":{"num":1}}"#)).unwrap();
        assert_eq!((c.must.as_ref().unwrap().tier(), c.score.tier(), c.tier()), (Tier::Point, Tier::Point, Tier::Point));
        let c = check(&cost(r#"{"name":"t","about":"t","score":{"op":"and","of":[{"f":"roof"},{"f":"sun_clear"}]}}"#)).unwrap();
        assert_eq!(c.tier(), Tier::Skyline);
    }

    #[test]
    fn errors_say_where_and_what() {
        let bad = cost(
            r#"{"name":"Bad Name","about":" ","params":["a","a"],
                "must":{"f":"sun_clear","obj":"a"},
                "score":{"op":"and","of":[{"f":"roof"},{"f":"sunclear"},{"op":"not","x":{"f":"dist","obj":"c"}},
                    {"f":"dist","a":"a"},{"f":"bearing_diff","a":"a"}]}}"#,
        );
        let errs = check(&bad).unwrap_err();
        let text: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        let has = |at: &str, says: &str| text.iter().any(|t| t.starts_with(&format!("{at}: ")) && t.contains(says));
        assert!(has("name", "snake_case"), "{text:?}");
        assert!(has("about", "one line"), "{text:?}");
        assert!(has("params[1]", "repeated"), "{text:?}");
        assert!(has("must", "takes no object"), "{text:?}");
        assert!(has("score.of[1]", "unknown feature \"sunclear\"; features are ground_m"), "{text:?}");
        assert!(has("score.of[2].x", "\"c\" isn't a parameter (params: a, a)"), "{text:?}");
        assert!(has("score.of[3]", "takes one object as \"obj\""), "{text:?}");
        assert!(has("score.of[4]", "takes two objects as \"a\" and \"b\""), "{text:?}");
        assert_eq!(errs.len(), 8);
    }

    #[test]
    fn objects_are_collected_once_each() {
        let c = check(&cost(
            r#"{"name":"t","about":"t","params":["a","b"],
                "must":{"op":"lt","x":{"f":"dist","obj":"a"},"than":{"f":"dist","obj":"b"}},
                "score":{"op":"avg","of":[{"f":"visible","obj":"b"},{"f":"bearing_diff","a":"a","b":{"kind":"tree"}},
                    {"f":"size_deg","obj":{"kind":"tree"}}]}}"#,
        ))
        .unwrap();
        let tree: ObjArg = serde_json::from_str(r#"{"kind":"tree"}"#).unwrap();
        assert_eq!(c.refs, vec![ObjArg::Param("a".into()), ObjArg::Param("b".into()), tree]);
        let mut calls = Vec::new();
        c.score.each_feature(&mut |k| calls.push(c.call_text(k)));
        assert_eq!(calls, ["visible(b)", "bearing_diff(a, tree)", "size_deg(tree)"]);
        assert_eq!(c.tier(), Tier::Ray);
        let no_params = check(&cost(r#"{"name":"t","about":"t","score":{"f":"dist","obj":"a"}}"#)).unwrap_err();
        assert!(no_params[0].msg.contains("this function has none"), "{no_params:?}");
    }
}
