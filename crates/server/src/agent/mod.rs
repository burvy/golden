//! `POST /api/agent`: typed brief in, `CallSheet` out (docs/contracts.md#agentrequest-and-callsheet).
//!
//! The loop, each step logged into `CallSheet.steps` so the app shows the agent's work:
//! 1. **read**: text -> `Brief` (place, date, sunrise/sunset, roof/ground, framing). Claude if
//!    `ANTHROPIC_API_KEY` is set, else the offline reader (`read.rs`). Places resolve to coordinates through
//!    our presets or Google Places, never from model text.
//! 2. **search** (`/api/search` logic, core's ranking over the height map). If nothing fits, **plan**:
//!    relax the framing/standing, then widen the area, and search again.
//! 3. **sun** (sun path at the primary spot), **photo** (Vexcel oblique toward the sun),
//!    **buildings** (CoStar contact for roof spots).
//! 4. **write**: title, summary, notes, access requests. Claude's words are only accepted if every clock
//!    time in them came from the tools (`times::times_ok`); otherwise the templated text is used.

pub mod chat;
pub mod claude;
pub mod read;
pub mod times;

use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use chrono_tz::Tz;
use contracts::{
    Access, AgentRequest, AgentStep, Area, Brief, Building, CallSheet, ContactRole, DsmSource, Event, Framing, SearchResult, SheetSpot, Spot,
    SpotRole, Standing, SunPath, TimelineItem,
};
use golden_core::score::compass;
use serde_json::json;

use crate::api::{self, PhotoReq};
use crate::state::{ApiErr, AppState};
use crate::{secrets, sunpath, tz};
use claude::Claude;

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new().route("/api/agent", post(post_agent)).route("/api/chat", post(chat::post_chat)).with_state(state)
}

async fn post_agent(State(st): State<Arc<AppState>>, body: Result<Json<AgentRequest>, JsonRejection>) -> Result<Response, ApiErr> {
    let Json(req) = body.map_err(|e| ApiErr::bad(e.body_text()))?;
    if req.text.trim().is_empty() {
        return Err(ApiErr::bad("text is empty: describe the shot"));
    }
    Ok(Json(run(&st, req, Utc::now()).await?).into_response())
}

#[derive(Default)]
struct Log(Vec<AgentStep>);

impl Log {
    fn add(&mut self, tool: &str, summary: impl Into<String>) {
        self.0.push(AgentStep { tool: tool.into(), summary: summary.into() });
    }
}

/// Where we're looking, by name, for the sheet's words.
struct Place {
    name: String,
    area: Area,
    tz: String,
}

pub async fn run(st: &AppState, req: AgentRequest, now: DateTime<Utc>) -> Result<CallSheet, ApiErr> {
    run_with(st, req, now, Claude::from_env(&st.http)).await
}

/// `run` with the model passed in: `None` is the offline mock (tests use it so a key in the environment
/// can't make them call the real API).
async fn run_with(st: &AppState, req: AgentRequest, now: DateTime<Utc>, llm: Option<Claude>) -> Result<CallSheet, ApiErr> {
    let mut log = Log::default();
    let t0 = std::time::Instant::now();
    // 1. read
    let (brief, place) = read_brief(st, &req, now, llm.as_ref(), &mut log).await?;
    eprintln!("agent: read @{} ms", t0.elapsed().as_millis());
    sheet_for(st, brief, place, llm.as_ref(), log).await
}

