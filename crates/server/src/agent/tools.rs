//! The agent's search step as a Claude tool loop (docs/metrics.md v2): Claude picks or writes the cost
//! functions for the brief, test-runs them, saves the good ones, searches, looks at the best spots and says
//! which to shoot. Every number comes from our tools; Claude only chooses and writes expressions.
//!
//! Tools: `list_cost_functions`, `define_cost_function`, `save_cost_function`, `find_objects`, `search`,
//! `investigate`, `finish`. The brief's place, date and event stay as read; the loop only chooses goals.
//!
//! Guards, so a confused or slow model can't break the sheet:
//! - at most `MAX_TURNS` Claude calls, `MAX_TOOL_CALLS` tool calls, `MAX_DEFINES` defines, `TIME_BUDGET` overall;
//! - tool errors go back to Claude verbatim as `is_error` results, so it can fix them;
//! - a refusal, a cut-off answer, an HTTP error, a loop that never searched or found nothing: the old fixed
//!   search (`search_with_replan`) runs instead and the log says why.
//!
//! Every tool call is logged (stderr and the sheet's steps): name, how long, and a one-line result.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use base64::Engine;
use contracts::{Brief, CostFn, Goal, ObjRef, Object, SearchResult, Spot, OBJECT_KINDS};
use golden_core::expr::check;
use golden_core::features::FEATURES;
use golden_core::search::skyline_need_m;
use serde_json::{json, Value};

use super::claude::Claude;
use super::library::{valid_name, Library};
use super::trial::{trial, Trial};
use super::{facing, search_with_replan, Log};
use crate::api::{self, PhotoReq};
use crate::state::{ApiErr, AppState};
use crate::{objects, sunpath, tz};

pub const MAX_TURNS: usize = 10;
pub const MAX_TOOL_CALLS: usize = 16;
pub const MAX_DEFINES: usize = 4;
pub const MAX_GOALS: usize = 6;
pub const MAX_INVESTIGATE: usize = 3;
pub const TIME_BUDGET: Duration = Duration::from_secs(120);
/// Photos sent to Claude in `investigate`: small, so three fit easily.
const PHOTO_PX: u32 = 768;

/// The planner runs only with `GOLDEN_PLANNER=1` (or `true`/`on`).
pub fn planner_on() -> bool {
    std::env::var("GOLDEN_PLANNER").is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"))
}

/// The search step: the tool loop when Claude is available, else (or if the loop fails) the fixed search.
pub(super) async fn search_step(st: &AppState, brief: Brief, place: &str, llm: Option<&Claude>, log: &mut Log) -> Result<(SearchResult, Vec<String>), ApiErr> {
    let Some(g) = llm else { return search_with_replan(st, brief, place, log).await };
    match plan(st, &brief, place, g, vexcel::auth::ApiKey::from_env().ok(), log).await {
        Ok((r, notes)) => Ok((r, notes)),
        Err(why) => {
            log.add("plan", format!("The cost-function planner stopped ({why}); searching the brief as read"));
            search_with_replan(st, brief, place, log).await
        }
    }
}

/// The tool loop. `Err` = no usable search came out of it (the caller falls back).
pub(super) async fn plan(
    st: &AppState,
    brief: &Brief,
    place: &str,
    g: &Claude,
    vexcel_key: Option<vexcel::auth::ApiKey>,
    log: &mut Log,
) -> Result<(SearchResult, Vec<String>), String> {
    let t0 = Instant::now();
    let mut s = Session::new(st, brief.clone(), vexcel_key);
    let system = system_prompt();
    let tools = tool_defs();
    let mut messages = vec![json!({"role": "user", "content": first_message(brief, place)})];
    let mut calls = 0;
    let mut turns = 0;
    while turns < MAX_TURNS && !s.finished {
        if t0.elapsed() > TIME_BUDGET {
            log.add("plan", format!("Planner out of time after {turns} turns"));
            break;
        }
        turns += 1;
        let resp = g.tool_turn(&system, &messages, &tools).await?;
        match resp["stop_reason"].as_str() {
            Some("refusal") => return Err("Claude declined".into()),
            Some("max_tokens") => return Err("Claude's answer was cut off".into()),
            _ => {}
        }
        let content = resp["content"].clone();
        let uses: Vec<Value> = content.as_array().map(|b| b.iter().filter(|x| x["type"] == "tool_use").cloned().collect()).unwrap_or_default();
        messages.push(json!({"role": "assistant", "content": content}));
        if uses.is_empty() {
            break;
        }
        let mut results = Vec::new();
        for u in &uses {
            calls += 1;
            let (name, input, id) = (u["name"].as_str().unwrap_or(""), &u["input"], u["id"].clone());
            let t = Instant::now();
            let out = if calls > MAX_TOOL_CALLS {
                Err(format!("tool call limit ({MAX_TOOL_CALLS}) reached: call finish now"))
            } else {
                s.call(name, input).await
            };
            let ms = t.elapsed().as_millis();
            let line = match &out {
                Ok(o) => o.line.clone(),
                Err(e) => format!("error: {}", first_line(e)),
            };
            eprintln!("agent tool: {name} ({ms} ms): {line}");
            log.add(name, line);
            results.push(match out {
                Ok(o) => json!({"type": "tool_result", "tool_use_id": id, "content": o.content}),
                Err(e) => json!({"type": "tool_result", "tool_use_id": id, "content": e, "is_error": true}),
            });
        }
        messages.push(json!({"role": "user", "content": results}));
    }
    eprintln!("agent: planner {turns} turns, {calls} tool calls, {} ms", t0.elapsed().as_millis());
    let r = s.last.take().ok_or("Claude never ran a search")?;
    if r.spots.is_empty() {
        return Err("the planner's search found no spots".into());
    }
    Ok((r, s.notes))
}

