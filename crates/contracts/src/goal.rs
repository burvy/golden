//! Goals, cost functions and their expression language (docs/contracts.md#goals, docs/metrics.md v2).
//!
//! A brief carries a list of `Goal`s. Each goal names a cost function from the library (`use`) or
//! brings one inline (`def`). A `CostFn` is plain data: a small JSON expression tree (`Expr`) over
//! features that core computes for each candidate standpoint. This module only holds the shapes and
//! their structural checks (one of `use`/`def`, a known `op`, the fields an op needs). Whether a
//! feature or parameter name exists is checked by `core::expr`, which knows the features.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Default lens angle for "in one frame" (about 30 mm on full frame). A brief can override it.
pub const DEFAULT_LENS_DEG: f64 = 60.0;

/// Object kinds we have checked against a source (Vexcel detection, task 049, plus CoStar buildings).
/// A kind is just its label string; adding one is one line here.
pub const OBJECT_KINDS: &[&str] = &["building", "tree", "car", "rooftop HVAC unit", "swimming pool", "water tower"];

// ---- goals ----------------------------------------------------------------------------------

/// One thing the shot should do. JSON: `{"use": "sun_between", "args": {...}, "weight": 1}` or
/// `{"def": {...CostFn...}, "must": true}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawGoal", into = "RawGoal")]
pub struct Goal {
    pub cost: GoalFn,
    /// Share of the ranking score. `None` means 1 for a soft goal and 0 for a must.
    pub weight: Option<f64>,
    /// A gate: a candidate scoring 0 on this goal is dropped.
    pub must: bool,
}

/// Where a goal's cost function comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum GoalFn {
    /// A library entry by name, with its object parameters filled in.
    Use { name: String, args: BTreeMap<String, ObjRef> },
    /// A cost function written for this brief (not saved yet).
    Def(CostFn),
}

impl Goal {
    /// A soft goal using a library entry with no arguments, e.g. `Goal::using("sun_clear")`.
    pub fn using(name: &str) -> Goal {
        Goal { cost: GoalFn::Use { name: name.to_string(), args: BTreeMap::new() }, weight: None, must: false }
    }

    /// The name shown in `Spot.scores[].goal`: the library name, or the inline function's name.
    pub fn name(&self) -> &str {
        match &self.cost {
            GoalFn::Use { name, .. } => name,
            GoalFn::Def(f) => &f.name,
        }
    }

    /// The weight in the ranking sum: as given, else 1 for a soft goal and 0 for a must.
    pub fn weight_or_default(&self) -> f64 {
        self.weight.unwrap_or(if self.must { 0.0 } else { 1.0 })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGoal {
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    use_: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    args: BTreeMap<String, ObjRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    def: Option<CostFn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weight: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    must: bool,
}

impl TryFrom<RawGoal> for Goal {
    type Error = String;

    fn try_from(r: RawGoal) -> Result<Goal, String> {
        if let Some(w) = r.weight {
            if !(w.is_finite() && w >= 0.0) {
                return Err(format!("goal weight must be a number >= 0, got {w}"));
            }
        }
        let cost = match (r.use_, r.def) {
            (Some(name), None) => GoalFn::Use { name, args: r.args },
            (None, Some(f)) if r.args.is_empty() => GoalFn::Def(f),
            (None, Some(_)) => return Err("a goal with \"def\" takes no \"args\"; write the objects into the expression".into()),
            (Some(_), Some(_)) => return Err("a goal has either \"use\" (a library name) or \"def\" (a cost function), not both".into()),
            (None, None) => return Err("a goal needs \"use\" (a library name) or \"def\" (a cost function)".into()),
        };
        Ok(Goal { cost, weight: r.weight, must: r.must })
    }
}

impl From<Goal> for RawGoal {
    fn from(g: Goal) -> RawGoal {
        let (use_, args, def) = match g.cost {
            GoalFn::Use { name, args } => (Some(name), args, None),
            GoalFn::Def(f) => (None, BTreeMap::new(), Some(f)),
        };
        RawGoal { use_, args, def, weight: g.weight, must: g.must }
    }
}

// ---- cost functions -------------------------------------------------------------------------

/// A library entry: a named, documented expression. Seeds live in `library/*.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostFn {
    /// snake_case, unique in the library, e.g. `"fountain_in_front_of_tower"`.
    pub name: String,
    /// One line for people and the agent; also the start of each spot's `why` for this goal.
    pub about: String,
    /// Names of the object parameters a goal fills in with `args`, e.g. `["a", "b"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<String>,
    /// Optional gate: a candidate where this is 0 is dropped, whatever the goal's weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub must: Option<Expr>,
    /// 0..1 (clamped), higher is better.
    pub score: Expr,
}

// ---- objects --------------------------------------------------------------------------------

/// Which object a feature looks at: one by id, or the best one of a kind (optionally near another).
/// JSON: `{"id": "costar:123"}` or `{"kind": "tree", "near": {"id": "costar:123"}, "within_m": 300}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawObjRef", into = "RawObjRef")]
pub enum ObjRef {
    Id(String),
    Kind { kind: String, near: Option<Box<ObjRef>>, within_m: Option<f64> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawObjRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    near: Option<Box<ObjRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    within_m: Option<f64>,
}

impl TryFrom<RawObjRef> for ObjRef {
    type Error = String;