/// Steps 2-4 for a brief that's already settled (by `read_brief`, or by the chat).
async fn sheet_for(st: &AppState, brief: Brief, place: Place, llm: Option<&Claude>, mut log: Log) -> Result<CallSheet, ApiErr> {
    let clock = std::time::Instant::now();
    let mut split: Vec<(&str, u128)> = Vec::new();
    let lap = |name, split: &mut Vec<(&str, u128)>| split.push((name, clock.elapsed().as_millis()));

    // 2. search, re-planning if nothing fits
    let (result, notes) = search_with_replan(st, brief, &place.name, &mut log).await?;
    lap("search", &mut split);
    let brief = result.brief.clone();
    let date = api::parse_date(&brief.date)?;
    let zone = tz::parse(&brief.tz).map_err(ApiErr::bad)?;
    let picked: Vec<Spot> = result.spots.iter().take(3).cloned().collect();
    let primary = picked.first().ok_or_else(|| ApiErr::unprocessable("no spots found, even after widening the search"))?.clone();
    let grid = st.grids.around(brief.area.lat, brief.area.lon, golden_core::search::skyline_need_m(brief.area.radius_m)).await?;
    let source = grid.meta.source.clone();

    // 3. sun, photos, buildings
    let path = sunpath::sun_path(primary.lat, primary.lon, date, zone, 10);
    let golden = golden_of(&path, brief.event);
    log.add("sun", match &golden {
        Some((a, b)) => format!("{} golden hour {}–{} at the primary spot", event_word(brief.event), hhmm(a), hhmm(b)),
        None => "No golden hour that day".to_string(),
    });

    let mut photos = Vec::new();
    for (i, s) in picked.iter().enumerate() {
        let url = if source.provider != "vexcel" {
            None
        } else {
            let req = PhotoReq { lat: s.lat, lon: s.lon, az: facing(s, &path, brief.event), radius_m: 150, max_px: 1024 };
            // fetch the primary's photo now (it's what the sheet opens with); backups load when opened
            if i == 0 {
                match vexcel::get_oblique_with(req.lat, req.lon, req.az, &req.options()).await {
                    Ok(o) => {
                        log.add("photo", format!("Vexcel oblique looking {:.0}° ({}) at the primary spot", o.look_az_deg, compass(o.look_az_deg)));
                        Some(req.url())
                    }
                    Err(e) => {
                        log.add("photo", format!("No Vexcel oblique for the primary spot ({e})"));
                        None
                    }
                }
            } else {
                Some(req.url())
            }
        };
        photos.push(url);
    }
    if source.provider != "vexcel" {
        log.add("photo", "No Vexcel aerial photos here (heights come from USGS LiDAR)");
    }

    // A roof's own building (search's 40 m match, `dist_m` None), else the nearest CoStar building within
    // NEARBY_BUILDING_M (`dist_m` set): CoStar's point is the street address, often 50-100 m from a roof on a big block.
    let buildings: Vec<Option<Building>> = picked
        .iter()
        .map(|s| {
            let kit = st.kit?;
            match &s.building_id {
                Some(id) => kit.building(id),
                None if s.standing == Standing::Roof => kit.buildings_near(s.lat, s.lon, NEARBY_BUILDING_M).into_iter().next(),
                None => None,
            }
        })
        .collect();
    let roofs = picked.iter().filter(|s| s.standing == Standing::Roof).count();
    let costar = match st.kit {
        None => Costar::NoKit,
        Some(k) if k.buildings_near(brief.area.lat, brief.area.lon, brief.area.radius_m + COSTAR_HERE_M).is_empty() => Costar::NotHere,
        Some(_) => Costar::Here,
    };
    if roofs > 0 && costar == Costar::NotHere {
        log.add("buildings", format!("{roofs} roof spot(s); CoStar has no buildings here (its data covers Los Angeles and Houston)"));
    } else if roofs > 0 {
        let found = buildings.iter().flatten().filter(|b| b.dist_m.is_none()).count();
        let near = buildings.iter().flatten().filter(|b| b.dist_m.is_some()).count();
        log.add("buildings", match st.kit {
            Some(_) if near > 0 => format!("{roofs} roof spot(s); {found} on a CoStar building, {near} next to one (nearest within {NEARBY_BUILDING_M:.0} m)"),
            Some(_) => format!("{roofs} roof spot(s); {found} matched to a CoStar building"),
            None => format!("{roofs} roof spot(s); CoStar kit not loaded, so no access contacts"),
        });
    }

    lap("tools", &mut split);

    // 4. assemble, then write the words
    let mut sheet = assemble(&brief, &place, zone, &path, &picked, &photos, &buildings, &source, costar, notes);
    let mut model = "mock".to_string();
    if let Some(g) = llm {
        match write_with_claude(g, &place, &sheet).await {
            Ok(words) => {
                apply(&mut sheet, words);
                model = g.model.clone();
                log.add("write", format!("{} wrote the sheet; every time in it was checked against the tools", g.model));
            }
            Err(why) => log.add("write", format!("Kept the templated wording: {why}")),
        }
    } else {
        log.add("write", "Wrote the sheet from templates (no ANTHROPIC_API_KEY: offline mock)");
        sheet.caveats.push("Written by the offline mock (no Claude key yet): the wording is templated; all numbers come from the tools.".into());
    }
    lap("write", &mut split);
    eprintln!("agent: {}", split.iter().map(|(n, ms)| format!("{n} @{ms} ms")).collect::<Vec<_>>().join(", "));
    sheet.model = model;
    sheet.steps = log.0;
    Ok(sheet)
}

// ---- 1. read ---------------------------------------------------------------------------------

async fn read_brief(st: &AppState, req: &AgentRequest, now: DateTime<Utc>, llm: Option<&Claude>, log: &mut Log) -> Result<(Brief, Place), ApiErr> {
    let text = req.text.trim();
    let mut fields = Fields::read(text);
    let mut by = "the offline reader";
    if let Some(g) = llm {
        match claude_read(g, text, now, req).await {
            Ok(v) => {
                by = "Claude";
                fields = fields.with_model(&v);
            }
            Err(e) => log.add("read", format!("Claude couldn't read the brief ({e}); using the offline reader")),
        }
    }
    let (brief, place) = settle(st, text, &fields, req.area.as_ref(), req.date.as_deref(), &req.tz, now, log).await?;
    log.add("read", format!("Brief read: {} (by {by})", understood(&brief, &place).join(", ")));
    Ok((brief, place))
}

/// What a reading of the brief (the offline reader's, Claude's, or the chat's) says, before the place is looked up.
#[derive(Debug, Clone, PartialEq)]
pub struct Fields {
    /// The place as named ("Princeton", "Griffith Park"); None = not said.
    pub place: Option<String>,
    /// YYYY-MM-DD if the model gave one; checked before use.
    pub date: Option<String>,
    pub event: Event,
    pub standing: Standing,
    pub framing: Framing,
}

impl Fields {
    /// The offline reader's reading of `text`.
    pub fn read(text: &str) -> Fields {
        Fields {
            place: read::preset_in(text).map(|p| p.name.to_string()),
            date: None,
            event: read::event(text),
            standing: read::standing(text),
            framing: read::framing(text),
        }
    }