/// What a tool hands back: the content for Claude, and one line for the log.
pub struct ToolOut {
    pub content: Value,
    pub line: String,
}

impl ToolOut {
    fn json(v: Value, line: impl Into<String>) -> ToolOut {
        ToolOut { content: Value::String(v.to_string()), line: line.into() }
    }
}

/// One brief's tool state.
pub struct Session<'a> {
    st: &'a AppState,
    brief: Brief,
    library: Library,
    vexcel_key: Option<vexcel::auth::ApiKey>,
    /// Functions defined this brief (latest per name), with their trial.
    defined: Vec<(CostFn, Trial)>,
    defines: usize,
    objects: Vec<Object>,
    last: Option<SearchResult>,
    notes: Vec<String>,
    finished: bool,
}

impl<'a> Session<'a> {
    pub fn new(st: &'a AppState, brief: Brief, vexcel_key: Option<vexcel::auth::ApiKey>) -> Session<'a> {
        Session {
            library: Library::at(&st.cache_root),
            st,
            brief,
            vexcel_key,
            defined: Vec::new(),
            defines: 0,
            objects: Vec::new(),
            last: None,
            notes: Vec::new(),
            finished: false,
        }
    }

    pub async fn call(&mut self, name: &str, input: &Value) -> Result<ToolOut, String> {
        match name {
            "list_cost_functions" => Ok(self.list()),
            "define_cost_function" => self.define(str_field(input, "cost_function")?, input["args"].as_str()).await,
            "save_cost_function" => self.save(str_field(input, "name")?),
            "find_objects" => self.find_objects(&str_list(input, "kinds")?).await,
            "search" => self.search(str_field(input, "goals")?).await,
            "investigate" => self.investigate(&str_list(input, "spot_ids")?).await,
            "finish" => self.finish(&str_list(input, "spot_ids")?, str_field(input, "why")?),
            other => Err(format!("no tool {other:?}")),
        }
    }

    fn list(&self) -> ToolOut {
        let mut fns: Vec<Value> = self
            .library
            .entries()
            .iter()
            .map(|e| json!({"name": e.cost.name, "about": e.cost.about, "params": e.cost.params, "tier": e.tier.name(), "origin": e.origin.name(), "has_must": e.cost.must.is_some()}))
            .collect();
        for (c, _) in &self.defined {
            fns.push(json!({"name": c.name, "about": c.about, "params": c.params, "origin": "defined this brief, not saved"}));
        }
        let n = fns.len();
        ToolOut::json(json!({"cost_functions": fns}), format!("{n} cost functions in the library"))
    }

    async fn define(&mut self, text: &str, args: Option<&str>) -> Result<ToolOut, String> {
        if self.defines >= MAX_DEFINES {
            return Err(format!("define limit ({MAX_DEFINES}) reached: use a function that passed, or one from the library"));
        }
        self.defines += 1;
        let cost: CostFn = serde_json::from_str(text).map_err(|e| format!("not a cost function: {e}"))?;
        valid_name(&cost.name)?;
        if let Some(e) = self.library.get(&cost.name) {
            return Err(format!("\"{}\" is already a {} library function: use it, or pick a new name", cost.name, e.origin.name()));
        }
        let checked = check(&cost).map_err(|errs| errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"))?;
        let args: BTreeMap<String, ObjRef> = match args.map(str::trim).filter(|a| !a.is_empty()) {
            Some(a) => serde_json::from_str(a).map_err(|e| format!("args must be a JSON object of object references, like {{\"a\": {{\"kind\": \"tree\"}}}}: {e}"))?,
            None => BTreeMap::new(),
        };
        if !cost.params.is_empty() && args.is_empty() {
            return Err(format!("\"{}\" has params {:?}: pass args to test it with, like {{\"{}\": {{\"kind\": \"building\"}}}}", cost.name, cost.params, cost.params[0]));
        }
        let grid = self.grid().await?;
        let lib = self.library_now();
        let (b, c, objects) = (self.brief.clone(), cost.clone(), self.objects.clone());
        let t = api::blocking(move || trial(&grid, &b, &c, &args, &lib, &objects)).await.map_err(|e| e.1)??;
        let mut summary = t.summary();
        summary["name"] = json!(cost.name);
        summary["tier"] = json!(checked.tier().name());
        let line = format!(
            "defined {}: {} of {} points dropped, scores {}..{}; {}",
            cost.name,
            t.dropped(),
            t.points.len(),
            summary["score"]["min"],
            summary["score"]["max"],
            t.degenerate().unwrap_or_else(|| "separates points".into())
        );
        self.defined.retain(|(c, _)| c.name != cost.name);
        self.defined.push((cost, t));
        Ok(ToolOut::json(summary, line))
    }

    fn save(&mut self, name: &str) -> Result<ToolOut, String> {
        let Some((cost, t)) = self.defined.iter().find(|(c, _)| c.name == name) else {
            return Err(format!("nothing called \"{name}\" was defined this brief: define_cost_function first"));
        };
        if let Some(why) = t.degenerate() {
            return Err(format!("not saved: {why}. Change it and define it again"));
        }
        let path = self.library.save(cost)?;
        self.defined.retain(|(c, _)| c.name != name);
        eprintln!("library: saved {}", path.display());
        Ok(ToolOut::json(json!({"saved": name}), format!("saved {name} to the library")))
    }

    async fn find_objects(&mut self, kinds: &[String]) -> Result<ToolOut, String> {
        let found = objects::find_objects(self.st, &self.brief.area, kinds, self.vexcel_key.clone()).await.map_err(|e| e.1)?;
        let (lat, lon) = (self.brief.area.lat, self.brief.area.lon);
        let mut by_kind = serde_json::Map::new();
        let kinds_found: Vec<String> = objects::check_kinds(kinds)?;
        for k in &kinds_found {
            let of: Vec<&Object> = found.objects.iter().filter(|o| &o.kind == k).collect();
            let nearest: Vec<Value> = of
                .iter()
                .take(8)
                .map(|o| {
                    json!({"id": o.id, "dist_m": costar::distance_m(lat, lon, o.lat, o.lon).round(), "bearing_deg": bearing(lat, lon, o.lat, o.lon).round(),
                           "height_m": ((o.top_m - o.base_m) * 10.0).round() / 10.0})
                })
                .collect();
            by_kind.insert(k.clone(), json!({"count": of.len(), "nearest": nearest}));
        }
        let line = kinds_found.iter().map(|k| format!("{} {k}", found.objects.iter().filter(|o| &o.kind == k).count())).collect::<Vec<_>>().join(", ");
        self.objects = found.objects;
        Ok(ToolOut::json(
            json!({"objects": by_kind, "notes": found.notes, "from_the_area_centre": true}),
            format!("found {line} within {:.0} m", self.brief.area.radius_m),
        ))
    }

    async fn search(&mut self, goals_json: &str) -> Result<ToolOut, String> {
        let goals: Vec<Goal> = serde_json::from_str(goals_json).map_err(|e| format!("goals must be a JSON list of goals: {e}"))?;
        if goals.is_empty() || goals.len() > MAX_GOALS {
            return Err(format!("give 1 to {MAX_GOALS} goals"));
        }
        let lib = self.library_now();
        let names: Vec<String> = goals.iter().map(|g| g.name().to_string()).collect();
        // the objects find_objects listed, else whatever the goals name
        let objects = if self.objects.is_empty() { objects::for_goals(self.st, &self.brief.area, &goals, &lib).await } else { self.objects.clone() };
        let brief = Brief { goals, ..self.brief.clone() };
        let r = api::run_search_with(self.st, brief, lib, objects).await.map_err(|e| e.1)?;
        let spots: Vec<Value> = r.spots.iter().take(5).map(spot_summary).collect();
        let line = format!(
            "searched {} points for {}: {} spot(s){}",
            r.searched,
            names.join(" + "),
            r.spots.len(),
            r.spots.first().map(|s| format!(", best {:.2}", s.score)).unwrap_or_default()
        );
        let out = json!({"searched": r.searched, "spots": spots, "more": r.spots.len().saturating_sub(5)});
        self.last = Some(r);
        Ok(ToolOut::json(out, line))
    }

    async fn investigate(&mut self, ids: &[String]) -> Result<ToolOut, String> {
        let last = self.last.as_ref().ok_or("search first")?;
        if ids.is_empty() || ids.len() > MAX_INVESTIGATE {
            return Err(format!("give 1 to {MAX_INVESTIGATE} spot ids from the last search"));
        }
        let spots: Vec<Spot> = ids.iter().map(|id| last.spots.iter().find(|s| &s.id == id).cloned().ok_or(format!("no spot {id:?} in the last search"))).collect::<Result<_, _>>()?;
        let grid = self.grid().await?;
        let vexcel = grid.meta.source.provider == "vexcel";
        let date = api::parse_date(&self.brief.date).map_err(|e| e.1)?;
        let zone = tz::parse(&self.brief.tz)?;
        let mut blocks = vec![json!({"type": "text", "text": "Each spot's numbers, then (where Vexcel has one) an oblique aerial photo looking the way you'd face. Judge what the numbers can't see: is the view open or blocked by something near, is it a real roof or terrace someone could stand on, is the scene attractive?"})];
        let mut photos = 0;
        for s in &spots {
            blocks.push(json!({"type": "text", "text": spot_summary(s).to_string()}));
            if !vexcel {
                continue;
            }
            let path = sunpath::sun_path(s.lat, s.lon, date, zone, 10);
            let req = PhotoReq { lat: s.lat, lon: s.lon, az: facing(s, &path, self.brief.event), radius_m: 150, max_px: PHOTO_PX };
            match vexcel::get_oblique_with(req.lat, req.lon, req.az, &req.options()).await {
                Ok(o) => {
                    photos += 1;
                    let data = base64::engine::general_purpose::STANDARD.encode(&o.jpeg);
                    blocks.push(json!({"type": "text", "text": format!("Photo for {}: camera looking {:.0} deg", s.id, o.look_az_deg)}));
                    blocks.push(json!({"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": data}}));
                }
                Err(e) => blocks.push(json!({"type": "text", "text": format!("No photo for {}: {e}", s.id)})),
            }
        }
        if !vexcel {
            blocks.push(json!({"type": "text", "text": "No aerial photos here (heights come from USGS LiDAR): judge from the numbers."}));
        }
        Ok(ToolOut { content: Value::Array(blocks), line: format!("looked at {} spot(s), {photos} photo(s)", spots.len()) })
    }

    fn finish(&mut self, ids: &[String], why: &str) -> Result<ToolOut, String> {
        let last = self.last.as_mut().ok_or("search first")?;
        if ids.is_empty() {
            return Err("give the spot ids in the order you recommend".into());
        }
        let mut ordered: Vec<Spot> = Vec::new();
        for id in ids {
            let s = last.spots.iter().find(|s| &s.id == id).ok_or(format!("no spot {id:?} in the last search"))?;
            if !ordered.iter().any(|o| &o.id == id) {
                ordered.push(s.clone());
            }
        }
        ordered.extend(last.spots.iter().filter(|s| !ids.contains(&s.id)).cloned());
        for (i, s) in ordered.iter_mut().enumerate() {
            s.rank = i as u32 + 1;
        }
        last.spots = ordered;
        let why: String = why.chars().take(300).collect();
        self.notes.push(format!("Agent's pick: {why}"));
        self.finished = true;
        Ok(ToolOut::json(json!({"ok": true}), format!("recommends {}: {why}", ids.join(", "))))
    }

    /// The library plus what was defined this brief (a defined function shadows nothing: names are unique).
    fn library_now(&self) -> Vec<CostFn> {
        let mut lib = self.library.all();
        lib.extend(self.defined.iter().map(|(c, _)| c.clone()));
        lib
    }

    async fn grid(&self) -> Result<std::sync::Arc<contracts::DsmGrid>, String> {
        let a = &self.brief.area;
        self.st.grids.around(a.lat, a.lon, skyline_need_m(a.radius_m)).await.map_err(|e| e.1)
    }
}

fn str_field<'v>(input: &'v Value, key: &str) -> Result<&'v str, String> {
    input[key].as_str().ok_or(format!("missing string \"{key}\""))
}