    fn try_from(r: RawObjRef) -> Result<ObjRef, String> {
        match (r.id, r.kind) {
            (Some(id), None) if r.near.is_none() && r.within_m.is_none() => Ok(ObjRef::Id(id)),
            (Some(_), None) => Err("an object by \"id\" takes no \"near\" or \"within_m\"".into()),
            (None, Some(kind)) => {
                if let Some(m) = r.within_m {
                    if !(m.is_finite() && m > 0.0) {
                        return Err(format!("within_m must be > 0 metres, got {m}"));
                    }
                }
                Ok(ObjRef::Kind { kind, near: r.near, within_m: r.within_m })
            }
            (Some(_), Some(_)) => Err("an object reference has \"id\" or \"kind\", not both".into()),
            (None, None) => Err("an object reference needs \"id\" or \"kind\"".into()),
        }
    }
}

impl From<ObjRef> for RawObjRef {
    fn from(o: ObjRef) -> RawObjRef {
        match o {
            ObjRef::Id(id) => RawObjRef { id: Some(id), kind: None, near: None, within_m: None },
            ObjRef::Kind { kind, near, within_m } => RawObjRef { id: None, kind: Some(kind), near, within_m },
        }
    }
}

/// An object argument inside an expression: a parameter name (`"a"`) or an inline reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ObjArg {
    Param(String),
    Ref(ObjRef),
}

/// A thing in the scene that features can measure from a standpoint. Comes from any source
/// (CoStar, Vexcel detection, OSM); core only sees this plain data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    /// Stable and source-prefixed, e.g. `"costar:1234567"`, `"vexcel:tree:34.05012,-118.25031"`.
    pub id: String,
    /// One of `OBJECT_KINDS`.
    pub kind: String,
    pub lat: f64,
    pub lon: f64,
    /// Ground height at the object's foot, and the height of its top, metres (same datum as `DsmGrid`).
    pub base_m: f64,
    pub top_m: f64,
    /// Rough footprint radius, metres.
    pub radius_m: f64,
    /// `"costar"`, `"vexcel"`, `"osm"`, ...
    pub source: String,
}

// ---- expressions ----------------------------------------------------------------------------