    /// These fields overridden by a model's JSON reading (`place`, `date`, `event`, `standing`, `framing`;
    /// empty or unknown values keep ours).
    pub fn with_model(self, v: &serde_json::Value) -> Fields {
        Fields {
            place: v["place"].as_str().map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).or(self.place),
            date: v["date"].as_str().filter(|d| !d.is_empty()).map(str::to_string).or(self.date),
            event: parse_enum(&v["event"]).unwrap_or(self.event),
            standing: parse_enum(&v["standing"]).unwrap_or(self.standing),
            framing: parse_enum(&v["framing"]).unwrap_or(self.framing),
        }
    }
}

/// Fields -> a searchable `Brief`: the place resolved to coordinates (a given area wins, then presets, then
/// Google Places, else Downtown LA), and the date checked (a given date wins, then the model's if it isn't in
/// the past, then the text's, else tomorrow).
#[allow(clippy::too_many_arguments)]
async fn settle(
    st: &AppState,
    text: &str,
    f: &Fields,
    area: Option<&Area>,
    date: Option<&str>,
    tz_name: &str,
    now: DateTime<Utc>,
    log: &mut Log,
) -> Result<(Brief, Place), ApiErr> {
    let place = match area {
        Some(a) => Place { name: "the chosen area".into(), area: a.clone(), tz: tz_name.to_string() },
        None => match f.place.as_deref() {
            Some(w) => match read::preset_named(w) {
                Some(p) => Place { name: p.name.into(), area: p.area(), tz: p.tz.into() },
                None => match geocode(st, w).await {
                    Some((lat, lon)) => {
                        log.add("plan", format!("Google Places put \"{w}\" at {lat:.4}, {lon:.4}"));
                        Place { name: w.into(), area: Area { lat, lon, radius_m: 300.0 }, tz: tz_guess(lon).into() }
                    }
                    None => {
                        log.add("plan", format!("Couldn't place \"{w}\" (no Google Places key, or no match); searching Downtown LA"));
                        default_place()
                    }
                },
            },
            None => default_place(),
        },
    };

    let zone = tz::parse(&place.tz).map_err(ApiErr::bad)?;
    let today = now.with_timezone(&zone).date_naive();
    let model_date = f.date.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).filter(|d| *d >= today);
    let date = match date {
        Some(d) => api::parse_date(d)?,
        None => model_date.or_else(|| read::date(text, today)).unwrap_or(today + Duration::days(1)),
    };

    let brief = Brief {
        text: text.to_string(),
        area: place.area.clone(),
        date: date.format("%Y-%m-%d").to_string(),
        tz: place.tz.clone(),
        event: f.event,
        standing: f.standing,
        framing: f.framing,
        max_spots: 10,
        goals: Vec::new(),
        lens_deg: contracts::DEFAULT_LENS_DEG,
    };
    Ok((brief, place))
}

/// Pure: the brief in plain words, for the read step and the chat ("Sunset", "from a roof", ..., "Sat 15 November").
fn understood(b: &Brief, place: &Place) -> Vec<String> {
    let day = NaiveDate::parse_from_str(&b.date, "%Y-%m-%d").map(|d| d.format("%a %-d %B").to_string()).unwrap_or_else(|_| b.date.clone());
    vec![event_word(b.event).into(), framing_words(b.framing).into(), standing_words(b.standing).into(), place.name.clone(), day]
}

fn default_place() -> Place {
    let p = &read::PRESETS[0];
    Place { name: p.name.into(), area: p.area(), tz: p.tz.into() }
}

fn parse_enum<T: serde::de::DeserializeOwned>(v: &serde_json::Value) -> Option<T> {
    serde_json::from_value(v.clone()).ok()
}

async fn claude_read(g: &Claude, text: &str, now: DateTime<Utc>, req: &AgentRequest) -> Result<serde_json::Value, String> {
    let zone = tz::parse(&req.tz).unwrap_or(chrono_tz::America::Los_Angeles);
    let today = now.with_timezone(&zone).date_naive();
    let places: Vec<&str> = read::PRESETS.iter().map(|p| p.name).collect();
    let prompt = format!(
        "You set up a search for a sunset/sunrise photo-spot app. Today is {} ({}).\n\
         The photographer wrote: \"{}\"\n\
         Return: place = the place they name (one of {:?} if it matches, else the name as written, or \"\" if none); \
         date = the shoot day as YYYY-MM-DD if they imply one (\"late June\" = the 24th of the next June), else \"\"; \
         event = sunset or sunrise; standing = roof (rooftop, terrace), ground (street, park) or any; \
         framing = between (sun in a gap between buildings), behind (sun behind a building, silhouette) or clear.",
        today.format("%A %Y-%m-%d"),
        zone.name(),
        text.replace('"', "'"),
        places
    );
    let schema = json!({
        "type": "object",
        "properties": {
            "place": { "type": "string" },
            "date": { "type": "string" },
            "event": { "type": "string", "enum": ["sunset", "sunrise"] },
            "standing": { "type": "string", "enum": ["roof", "ground", "any"] },
            "framing": { "type": "string", "enum": ["clear", "between", "behind"] }
        },
        "required": ["place", "date", "event", "standing", "framing"],
        "additionalProperties": false
    });
    g.json(&prompt, schema).await
}