fn str_list(input: &Value, key: &str) -> Result<Vec<String>, String> {
    let a = input[key].as_array().ok_or(format!("missing list \"{key}\""))?;
    a.iter().map(|v| v.as_str().map(str::to_string).ok_or(format!("\"{key}\" must hold strings"))).collect()
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

/// Pure: compass bearing from one point to another, degrees.
fn bearing(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let east = (lon2 - lon1).to_radians() * lat1.to_radians().cos();
    let north = (lat2 - lat1).to_radians();
    east.atan2(north).to_degrees().rem_euclid(360.0)
}

/// Pure: what the agent sees of a spot.
fn spot_summary(s: &Spot) -> Value {
    json!({
        "id": s.id, "rank": s.rank, "score": s.score, "standing": s.standing,
        "lat": s.lat, "lon": s.lon, "eye_m": s.eye_m,
        "sun_hidden_at": s.sun.hidden_at, "sun_hidden_elev_deg": s.sun.hidden_elev_deg, "golden_min_visible": s.sun.golden_min_visible,
        "scores": s.scores, "why": s.why.iter().take(3).collect::<Vec<_>>()
    })
}

/// Pure: the brief as Claude first sees it.
fn first_message(b: &Brief, place: &str) -> String {
    let brief = json!({"text": b.text, "place": place, "area": b.area, "date": b.date, "event": b.event, "standing": b.standing, "framing": b.framing});
    format!(
        "The photographer's brief, as read: {brief}\n\
         Plan the search: reuse or write cost functions that capture what they asked for, test them, search, \
         look at the best spots, and finish with the order you recommend."
    )
}

/// Pure: the system prompt. The same for every brief (it's cached), so nothing brief-specific goes here.
pub fn system_prompt() -> String {
    let features: Vec<String> = FEATURES.iter().map(|f| format!("- {} ({}): {}", f.name, f.tier.name(), f.about)).collect();
    format!(
        "You plan photo-spot searches for golden, a phone app that finds where to stand for a sunset or sunrise shot in a real city.\n\
         Search scores every standpoint in the area with cost functions: small JSON expressions over features our simulation \
         computes from a 1 m height map of the real buildings (the sun's path against each spot's real skyline, and the standpoint). \
         You choose or write the cost functions; every number comes from the tools, never from you.\n\n\
         How to work:\n\
         1. list_cost_functions. If one already captures the brief, use it. Reusing is better than writing.\n\
         2. Otherwise define_cost_function. It's checked, then test-run on ~50 points of this area: read the error, or the verdict \
            and the best/median/worst examples, and fix and define again if it doesn't separate good points from bad. At most {MAX_DEFINES} defines.\n\
         3. save_cost_function when it captures a reusable class of shot. Name it for the class (stays_sunny_rooftop), not for this place.\n\
         4. search with 1-{MAX_GOALS} goals. 5. investigate the top 1-{MAX_INVESTIGATE} spots. 6. finish with the order you recommend and one sentence why.\n\
         find_objects lists what's in the area (CoStar buildings, Vexcel detections: {kinds}). Object features (dist, bearing, visible) \
         are being added to the expression language; until the checker accepts them, use objects only to inform the plan.\n\n\
         Features (cost tier: point is cheap, skyline is the expensive sun trace):\n{features}\n\n\
         Expressions (JSON, every one evaluates to a number; scores are clamped to 0..1):\n\
         {{\"f\": \"<feature>\"}}  {{\"num\": 0.5}}\n\
         {{\"op\": \"ramp\", \"x\": E, \"lo\": 20, \"hi\": 5}}  0 at lo, 1 at hi, linear, clamped (lo > hi allowed)\n\
         {{\"op\": \"bump\", \"x\": E, \"at\": 0, \"width\": 10}}  1 at `at`, 0 beyond +-width\n\
         {{\"op\": \"and\"|\"or\"|\"min\"|\"max\"|\"mul\"|\"add\"|\"avg\", \"of\": [E, ...]}}  {{\"op\": \"not\", \"x\": E}}  {{\"op\": \"lt\"|\"gt\", \"x\": E, \"than\": E}} (0 or 1)\n\
         A cost function: {{\"name\": \"snake_case\", \"about\": \"one line\", \"must\": E (optional gate: 0 drops the point), \"score\": E}}\n\
         Goals for search, a JSON list: {{\"use\": \"<name>\"}} (a library function or one you defined), optional \"weight\" (default 1), \
         \"must\": true (a gate, weight 0 unless given); or {{\"def\": {{cost function}}}} inline.\n\n\
         Keep your own text short. Never write clock times or numbers the tools didn't give you.",
        kinds = OBJECT_KINDS.join(", "),
        features = features.join("\n"),
    )
}

/// Pure: the tools, as the Messages API takes them. Strict: Claude's input always matches. Expressions travel as
/// JSON text, so a malformed one comes back to Claude as a readable error instead of a schema failure.
pub fn tool_defs() -> Value {
    let obj = |props: Value, req: &[&str]| json!({"type": "object", "properties": props, "required": req, "additionalProperties": false});
    let tool = |name: &str, about: &str, schema: Value| json!({"name": name, "description": about, "input_schema": schema, "strict": true});
    json!([
        tool("list_cost_functions", "The cost-function library: name, what it looks for, params, cost tier, seed or saved.", obj(json!({}), &[])),
        tool(
            "define_cost_function",
            "Check a cost function and test-run it on every candidate in the area. Returns errors (fix and retry), or the score distribution, the share its must drops, best/median/worst points with the numbers behind their scores, and a verdict.",
            obj(
                json!({
                    "cost_function": {"type": "string", "description": "The cost function as JSON text: {\"name\", \"about\", \"params\"?, \"must\"?, \"score\"}"},
                    "args": {"type": "string", "description": "Only if it has params: JSON text giving each param an object, e.g. {\"a\": {\"kind\": \"tree\"}, \"b\": {\"kind\": \"building\"}}. Empty string otherwise."}
                }),
                &["cost_function", "args"],
            ),
        ),
        tool(
            "save_cost_function",
            "Save a function you defined this brief to the library, for later briefs. Refused if its test run didn't separate points.",
            obj(json!({"name": {"type": "string"}}), &["name"]),
        ),
        tool(
            "find_objects",
            "Objects of these kinds in the search area, nearest the centre first (count, ids, distance, bearing, height).",
            obj(json!({"kinds": {"type": "array", "items": {"type": "string", "enum": OBJECT_KINDS}}}), &["kinds"]),
        ),
        tool(
            "search",
            "Rank every standpoint in the area for these goals. Returns the top spots with each goal's score and reason.",
            obj(json!({"goals": {"type": "string", "description": "A JSON list of goals, e.g. [{\"use\": \"sun_clear\"}, {\"use\": \"stays_sunny\", \"must\": true}]"}}), &["goals"]),
        ),
        tool(
            "investigate",
            "For 1-3 spots from the last search: their numbers and an oblique aerial photo looking the way you'd face, to judge what the numbers can't see.",
            obj(json!({"spot_ids": {"type": "array", "items": {"type": "string"}}}), &["spot_ids"]),
        ),
        tool(
            "finish",
            "End the plan: spot ids from the last search in the order you recommend, and one sentence why.",
            obj(json!({"spot_ids": {"type": "array", "items": {"type": "string"}}, "why": {"type": "string"}}), &["spot_ids", "why"]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::library::Origin;
    use crate::testkit;
    use axum::routing::post;
    use axum::Router;
    use contracts::{Area, Event, Framing, Standing};
    use std::sync::{Arc, Mutex};

    type Script = Arc<dyn Fn(usize, &Value) -> Value + Send + Sync>;

    /// A local Messages API whose n-th answer is `script(n, request)`. Records every request body.
    async fn fake_claude(script: Script) -> (Claude, Arc<Mutex<Vec<Value>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let app = Router::new().route(
            "/v1/messages",
            post(move |axum::Json(body): axum::Json<Value>| {
                let (log, script) = (log.clone(), script.clone());
                async move {
                    let n = {
                        let mut l = log.lock().unwrap();
                        l.push(body.clone());
                        l.len() - 1
                    };
                    axum::Json(script(n, &body))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (Claude::new("test-key".into(), "claude-sonnet-5-5".into(), format!("http://{addr}"), &reqwest::Client::new()), seen)
    }

    fn uses(calls: &[(&str, Value)]) -> Value {
        let content: Vec<Value> = std::iter::once(json!({"type": "thinking", "thinking": "", "signature": "sig"}))
            .chain(calls.iter().enumerate().map(|(i, (n, input))| json!({"type": "tool_use", "id": format!("toolu_{n}_{i}"), "name": n, "input": input})))
            .collect();
        json!({"type": "message", "role": "assistant", "stop_reason": "tool_use", "content": content, "usage": {"input_tokens": 1, "output_tokens": 1}})
    }

    fn done(text: &str) -> Value {
        json!({"type": "message", "role": "assistant", "stop_reason": "end_turn", "content": [{"type": "text", "text": text}]})
    }

    /// The last tool results Claude was sent (the last user message of a request).
    fn results(req: &Value) -> Vec<Value> {
        req["messages"].as_array().unwrap().last().unwrap()["content"].as_array().cloned().unwrap_or_default()
    }

    fn brief() -> Brief {
        Brief {
            text: "rooftop where the sun stays out all golden hour".into(),
            area: Area { lat: testkit::LAT, lon: testkit::LON, radius_m: 300.0, name: None },
            date: "2026-06-21".into(),
            tz: "America/Los_Angeles".into(),
            event: Event::Sunset,
            standing: Standing::Any,
            framing: Framing::Clear,
            max_spots: 10,
            goals: vec![],
            lens_deg: contracts::DEFAULT_LENS_DEG,
        }
    }

    const BAD: &str = r#"{"name":"stays_sunny","about":"Keeps direct sun for most of golden hour","score":{"op":"ramp","x":{"f":"shade"},"lo":1,"hi":0}}"#;
    const GOOD: &str = r#"{"name":"stays_sunny","about":"Keeps direct sun for most of golden hour",
        "must":{"op":"gt","x":{"f":"sun_visible_min"},"than":{"num":5}},
        "score":{"op":"ramp","x":{"f":"sun_visible_min"},"lo":5,"hi":30}}"#;

    /// The script from the task: list + objects, a wrong define, a right one, save, search, investigate, finish.
    fn the_script() -> Script {
        Arc::new(|n, req| match n {
            0 => uses(&[("list_cost_functions", json!({})), ("find_objects", json!({"kinds": ["building", "tree"]}))]),
            1 => uses(&[("define_cost_function", json!({"cost_function": BAD}))]),
            2 => uses(&[("define_cost_function", json!({"cost_function": GOOD}))]),
            3 => uses(&[("save_cost_function", json!({"name": "stays_sunny"}))]),
            4 => uses(&[("search", json!({"goals": r#"[{"use":"stays_sunny"},{"use":"sun_clear","weight":0.5}]"#}))]),
            5 => {
                // the spot ids come from the search result Claude was sent
                let v: Value = serde_json::from_str(results(req)[0]["content"].as_str().unwrap()).unwrap();
                let ids: Vec<Value> = v["spots"].as_array().unwrap().iter().take(2).map(|s| s["id"].clone()).collect();
                uses(&[("investigate", json!({"spot_ids": ids}))])
            }
            _ => done("unexpected"),
        })
    }

    #[tokio::test]
    async fn the_agent_defines_fixes_saves_searches_and_reuses_a_function() {
        let st = testkit::state(testkit::city());
        let spot_ids: Arc<Mutex<Vec<Value>>> = Arc::default();
        let ids = spot_ids.clone();
        let base = the_script();
        let script: Script = Arc::new(move |n, req| {
            if n == 5 {
                let v = base(n, req);
                *ids.lock().unwrap() = v["content"][1]["input"]["spot_ids"].as_array().unwrap().clone();
                v
            } else if n == 6 {
                // finish with the second spot first: the agent re-ranks after looking
                let mut order = ids.lock().unwrap().clone();
                order.reverse();
                uses(&[("finish", json!({"spot_ids": order, "why": "the second roof's view is more open"}))])
            } else {
                base(n, req)
            }
        });
        let (g, seen) = fake_claude(script).await;
        let mut log = Log::default();
        let (r, notes) = plan(&st, &brief(), "the test city", &g, None, &mut log).await.unwrap();

        // the tool calls, in order, are in the sheet's steps
        let tools: Vec<&str> = log.0.iter().map(|s| s.tool.as_str()).collect();
        assert_eq!(tools, ["list_cost_functions", "find_objects", "define_cost_function", "define_cost_function", "save_cost_function", "search", "investigate", "finish"]);
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 7, "no call after finish");

        // the wrong define went back as an error Claude can read, verbatim from core's checker
        let bad = &results(&seen[2])[0];
        assert_eq!(bad["is_error"], true);
        assert!(bad["content"].as_str().unwrap().contains("score.x: unknown feature \"shade\"; features are ground_m"), "{bad}");
        // the right one came back with its test run
        let good: Value = serde_json::from_str(results(&seen[3])[0]["content"].as_str().unwrap()).unwrap();
        assert!(good["standable"].as_u64().unwrap() > 20 && good["verdict"].as_str().unwrap().starts_with("separates"), "{good}");
        // on the test city the wall either hides the sun early (dropped) or not at all: the gate does the separating
        assert!(good["best"]["why"].as_str().is_some_and(|w| !w.is_empty()) && good["a_dropped_point"]["score"] == 0.0, "{good}");
        assert!(good["dropped_by_must"].as_u64().unwrap() > 0, "{good}");

        // saved, and the search used it: every spot is scored on both goals, ranked by the agent's pick
        assert!(Library::at(&st.cache_root).get("stays_sunny").is_some_and(|e| e.origin == Origin::Saved));
        assert!(r.spots.iter().all(|s| s.scores.iter().map(|x| x.goal.as_str()).collect::<Vec<_>>() == ["stays_sunny", "sun_clear"]));
        let picked = spot_ids.lock().unwrap().clone();
        assert_eq!(json!(r.spots[0].id), picked[1]);
        assert_eq!(r.spots.iter().map(|s| s.rank).collect::<Vec<_>>(), (1..=r.spots.len() as u32).collect::<Vec<_>>());
        assert!(notes[0].contains("more open"));

        // the request shape: strict tools, auto choice, thinking blocks passed back untouched
        assert_eq!(seen[0]["tool_choice"]["type"], "auto");
        assert!(seen[0]["tools"].as_array().unwrap().iter().all(|t| t["strict"] == true));
        assert_eq!(seen[1]["messages"][1]["content"][0]["signature"], "sig");

        // reused later: a plain search that names the saved function, with no agent at all
        let b = Brief { goals: serde_json::from_str(r#"[{"use":"stays_sunny"}]"#).unwrap(), ..brief() };
        let again = api::run_search(&st, b).await.unwrap();
        assert!(!again.spots.is_empty() && again.spots[0].scores[0].goal == "stays_sunny");
        let _ = std::fs::remove_dir_all(&st.cache_root);
    }

    #[tokio::test]
    async fn a_degenerate_function_is_not_saved() {
        let st = testkit::state(testkit::city());
        let flat = r#"{"name":"always_half","about":"Scores everything the same","score":{"num":0.5}}"#;
        let script: Script = Arc::new(move |n, _| match n {
            0 => uses(&[("define_cost_function", json!({"cost_function": flat}))]),
            1 => uses(&[("save_cost_function", json!({"name": "always_half"}))]),
            2 => uses(&[("search", json!({"goals": r#"[{"use":"sun_clear"}]"#}))]),
            _ => done("Searched with sun_clear."),
        });
        let (g, seen) = fake_claude(script).await;
        let mut log = Log::default();
        let (r, _) = plan(&st, &brief(), "the test city", &g, None, &mut log).await.unwrap();
        let refused = &results(&seen.lock().unwrap()[2])[0];
        assert_eq!(refused["is_error"], true);
        assert!(refused["content"].as_str().unwrap().contains("doesn't separate"), "{refused}");
        assert!(Library::at(&st.cache_root).get("always_half").is_none());
        assert!(!r.spots.is_empty(), "ending without finish still uses the last search");
        let _ = std::fs::remove_dir_all(&st.cache_root);
    }

    #[tokio::test]
    async fn a_planner_that_never_searches_falls_back_to_the_fixed_search() {
        let st = testkit::state(testkit::city());
        let (g, _) = fake_claude(Arc::new(|_, _| done("I don't know."))).await;
        let mut log = Log::default();
        let (r, _) = search_step(&st, brief(), "the test city", Some(&g), &mut log).await.unwrap();
        assert!(!r.spots.is_empty());
        let steps: Vec<String> = log.0.iter().map(|s| format!("{}: {}", s.tool, s.summary)).collect();
        assert!(steps[0].starts_with("plan: The cost-function planner stopped (Claude never ran a search)"), "{steps:?}");
        assert!(steps.iter().any(|s| s.starts_with("search: Scored")), "{steps:?}");
    }

    #[tokio::test]
    async fn refusals_and_runaway_loops_stop_safely() {
        let st = testkit::state(testkit::city());
        let refusal = json!({"type": "message", "stop_reason": "refusal", "content": []});
        let (g, _) = fake_claude(Arc::new(move |_, _| refusal.clone())).await;
        let e = plan(&st, &brief(), "x", &g, None, &mut Log::default()).await.unwrap_err();
        assert!(e.contains("declined"));

        // a model that lists forever: capped at MAX_TURNS calls
        let (g, seen) = fake_claude(Arc::new(|_, _| uses(&[("list_cost_functions", json!({}))]))).await;
        let e = plan(&st, &brief(), "x", &g, None, &mut Log::default()).await.unwrap_err();
        assert!(e.contains("never ran a search"));
        assert_eq!(seen.lock().unwrap().len(), MAX_TURNS);
    }

    #[tokio::test]
    async fn bad_tool_input_is_an_error_result_not_a_crash() {
        let st = testkit::state(testkit::city());
        let mut s = Session::new(&st, brief(), None);
        for (tool, input, says) in [
            ("search", json!({"goals": "not json"}), "goals must be a JSON list"),
            ("search", json!({"goals": "[]"}), "give 1 to"),
            ("save_cost_function", json!({"name": "never_defined"}), "define_cost_function first"),
            ("investigate", json!({"spot_ids": ["s-1"]}), "search first"),
            ("define_cost_function", json!({"cost_function": "{\"name\":\"x\"}"}), "not a cost function"),
            ("define_cost_function", json!({"cost_function": r#"{"name":"sun_clear","about":"x","score":{"num":1}}"#}), "already a seed"),
            ("find_objects", json!({"kinds": ["fountain"]}), "unknown kind"),
            ("nope", json!({}), "no tool"),
        ] {
            let e = s.call(tool, &input).await.err().unwrap_or_else(|| panic!("{tool} {input} should fail"));
            assert!(e.contains(says), "{tool}: {e}");
        }
    }

    #[test]
    fn the_prompt_and_tools_name_every_feature_and_kind() {
        let p = system_prompt();
        assert!(FEATURES.iter().all(|f| p.contains(f.name)));
        let t = tool_defs();
        let names: Vec<&str> = t.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["list_cost_functions", "define_cost_function", "save_cost_function", "find_objects", "search", "investigate", "finish"]);
        assert_eq!(t[3]["input_schema"]["properties"]["kinds"]["items"]["enum"].as_array().unwrap().len(), OBJECT_KINDS.len());
        assert!((bearing(0.0, 0.0, 0.0, 1.0) - 90.0).abs() < 1e-9 && (bearing(0.0, 0.0, -1.0, 0.0) - 180.0).abs() < 1e-9);
    }
}