/// A small JSON expression that always evaluates to a number.
///
/// ```jsonc
/// {"f": "sun_visible_min"}                       // feature (obj/a/b for object features)
/// {"num": 0.5}
/// {"op": "ramp", "x": E, "lo": 20, "hi": 5}      // 0 at lo, 1 at hi, linear, clamped
/// {"op": "bump", "x": E, "at": 0, "width": 10}   // 1 at `at`, 0 beyond +-width
/// {"op": "and"|"or"|"min"|"max"|"mul"|"add"|"avg", "of": [E, ...]}
/// {"op": "not", "x": E}
/// {"op": "lt"|"gt", "x": E, "than": E}           // 0 or 1
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawExpr", into = "RawExpr")]
pub enum Expr {
    Feature { f: String, obj: Option<ObjArg>, a: Option<ObjArg>, b: Option<ObjArg> },
    Num(f64),
    Ramp { x: Box<Expr>, lo: f64, hi: f64 },
    Bump { x: Box<Expr>, at: f64, width: f64 },
    List { op: ListOp, of: Vec<Expr> },
    Not(Box<Expr>),
    Cmp { op: CmpOp, x: Box<Expr>, than: Box<Expr> },
}

/// Ops over a list. `and` = min, `or` = max on 0..1 values; kept separate so the intent reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListOp {
    And,
    Or,
    Min,
    Max,
    Mul,
    Add,
    Avg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Gt,
}

/// Every `op` name, for error messages and the agent's prompt.
pub const EXPR_OPS: &[&str] = &["ramp", "bump", "and", "or", "min", "max", "mul", "add", "avg", "not", "lt", "gt"];

impl ListOp {
    pub fn name(self) -> &'static str {
        match self {
            ListOp::And => "and",
            ListOp::Or => "or",
            ListOp::Min => "min",
            ListOp::Max => "max",
            ListOp::Mul => "mul",
            ListOp::Add => "add",
            ListOp::Avg => "avg",
        }
    }
}

impl CmpOp {
    pub fn name(self) -> &'static str {
        match self {
            CmpOp::Lt => "lt",
            CmpOp::Gt => "gt",
        }
    }
}

/// The JSON object as written: every key optional, sorted out by `TryFrom`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExpr {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    f: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    obj: Option<ObjArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    a: Option<ObjArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    b: Option<ObjArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    num: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    op: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x: Option<Box<Expr>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lo: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hi: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    of: Option<Vec<Expr>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    than: Option<Box<Expr>>,
}

impl RawExpr {
    /// The names of the keys that are set, to say which ones don't belong.
    fn keys(&self) -> Vec<&'static str> {
        let set = [
            ("f", self.f.is_some()),
            ("obj", self.obj.is_some()),
            ("a", self.a.is_some()),
            ("b", self.b.is_some()),
            ("num", self.num.is_some()),
            ("op", self.op.is_some()),
            ("x", self.x.is_some()),
            ("lo", self.lo.is_some()),
            ("hi", self.hi.is_some()),
            ("at", self.at.is_some()),
            ("width", self.width.is_some()),
            ("of", self.of.is_some()),
            ("than", self.than.is_some()),
        ];
        set.iter().filter(|(_, on)| *on).map(|(k, _)| *k).collect()
    }

    /// Error unless exactly `allowed` keys (and all of `required`) are set.
    fn only(&self, what: &str, required: &[&str], optional: &[&str]) -> Result<(), String> {
        let keys = self.keys();
        if let Some(extra) = keys.iter().find(|k| !required.contains(k) && !optional.contains(k)) {
            return Err(format!("{what} doesn't take \"{extra}\" (it takes {})", list(required, optional)));
        }
        if let Some(missing) = required.iter().find(|k| !keys.contains(k)) {
            return Err(format!("{what} needs \"{missing}\" (it takes {})", list(required, optional)));
        }
        Ok(())
    }
}

fn list(required: &[&str], optional: &[&str]) -> String {
    let opt = optional.iter().map(|k| format!("{k}?"));
    required.iter().map(|k| k.to_string()).chain(opt).collect::<Vec<_>>().join(", ")
}

fn finite(name: &str, v: Option<f64>) -> Result<f64, String> {
    match v {
        Some(x) if x.is_finite() => Ok(x),
        Some(x) => Err(format!("\"{name}\" must be a finite number, got {x}")),
        None => Err(format!("missing \"{name}\"")),
    }
}

impl TryFrom<RawExpr> for Expr {
    type Error = String;