/// Google Places (New) text search -> (lat, lon), if `GOOGLE_MAPS_API_KEY` is set and it answers.
async fn geocode(st: &AppState, text: &str) -> Option<(f64, f64)> {
    let key = secrets::get("GOOGLE_MAPS_API_KEY")?;
    let resp = st
        .http
        .post("https://places.googleapis.com/v1/places:searchText")
        .header("X-Goog-Api-Key", key)
        .header("X-Goog-FieldMask", "places.displayName,places.location")
        .json(&json!({ "textQuery": text }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        eprintln!("Google Places: HTTP {}", resp.status().as_u16());
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    let loc = &v["places"][0]["location"];
    Some((loc["latitude"].as_f64()?, loc["longitude"].as_f64()?))
}

/// US time zone from longitude: good enough for our US-only height data.
pub fn tz_guess(lon: f64) -> &'static str {
    if lon > -87.5 {
        "America/New_York"
    } else if lon > -101.5 {
        "America/Chicago"
    } else if lon > -114.5 {
        "America/Denver"
    } else {
        "America/Los_Angeles"
    }
}

// ---- 2. search and re-plan --------------------------------------------------------------------

async fn search_with_replan(st: &AppState, mut brief: Brief, place: &str, log: &mut Log) -> Result<(SearchResult, Vec<String>), ApiErr> {
    let mut notes = Vec::new();
    for attempt in 0..3 {
        let r = api::run_search(st, brief.clone()).await?;
        log.add("search", format!(
            "Scored {} candidates within {:.0} m of {place}: {} spot(s){}",
            r.searched,
            brief.area.radius_m,
            r.spots.len(),
            r.spots.first().map(|s| format!(", best: {}", s.why.first().cloned().unwrap_or_default())).unwrap_or_default()
        ));
        if !r.spots.is_empty() {
            return Ok((r, notes));
        }
        if attempt == 0 && (brief.framing != Framing::Clear || brief.standing != Standing::Any) {
            let msg = format!("Nothing fits \"{}\" {}; relaxing to any open view", framing_words(brief.framing), standing_words(brief.standing));
            log.add("plan", msg.clone());
            notes.push(msg);
            brief.framing = Framing::Clear;
            brief.standing = Standing::Any;
        } else {
            let wider = (brief.area.radius_m * 1.6).min(contracts::search::MAX_BRIEF_RADIUS_M);
            let msg = format!("Still nothing; widening the area from {:.0} m to {wider:.0} m", brief.area.radius_m);
            log.add("plan", msg.clone());
            notes.push(msg);
            brief.area.radius_m = wider;
        }
    }
    Err(ApiErr::unprocessable("no spots found, even after relaxing the brief and widening the area"))
}

// ---- 3/4. assemble the sheet --------------------------------------------------------------------

fn event_word(e: Event) -> &'static str {
    match e {
        Event::Sunset => "Sunset",
        Event::Sunrise => "Sunrise",
    }
}

fn standing_words(s: Standing) -> &'static str {
    match s {
        Standing::Roof => "from a roof",
        Standing::Ground => "from street level",
        Standing::Any => "roof or street",
    }
}

fn framing_words(f: Framing) -> &'static str {
    match f {
        Framing::Between => "sun between two buildings",
        Framing::Behind => "sun behind a building",
        Framing::Clear => "clear view of the sun",
    }
}

fn parse_t(iso: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(iso).ok()
}

/// Unmatched roof spots ask the nearest CoStar building within this distance (flagged in the caveats).
const NEARBY_BUILDING_M: f64 = 100.0;
/// CoStar "covers" a search if it has any building within this distance of the area. Its kit is LA and Houston
/// only, so e.g. Princeton (Vexcel, but no CoStar) gets one clear caveat instead of "roof #n isn't matched".
const COSTAR_HERE_M: f64 = 3000.0;

/// Whether CoStar can say who to ask for roof spots in this search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Costar {
    NoKit,
    NotHere,
    Here,
}

fn hhmm(iso: &str) -> String {
    parse_t(iso).map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}

fn golden_of(path: &SunPath, event: Event) -> Option<(String, String)> {
    let r = match event {
        Event::Sunset => path.golden_hour.evening.as_ref(),
        Event::Sunrise => path.golden_hour.morning.as_ref(),
    }?;
    Some((r.from.clone(), r.to.clone()))
}

/// The moment of the shot at a spot: when the sun meets the skyline, else the flat sunset / sunrise.
fn shot_time(s: &Spot, path: &SunPath, event: Event) -> Option<String> {
    s.sun.hidden_at.clone().or_else(|| s.sun.flat_sunset.clone()).or_else(|| match event {
        Event::Sunset => path.sunset.as_ref().map(|e| e.t.clone()),
        Event::Sunrise => path.sunrise.as_ref().map(|e| e.t.clone()),
    })
}

/// Which way to face: where the sun meets the skyline, else where it sets / rises.
fn facing(s: &Spot, path: &SunPath, event: Event) -> f64 {
    s.sun.hidden_az_deg.unwrap_or_else(|| match event {
        Event::Sunset => path.sunset.as_ref().map(|e| e.az_deg).unwrap_or(270.0),
        Event::Sunrise => path.sunrise.as_ref().map(|e| e.az_deg).unwrap_or(90.0),
    })
}

/// Arrive 15 min before golden hour starts, or 1 h before the shot, whichever is earlier.
fn arrive_by(shot: Option<&str>, golden_from: Option<&str>, zone: Tz) -> Option<String> {
    let a = shot.and_then(parse_t).map(|t| t - Duration::minutes(60));
    let b = golden_from.and_then(parse_t).map(|t| t - Duration::minutes(15));
    let t = match (a, b) {
        (Some(a), Some(b)) => a.min(b),
        (a, b) => a.or(b)?,
    };
    // round down to 5 minutes
    let t = t.with_timezone(&Utc);
    let t = t - Duration::seconds(t.timestamp().rem_euclid(300));
    Some(tz::iso(zone, t))
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    brief: &Brief,
    place: &Place,
    zone: Tz,
    path: &SunPath,
    picked: &[Spot],
    photos: &[Option<String>],
    buildings: &[Option<Building>],
    source: &DsmSource,
    costar: Costar,
    replan_notes: Vec<String>,
) -> CallSheet {
    let golden = golden_of(path, brief.event);
    let golden_from = golden.as_ref().map(|g| g.0.as_str());
    let date = NaiveDate::parse_from_str(&brief.date, "%Y-%m-%d").unwrap_or_default();

    let spots: Vec<SheetSpot> = picked
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let shot = shot_time(s, path, brief.event);
            let az = facing(s, path, brief.event);
            let mut notes = vec![format!("Face {az:.0}° ({})", compass(az))];
            if let Some(t) = &shot {
                notes.push(match (brief.event, s.sun.hidden_at.is_some()) {
                    (Event::Sunset, true) => format!("Sun meets the skyline at {}", hhmm(t)),
                    (Event::Sunrise, true) => format!("Sun clears the skyline at {}", hhmm(t)),
                    (Event::Sunset, false) => format!("Sun sets over a clear horizon at {}", hhmm(t)),
                    (Event::Sunrise, false) => format!("Sun rises over a clear horizon at {}", hhmm(t)),
                });
            }
            // core's reasons already include the golden-hour minutes
            notes.extend(s.why.iter().take(2).cloned());
            SheetSpot {
                role: if i == 0 { SpotRole::Primary } else { SpotRole::Backup },
                spot: s.clone(),
                arrive_by: arrive_by(shot.as_deref(), golden_from, zone).unwrap_or_else(|| brief.date.clone()),
                notes,
                photo_url: photos.get(i).cloned().flatten(),
            }
        })
        .collect();

    let primary = &spots[0];
    let primary_shot = shot_time(&primary.spot, path, brief.event);
    let mut timeline = vec![TimelineItem { t: primary.arrive_by.clone(), what: "Arrive and set up at the primary spot".into() }];
    if let Some((from, to)) = &golden {
        let (start, end) = match brief.event {
            Event::Sunset => ("Golden hour starts (sun 6° up)", "Golden hour ends; blue hour begins"),
            Event::Sunrise => ("Golden hour starts (sun 4° below the horizon)", "Golden hour ends (sun 6° up)"),
        };
        timeline.push(TimelineItem { t: from.clone(), what: start.into() });
        timeline.push(TimelineItem { t: to.clone(), what: end.into() });
    }
    if let Some(t) = &primary_shot {
        let what = match (brief.event, primary.spot.sun.hidden_at.is_some()) {
            (Event::Sunset, true) => "The shot: the sun meets the skyline",
            (Event::Sunrise, true) => "The shot: the sun clears the skyline",
            (Event::Sunset, false) => "The shot: sunset over a clear horizon",
            (Event::Sunrise, false) => "The shot: sunrise over a clear horizon",
        };
        timeline.push(TimelineItem { t: t.clone(), what: what.into() });
    }
    if brief.event == Event::Sunset && primary.spot.sun.hidden_at.is_some() {
        if let Some(e) = &path.sunset {
            timeline.push(TimelineItem { t: e.t.clone(), what: "Sunset on a flat horizon (the sky keeps glowing)".into() });
        }
    }
    timeline.sort_by(|a, b| parse_t(&a.t).cmp(&parse_t(&b.t)));
    timeline.dedup_by(|a, b| a.t == b.t);

    // who to ask, for roof spots
    let end = path.sunset.as_ref().and_then(|e| parse_t(&e.t)).map(|t| (t + Duration::minutes(30)).format("%H:%M").to_string());
    let mut access = Vec::new();
    let mut caveats = Vec::new();
    for (s, b) in spots.iter().zip(buildings) {
        if s.spot.standing != Standing::Roof {
            continue;
        }
        match b {
            // one request per building, even if several spots are on (or next to) it
            Some(b) if access.iter().any(|a: &Access| a.building_id == b.id) => {}
            Some(b) => {
                if let Some(d) = b.dist_m {
                    caveats.push(format!(
                        "Roof spot #{} isn't on a CoStar building's address point; the nearest is {} ({d:.0} m away). Check it's the right building before you call",
                        s.spot.rank,
                        b.name.as_deref().unwrap_or(&b.id)
                    ));
                }
                let contact = b
                    .contacts
                    .iter()
                    .find(|c| matches!(c.role, ContactRole::Leasing | ContactRole::Manager))
                    .or(b.contacts.first())
                    .cloned();
                access.push(Access {
                    building_id: b.id.clone(),
                    building_name: b.name.clone(),
                    contact,
                    ask: format!(
                        "Roof access {}–{} on {} for a 2–3 person photo crew, no lights; we'd stay clear of the edges",
                        hhmm(&s.arrive_by),
                        end.clone().unwrap_or_else(|| "sunset".into()),
                        date.format("%-d %B")
                    ),
                });
            }
            None if costar == Costar::Here => caveats.push(format!("Roof spot #{} isn't matched to a CoStar building, so there's no contact for it", s.spot.rank)),
            None => {}
        }
    }
    if spots.iter().any(|s| s.spot.standing == Standing::Roof) {
        match costar {
            Costar::NoKit => caveats.push("CoStar kit isn't loaded on this server, so roof spots have no access contacts yet".into()),
            Costar::NotHere => caveats.push(
                "CoStar's building data covers Los Angeles and Houston, not this area, so there are no access contacts: ask the building's front desk or manager".into(),
            ),
            Costar::Here => {}
        }
    }
    caveats.push(match source.provider.as_str() {
        "vexcel" => format!(
            "Heights: Vexcel {} (captured {}); anything built or grown since is missing",
            source.collection.clone().unwrap_or_default(),
            source.captured_from.as_deref().map(capture_day).unwrap_or_else(|| "date unknown".into())
        ),
        "usgs-3dep" => format!(
            "Heights: USGS LiDAR survey {} ({}); anything built since is missing",
            source.collection.clone().unwrap_or_default(),
            source.captured_from.clone().unwrap_or_else(|| "year unknown".into())
        ),
        other => format!("Heights: {other}"),
    });
    caveats.push("Weather isn't checked yet: look at the cloud forecast the day before".into());
    caveats.extend(replan_notes);

    let title = format!("{} · {} · {}", event_word(brief.event), framing_words(brief.framing), place.name);
    let primary_why = primary.spot.why.first().cloned().unwrap_or_default();
    let summary = format!(
        "{} on {}. {}. Arrive by {}.{}",
        match primary.spot.standing {
            Standing::Roof => "Shoot from a roof",
            _ => "Shoot from street level",
        },
        date.format("%a %-d %B"),
        primary_why.trim_end_matches('.'),
        hhmm(&primary.arrive_by),
        match spots.len() - 1 {
            0 => String::new(),
            n => format!(" {n} backup spot{} nearby.", if n == 1 { "" } else { "s" }),
        }
    );

    CallSheet { title, brief: brief.clone(), summary, timeline, spots, access, caveats, model: "mock".into(), steps: vec![] }
}