    fn try_from(r: RawExpr) -> Result<Expr, String> {
        if r.f.is_some() {
            r.only("a feature", &["f"], &["obj", "a", "b"])?;
            return Ok(Expr::Feature { f: r.f.clone().unwrap(), obj: r.obj.clone(), a: r.a.clone(), b: r.b.clone() });
        }
        if r.num.is_some() {
            r.only("a number", &["num"], &[])?;
            return Ok(Expr::Num(finite("num", r.num)?));
        }
        let Some(op) = r.op.clone() else {
            return Err("an expression needs \"f\" (a feature), \"num\" (a number) or \"op\"".into());
        };
        let what = format!("op \"{op}\"");
        let list_op = |o: ListOp| -> Result<Expr, String> {
            r.only(&what, &["op", "of"], &[])?;
            let of = r.of.clone().unwrap();
            if of.is_empty() {
                return Err(format!("{what} needs at least one expression in \"of\""));
            }
            Ok(Expr::List { op: o, of })
        };
        let cmp = |o: CmpOp| -> Result<Expr, String> {
            r.only(&what, &["op", "x", "than"], &[])?;
            Ok(Expr::Cmp { op: o, x: r.x.clone().unwrap(), than: r.than.clone().unwrap() })
        };
        match op.as_str() {
            "ramp" => {
                r.only(&what, &["op", "x", "lo", "hi"], &[])?;
                let (lo, hi) = (finite("lo", r.lo)?, finite("hi", r.hi)?);
                if lo == hi {
                    return Err(format!("{what}: \"lo\" and \"hi\" must differ"));
                }
                Ok(Expr::Ramp { x: r.x.clone().unwrap(), lo, hi })
            }
            "bump" => {
                r.only(&what, &["op", "x", "at", "width"], &[])?;
                let width = finite("width", r.width)?;
                if width <= 0.0 {
                    return Err(format!("{what}: \"width\" must be > 0"));
                }
                Ok(Expr::Bump { x: r.x.clone().unwrap(), at: finite("at", r.at)?, width })
            }
            "not" => {
                r.only(&what, &["op", "x"], &[])?;
                Ok(Expr::Not(r.x.clone().unwrap()))
            }
            "and" => list_op(ListOp::And),
            "or" => list_op(ListOp::Or),
            "min" => list_op(ListOp::Min),
            "max" => list_op(ListOp::Max),
            "mul" => list_op(ListOp::Mul),
            "add" => list_op(ListOp::Add),
            "avg" => list_op(ListOp::Avg),
            "lt" => cmp(CmpOp::Lt),
            "gt" => cmp(CmpOp::Gt),
            other => Err(format!("unknown op \"{other}\"; ops are {}", EXPR_OPS.join(", "))),
        }
    }
}

impl From<Expr> for RawExpr {
    fn from(e: Expr) -> RawExpr {
        match e {
            Expr::Feature { f, obj, a, b } => RawExpr { f: Some(f), obj, a, b, ..RawExpr::default() },
            Expr::Num(n) => RawExpr { num: Some(n), ..RawExpr::default() },
            Expr::Ramp { x, lo, hi } => RawExpr { op: Some("ramp".into()), x: Some(x), lo: Some(lo), hi: Some(hi), ..RawExpr::default() },
            Expr::Bump { x, at, width } => {
                RawExpr { op: Some("bump".into()), x: Some(x), at: Some(at), width: Some(width), ..RawExpr::default() }
            }
            Expr::List { op, of } => RawExpr { op: Some(op.name().into()), of: Some(of), ..RawExpr::default() },
            Expr::Not(x) => RawExpr { op: Some("not".into()), x: Some(x), ..RawExpr::default() },
            Expr::Cmp { op, x, than } => RawExpr { op: Some(op.name().into()), x: Some(x), than: Some(than), ..RawExpr::default() },
        }
    }
}

// ---- results --------------------------------------------------------------------------------

/// How one spot did on one goal (`Spot.scores`, same order as the brief's goals).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rated {
    /// `Goal::name()`.
    pub goal: String,
    /// 0..1.
    pub score: f64,
    /// One line: the cost function's `about` plus the numbers behind the score.
    pub why: String,
}