/// "Wed Jun 03 18:10:27 UTC 2026" (Vexcel) or an ISO date -> "3 Jun 2026"; anything else as is.
fn capture_day(s: &str) -> String {
    if let Ok(d) = NaiveDate::parse_from_str(s.get(..10).unwrap_or(s), "%Y-%m-%d") {
        return d.format("%-d %b %Y").to_string();
    }
    let p: Vec<&str> = s.split_whitespace().collect();
    match p[..] {
        [_, mon, day, _, _, year] => format!("{} {mon} {year}", day.trim_start_matches('0')),
        _ => s.to_string(),
    }
}

// ---- Claude writing ----------------------------------------------------------------------------

#[derive(Debug)]
struct Words {
    title: String,
    summary: String,
    notes: Vec<Vec<String>>,
    asks: Vec<String>,
}

async fn write_with_claude(g: &Claude, place: &Place, sheet: &CallSheet) -> Result<Words, String> {
    let allowed: Vec<String> = sheet
        .timeline
        .iter()
        .map(|t| hhmm(&t.t))
        .chain(sheet.spots.iter().flat_map(|s| s.notes.iter().flat_map(|n| times::clock_times(n))))
        .chain(sheet.spots.iter().map(|s| hhmm(&s.arrive_by)))
        .chain(sheet.access.iter().flat_map(|a| times::clock_times(&a.ask)))
        .collect();
    let facts = json!({
        "place": place.name,
        "brief": sheet.brief.text,
        "date": sheet.brief.date,
        "timeline": sheet.timeline.iter().map(|t| json!({"time": hhmm(&t.t), "what": t.what})).collect::<Vec<_>>(),
        "spots": sheet.spots.iter().map(|s| json!({
            "role": if s.role == SpotRole::Primary { "primary" } else { "backup" },
            "standing": s.spot.standing, "arrive_by": hhmm(&s.arrive_by), "facts": s.notes,
        })).collect::<Vec<_>>(),
        "access": sheet.access.iter().map(|a| json!({"building": a.building_name, "draft": a.ask})).collect::<Vec<_>>(),
        "caveats": sheet.caveats,
    });
    let prompt = format!(
        "Write a short call sheet for a photographer from these facts, computed by our tools:\n{facts}\n\
         Rules: use ONLY times that appear in the facts (24 h HH:MM), never invent numbers, plain friendly English. \
         title: at most 8 words. summary: 1-3 sentences. notes: one list per spot, same order, at most 4 short notes each. \
         asks: one polite one-line roof-access request per access item, same order (keep its times)."
    );
    let schema = json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "summary": { "type": "string" },
            "notes": { "type": "array", "items": { "type": "array", "items": { "type": "string" } } },
            "asks": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["title", "summary", "notes", "asks"],
        "additionalProperties": false
    });
    let v = g.json(&prompt, schema).await?;
    let strs = |v: &serde_json::Value| -> Vec<String> { v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
    let w = Words {
        title: v["title"].as_str().unwrap_or_default().to_string(),
        summary: v["summary"].as_str().unwrap_or_default().to_string(),
        notes: v["notes"].as_array().map(|a| a.iter().map(strs).collect()).unwrap_or_default(),
        asks: strs(&v["asks"]),
    };
    if w.title.is_empty() || w.summary.is_empty() {
        return Err("Claude left the title or summary empty".into());
    }
    let all: Vec<&str> = [w.title.as_str(), w.summary.as_str()]
        .into_iter()
        .chain(w.notes.iter().flatten().map(String::as_str))
        .chain(w.asks.iter().map(String::as_str))
        .collect();
    if let Some(bad) = all.iter().find(|t| !times::times_ok(t, &allowed)) {
        return Err(format!("Claude's text had a time the tools didn't compute: \"{}\"", bad.chars().take(80).collect::<String>()));
    }
    Ok(w)
}

fn apply(sheet: &mut CallSheet, w: Words) {
    sheet.title = w.title;
    sheet.summary = w.summary;
    for (s, n) in sheet.spots.iter_mut().zip(w.notes) {
        if !n.is_empty() {
            s.notes = n;
        }
    }
    for (a, ask) in sheet.access.iter_mut().zip(w.asks) {
        if !ask.trim().is_empty() {
            a.ask = ask;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrival_is_before_golden_hour_and_the_shot() {
        let zone = tz::parse("America/New_York").unwrap();
        let a = arrive_by(Some("2026-11-14T16:30:10-05:00"), Some("2026-11-14T15:58:00-05:00"), zone).unwrap();
        assert_eq!(a, "2026-11-14T15:30:00-05:00"); // 1 h before the shot (15:30) beats 15 min before golden (15:43)
        let b = arrive_by(Some("2026-11-14T16:30:10-05:00"), Some("2026-11-14T15:20:00-05:00"), zone).unwrap();
        assert_eq!(b, "2026-11-14T15:05:00-05:00");
    }

    #[test]
    fn capture_days_read_both_formats() {
        assert_eq!(capture_day("Wed Jun 03 18:10:27 UTC 2026"), "3 Jun 2026");
        assert_eq!(capture_day("2026-04-19"), "19 Apr 2026");
        assert_eq!(capture_day("2015"), "2015");
    }

    // ---- fake Claude: a local server that answers /v1/messages with a canned reply ----

    /// Serves `reply` as the Messages API answer and records the request headers and body it got.
    pub(crate) async fn fake_claude(reply: serde_json::Value) -> (Claude, Arc<std::sync::Mutex<Vec<(axum::http::HeaderMap, serde_json::Value)>>>) {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        let app = Router::new().route(
            "/v1/messages",
            post(move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
                let reply = reply.clone();
                let log = log.clone();
                async move {
                    log.lock().unwrap().push((headers, body));
                    Json(reply)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (Claude::new("test-key".into(), claude::DEFAULT_MODEL.into(), url, &reqwest::Client::new()), seen)
    }

    /// A Messages API answer whose text block is `text` (after an empty thinking block, as real answers have).
    pub(crate) fn answer(text: &str) -> serde_json::Value {
        json!({"type": "message", "role": "assistant", "stop_reason": "end_turn",
               "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": text}]})
    }

    fn test_state() -> AppState {
        AppState {
            grids: Default::default(),
            scenes: Default::default(),
            kit: None,
            http: reqwest::Client::new(),
            cache_root: std::env::temp_dir(),
        }
    }

    fn fixture_sheet() -> CallSheet {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/call_sheet.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn la_place() -> Place {
        Place { name: "Downtown LA".into(), area: Area { lat: 34.049, lon: -118.251, radius_m: 400.0 }, tz: "America/Los_Angeles".into() }
    }

    /// A brief whose words point one way; the fake model answers the other way, so we can tell who read it.
    fn brief_req() -> AgentRequest {
        AgentRequest {
            text: "sunset from the street, sun between two buildings".into(),
            area: Some(Area { lat: 34.049, lon: -118.251, radius_m: 300.0 }),
            date: None,
            tz: "America/Los_Angeles".into(),
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-06-20T18:00:00Z".parse().unwrap()
    }

    #[tokio::test]
    async fn read_uses_claudes_json() {
        let (g, seen) = fake_claude(answer(r#"{"place":"","date":"2026-06-24","event":"sunrise","standing":"roof","framing":"behind"}"#)).await;
        let mut log = Log::default();
        let (brief, _) = read_brief(&test_state(), &brief_req(), now(), Some(&g), &mut log).await.unwrap();
        assert_eq!((brief.event, brief.standing, brief.framing), (Event::Sunrise, Standing::Roof, Framing::Behind));
        assert_eq!(brief.date, "2026-06-24");
        assert!(log.0.last().unwrap().summary.ends_with("(by Claude)"));

        let seen = seen.lock().unwrap();
        let (headers, body) = &seen[0];
        assert_eq!(headers["x-api-key"], "test-key");
        assert_eq!(headers["anthropic-version"], "2023-06-01");
        assert_eq!(body["model"], claude::DEFAULT_MODEL);
        assert_eq!(body["output_config"]["format"]["schema"]["additionalProperties"], false);
    }

    #[tokio::test]
    async fn read_falls_back_to_the_offline_reader_on_bad_json() {
        let (g, _) = fake_claude(answer("Sure! It's a sunrise on a roof.")).await;
        let mut log = Log::default();
        let (brief, _) = read_brief(&test_state(), &brief_req(), now(), Some(&g), &mut log).await.unwrap();
        // the offline reader's reading of the words: sunset, street, between
        assert_eq!((brief.event, brief.standing, brief.framing), (Event::Sunset, Standing::Ground, Framing::Between));
        assert!(log.0.iter().any(|s| s.summary.starts_with("Claude couldn't read the brief")));
        assert!(log.0.last().unwrap().summary.ends_with("(by the offline reader)"));
    }

    #[tokio::test]
    async fn write_accepts_words_with_tool_times() {
        let words = json!({"title": "Two towers at sunset", "summary": "The sun drops into the gap at 19:58. Arrive by 19:00.",
                           "notes": [["Face west-north-west"], []], "asks": ["May we use the roof 19:00–20:30 on 24 June?"]});
        let (g, _) = fake_claude(answer(&words.to_string())).await;
        let mut sheet = fixture_sheet();
        let w = write_with_claude(&g, &la_place(), &sheet).await.unwrap();
        apply(&mut sheet, w);
        assert_eq!(sheet.title, "Two towers at sunset");
        assert_eq!(sheet.spots[0].notes, vec!["Face west-north-west"]);
        assert_eq!(sheet.spots[1].notes, vec!["Happens 17 minutes earlier than the primary"]); // empty list keeps the template
        assert_eq!(sheet.access[0].ask, "May we use the roof 19:00–20:30 on 24 June?");
    }

    #[tokio::test]
    async fn write_rejects_a_made_up_time() {
        let words = json!({"title": "Two towers", "summary": "The sun sets at 20:15.", "notes": [], "asks": []});
        let (g, _) = fake_claude(answer(&words.to_string())).await;
        let err = write_with_claude(&g, &la_place(), &fixture_sheet()).await.unwrap_err();
        assert!(err.contains("20:15"), "{err}");
    }

    #[tokio::test]
    async fn write_rejects_bad_json_and_errors() {
        let (g, _) = fake_claude(answer("{\"title\": ")).await;
        assert!(write_with_claude(&g, &la_place(), &fixture_sheet()).await.unwrap_err().contains("JSON"));
        let (g, _) = fake_claude(json!({"type": "message", "stop_reason": "refusal", "content": []})).await;
        assert!(write_with_claude(&g, &la_place(), &fixture_sheet()).await.unwrap_err().contains("declined"));
    }

    // ---- the whole agent, offline: synthetic grid, no model (the mock), no network ----

    fn agent_req(text: &str) -> AgentRequest {
        AgentRequest {
            text: text.into(),
            area: Some(Area { lat: crate::testkit::LAT, lon: crate::testkit::LON, radius_m: 300.0 }),
            date: Some("2026-06-21".into()),
            tz: "America/Los_Angeles".into(),
        }
    }

    #[tokio::test]
    async fn mock_agent_writes_a_sheet_from_the_tools() {
        let st = crate::testkit::state(crate::testkit::city());
        let sheet = run_with(&st, agent_req("sunset, clear view, roof or street"), now(), None).await.unwrap();
        assert_eq!(sheet.model, "mock");
        assert_eq!(sheet.brief.date, "2026-06-21");
        assert_eq!(sheet.spots.iter().filter(|s| s.role == SpotRole::Primary).count(), 1);
        assert_eq!(sheet.spots[0].role, SpotRole::Primary);
        assert!(sheet.spots.len() <= 3);
        let tools: Vec<&str> = sheet.steps.iter().map(|s| s.tool.as_str()).collect();
        for t in ["read", "search", "sun", "write"] {
            assert!(tools.contains(&t), "no {t} step in {tools:?}");
        }
        assert!(!tools.contains(&"plan"), "nothing to re-plan: {tools:?}");
        assert!(sheet.caveats.iter().any(|c| c.contains("offline mock")));
        // Synthetic heights are not Vexcel, so no photo was fetched.
        assert!(sheet.spots.iter().all(|s| s.photo_url.is_none()));
    }

    #[tokio::test]
    async fn mock_agent_relaxes_a_brief_nothing_fits() {
        // Flat ground: no roofs and no buildings to frame the sun, so "between, from a roof" finds nothing.
        let st = crate::testkit::state(crate::testkit::grid(&[]));
        let sheet = run_with(&st, agent_req("sunset between two buildings from a rooftop"), now(), None).await.unwrap();
        assert!(sheet.steps.iter().any(|s| s.tool == "plan" && s.summary.contains("relaxing to any open view")), "{:?}", sheet.steps);
        assert_eq!((sheet.brief.framing, sheet.brief.standing), (Framing::Clear, Standing::Any));
        assert_eq!(sheet.spots.iter().filter(|s| s.role == SpotRole::Primary).count(), 1);
        assert_eq!(sheet.spots[0].spot.standing, Standing::Ground);
    }

    #[test]
    fn time_zone_guess_by_longitude() {
        assert_eq!(tz_guess(-74.66), "America/New_York");
        assert_eq!(tz_guess(-95.37), "America/Chicago");
        assert_eq!(tz_guess(-118.25), "America/Los_Angeles");
    }
}
