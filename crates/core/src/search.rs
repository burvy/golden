//! Spot search: scan an area of the height map and rank places that fit a `Brief`
//! (docs/contracts.md#searchresult-and-spot). Pure: the caller passes the grid, the brief and
//! the UTC offset of the local day (the server resolves `brief.tz`, and adds `took_ms`).
//!
//! 1. **Candidates.** One per cell of a grid of at most 40 x 40 over the area circle. That grid
//!    is also the heatmap.
//! 2. **Site.** Read the surface under each candidate and decide ground or roof: a roof stands
//!    `ROOF_MIN_M` above the lowest surface within `STREET_SEARCH_M`. The surface must also be
//!    flat enough to stand on, so no spires, parapets or tree tops. Drop candidates that don't
//!    match `brief.standing`. The eye goes `EYE_M` above the surface.
//! 3. **Fast pass.** Work out the sun's path once for the whole area (`SunTrack`, from the flat
//!    sunset back to 20° up). For each candidate, trace its skyline over only the azimuths that
//!    path crosses, plus room either side for gaps. `score::look` and `score::score` rate it.
//!    This runs as a funnel over the brief's goals (`library::goals_of`, `[sun_<framing>]` when
//!    there are none): gates (`must`) cheapest tier first, dropping a candidate at the first 0,
//!    then the weighted sum of the goal scores. Features are computed lazily per candidate, so
//!    the skyline is only traced if an expression needs a Skyline feature. A goal that names
//!    objects (`objects::bind`) has a few choices of which objects it means; at each candidate
//!    it scores its best choice that passes its gates.
//!    Goals that compare two objects' bearings also add **marching** candidates on the line from
//!    b through a and beyond (`objects::march`), where a stands in front of b; they rank like
//!    the others but aren't on the heatmap.
//! 4. **Pick.** Normalise the scores to 0..1 and take the best, at least `min_spacing_m` apart.
//! 5. **Exact pass.** For just the picked spots, run `sunblock::sun_block` on their skylines, so
//!    the times on the cards are exact to the second, and write the reasons.
//!
//! The sun moves less than 0.01° across an 800 m area, so one track serves every candidate.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use contracts::{Area, Brief, CostFn, DsmGrid, Event, Goal, HeatGrid, HorizonPoint, Object, Rated, Spot, Standing, EARTH_R};
use contracts::search::MAX_HEAT_SIDE;

use crate::expr::{score_of, ExprError, Node};
use crate::features::{self, Call, Slot, Tier};
use crate::horizon::{normalize_az, ray_max_below, Eye};
use crate::library::{self, Ready};
use crate::objects::{self, Choices, View};
use crate::score::{self, Look, SkyWindow, SunTrack, TrackSample, FLANK_LOOK_DEG, GAP_SEARCH_DEG, TRACK_TOP_DEG};
use crate::sun::{sun_position, sunset};
use crate::sunblock::sun_block;

/// Eye height above the surface, metres (contract: surface + 1.7 m).
pub const EYE_M: f64 = 1.7;
/// Smallest candidate spacing, metres. Small areas get fewer cells rather than finer ones.
pub const MIN_CELL_M: f64 = 5.0;
/// A surface this much above the local street level is a roof.
pub const ROOF_MIN_M: f64 = 3.0;
/// How far around a candidate to look for street level. Wide enough to see past a big roof
/// (centres of roofs over 100 m across still read as ground); small enough that a 5% hill
/// does not make a street look like a roof.
pub const STREET_SEARCH_M: f64 = 50.0;
/// Standing surface: heights within this radius must vary by at most `MAX_ROUGHNESS_M`.
pub const STAND_RADIUS_M: f64 = 2.0;
pub const MAX_ROUGHNESS_M: f64 = 1.0;
/// How far out the skyline is traced from every candidate. Callers must fetch at least
/// `skyline_need_m(area radius)` of height map, or rays stop early at the grid edge.
pub const SKYLINE_RADIUS_M: f64 = 1500.0;
/// Azimuth step of the skyline in the fast pass.
pub const SKY_STEP_DEG: f64 = 0.5;
/// Step along the sun's path in the fast pass.
pub const TRACK_STEP_S: i64 = 10;
/// Picked spots are at least this far apart (or two candidate cells, if more).
pub const MIN_SPACING_M: f64 = 40.0;
/// `brief.max_spots` is capped here (contract: 1-50).
pub const MAX_SPOTS: u32 = 50;

/// What core returns; the server wraps it into a `SearchResult` with the brief and `took_ms`.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOutput {
    pub spots: Vec<Spot>,
    pub heat: HeatGrid,
    /// Candidates scored (after the standing filter).
    pub searched: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SearchError {
    /// `brief.date` isn't `YYYY-MM-DD`.
    BadDate(String),
    /// Area radius not positive, or a non-finite number.
    BadArea,
    /// The area's centre is not on the height map.
    AreaOffGrid,
    /// The sun doesn't rise or set that day (polar day or night).
    NoSunEvent,
    /// A goal names an unknown cost function, has a bad expression, or names objects that aren't
    /// there. Every problem, with its place.
    BadGoals(Vec<ExprError>),
    /// `brief.lens_deg` isn't between 1 and 179 degrees.
    BadLens(f64),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::BadDate(d) => write!(f, "bad date {d:?}, expected YYYY-MM-DD"),
            SearchError::BadArea => write!(f, "area needs finite lat/lon and a positive radius"),
            SearchError::AreaOffGrid => write!(f, "the area's centre is outside the height map"),
            SearchError::NoSunEvent => write!(f, "the sun doesn't rise or set there that day"),
            SearchError::BadGoals(errs) => {
                let lines: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
                write!(f, "bad goals: {}", lines.join("; "))
            }
            SearchError::BadLens(l) => write!(f, "lens_deg must be between 1 and 179 degrees, got {l}"),
        }
    }
}

impl std::error::Error for SearchError {}

/// Radius of height map, around the area's centre, that a search needs: a candidate on the area's
/// edge still sees `SKYLINE_RADIUS_M` of skyline in every direction.
pub fn skyline_need_m(area_radius_m: f64) -> f64 {
    area_radius_m + SKYLINE_RADIUS_M
}

/// What a search can draw on besides the height map and the brief.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    /// The cost functions a goal's `use` can name: the seeds plus any saved ones.
    pub library: &'a [CostFn],
    /// Objects in and around the area (CoStar buildings, Vexcel detections, ...), for goals
    /// that name objects. Only needs to cover `area radius + objects::OBJECT_REACH_M`.
    pub objects: &'a [Object],
}

/// Rank spots in `brief.area` for `brief`, with the seed library and no objects. `utc_offset_s`
/// is the offset of the local day (e.g. -25200 for PDT), used for the times in the results.
pub fn search(grid: &DsmGrid, brief: &Brief, utc_offset_s: i32) -> Result<SearchOutput, SearchError> {
    search_with(grid, brief, &Inputs { library: &library::seeds(), objects: &[] }, utc_offset_s)
}

/// Can `goal` run in `area` with `inputs`: a known cost function with the right args and a valid
/// expression, and every object it names found among `inputs.objects`? The same checks
/// `search_with` makes before it searches, so a caller can drop (or flag) a goal up front instead
/// of failing the whole search. Errors are prefixed `goals[0]`.
pub fn runnable(goal: &Goal, inputs: &Inputs, area: &Area) -> Result<(), Vec<ExprError>> {
    let ready = library::ready(std::slice::from_ref(goal), inputs.library)?;
    match bind_goals(&ready, inputs.objects, area) {
        Ok(_) => Ok(()),
        Err(SearchError::BadGoals(errs)) => Err(errs),
        Err(other) => Err(vec![ExprError { at: "goals[0]".into(), msg: other.to_string() }]),
    }
}

/// `search`, with the cost functions and objects the brief's goals can name.
pub fn search_with(grid: &DsmGrid, brief: &Brief, inputs: &Inputs, utc_offset_s: i32) -> Result<SearchOutput, SearchError> {
    let date = NaiveDate::parse_from_str(&brief.date, "%Y-%m-%d").map_err(|_| SearchError::BadDate(brief.date.clone()))?;
    let area = &brief.area;
    if ![area.lat, area.lon, area.radius_m].iter().all(|x| x.is_finite()) || area.radius_m <= 0.0 {
        return Err(SearchError::BadArea);
    }
    if !on_grid(grid, area.lat, area.lon) {
        return Err(SearchError::AreaOffGrid);
    }
    if !(brief.lens_deg.is_finite() && (1.0..=179.0).contains(&brief.lens_deg)) {
        return Err(SearchError::BadLens(brief.lens_deg));
    }
    let goals = library::ready(&library::goals_of(brief), inputs.library).map_err(SearchError::BadGoals)?;
    let choices = bind_goals(&goals, inputs.objects, area)?;
    let funnel = Funnel::new(&goals, &choices);
    let track = sun_track(area.lat, area.lon, date, brief.event).ok_or(SearchError::NoSunEvent)?;
    let layout = HeatLayout::for_area(area.lat, area.lon, area.radius_m);
    let marched = march_points(&goals, &choices, inputs.objects, area);
    let (mut sky, n_az) = empty_window(&track);
    let scene = Scene { grid, track: &track, objects: inputs.objects, lens_deg: brief.lens_deg, top_m: grid_top(grid), n_az };

    // Fast pass over every candidate: the heat grid's cells, then the marched points.
    let mut scored = Vec::new();
    for cell in 0..layout.len() + marched.len() {
        let at = if cell < layout.len() { layout.candidate(cell, area.radius_m) } else { Some(marched[cell - layout.len()]) };
        let Some((lat, lon)) = at else { continue };
        let Some(site) = site(grid, lat, lon) else { continue };
        if !standing_ok(brief.standing, site.standing) {
            continue;
        }
        let mut probe = Probe::new(&scene, lat, lon, site, Some(&mut sky));
        // A candidate dropped by a gate still counts as searched, with 0 on the heatmap.
        let raw = funnel.rank(&mut |c, choice| probe.value(c, choice)).unwrap_or(0.0);
        scored.push(Scored { cell, lat, lon, site, raw });
    }

    let best = scored.iter().map(|s| s.raw).fold(0.0, f64::max);
    let norm = |raw: f64| if best > 0.0 { raw / best } else { 0.0 };
    let mut heat = layout.heat_grid(area.lat, area.lon);
    for s in scored.iter().filter(|s| s.cell < layout.len()) {
        heat.values[s.cell] = Some(norm(s.raw) as f32);
    }

    let spacing = MIN_SPACING_M.max(2.0 * layout.cell_m);
    let picked = pick(&scored, brief.max_spots.min(MAX_SPOTS) as usize, spacing);
    let spots = picked
        .iter()
        .enumerate()
        .map(|(i, s)| {
            fill_skyline(grid, s.lat, s.lon, s.site.ground_m + EYE_M, scene.top_m, n_az, &mut sky);
            let ctx = SpotContext { brief, goals: &goals, funnel: &funnel, scene: &scene, date, utc_offset_s, sky: &sky };
            make_spot(&ctx, s, i as u32 + 1, norm(s.raw))
        })
        .collect();

    Ok(SearchOutput { spots, heat, searched: scored.len() as u32 })
}

// ---- the sun's path ---------------------------------------------------------------------

/// The sun's path from the flat sunset (sunrise) into the day, `TRACK_STEP_S` apart, until it
/// is `TRACK_TOP_DEG` up. None if there's no such event that day.
pub fn sun_track(lat: f64, lon: f64, date: NaiveDate, event: Event) -> Option<SunTrack> {
    let flat = flat_event(lat, lon, date, event)?;
    let toward_noon = match event {
        Event::Sunset => -1,
        Event::Sunrise => 1,
    };
    let mut samples = Vec::new();
    // At most 12 h: the sun is always past 20° or turned around by then.
    for k in 0..(12 * 3600 / TRACK_STEP_S) {
        let t = flat + Duration::seconds(k * TRACK_STEP_S * toward_noon);
        let p = sun_position(lat, lon, t);
        samples.push(TrackSample { t, az_deg: p.azimuth_deg, elev_deg: p.elevation_deg });
        let turned = samples.len() > 1 && p.elevation_deg < samples[samples.len() - 2].elev_deg;
        if p.elevation_deg >= TRACK_TOP_DEG || turned {
            break;
        }
    }
    Some(SunTrack { samples, step_s: TRACK_STEP_S })
}

/// Flat-horizon sunset or sunrise, UTC.
fn flat_event(lat: f64, lon: f64, date: NaiveDate, event: Event) -> Option<DateTime<Utc>> {
    match event {
        Event::Sunset => sunset(lat, lon, date).ok().map(|s| s.time),
        Event::Sunrise => {
            // sun.rs has no sunrise yet; sunblock finds it, and reports it as `flat_sunset`
            // when asked about a sunrise. An empty skyline makes that cheap.
            let iso = sun_block(&[], lat, lon, date, 0, Event::Sunrise).flat_sunset?;
            DateTime::parse_from_rfc3339(&iso).ok().map(|t| t.with_timezone(&Utc))
        }
    }
}

/// An empty skyline window covering every azimuth on the track, plus room for a gap's sides,
/// and how many azimuths it holds.
fn empty_window(track: &SunTrack) -> (SkyWindow, usize) {
    let center_az = track.samples.first().map_or(270.0, |s| s.az_deg);
    let rel = track.samples.iter().map(|s| score::wrap180(s.az_deg - center_az));
    let (lo, hi) = rel.fold((0.0f64, 0.0f64), |(lo, hi), r| (lo.min(r), hi.max(r)));
    let side = GAP_SEARCH_DEG + FLANK_LOOK_DEG + SKY_STEP_DEG;
    let lo_deg = lo.min(0.0) - side;
    let hi_deg = hi.max(0.0) + side;
    let n = ((hi_deg - lo_deg) / SKY_STEP_DEG).ceil() as usize + 1;
    (SkyWindow { center_az, lo_deg, step_deg: SKY_STEP_DEG, elev: Vec::with_capacity(n), dist: Vec::with_capacity(n) }, n)
}

/// Trace the skyline seen from `eye_m` at (lat, lon) into `sky` (`n` azimuths), reusing its buffers.
fn fill_skyline(grid: &DsmGrid, lat: f64, lon: f64, eye_m: f64, top_m: f64, n: usize, sky: &mut SkyWindow) {
    let (col, row) = grid.cell_of(lat, lon);
    let eye = Eye { col, row, eye_m };
    sky.elev.clear();
    sky.dist.clear();
    for i in 0..n {
        let m = ray_max_below(grid, eye, sky.az_of(i), SKYLINE_RADIUS_M, top_m);
        sky.elev.push(m.elev_deg);
        sky.dist.push(m.dist_m);
    }
}

/// Highest height in the grid (lets every ray stop early).
fn grid_top(grid: &DsmGrid) -> f64 {
    let heights = grid.data.iter().filter(|&&h| h != grid.meta.nodata);
    f64::from(heights.fold(f32::MIN, |a, &h| a.max(h)))
}

// ---- candidates and sites ---------------------------------------------------------------

/// The candidate grid, which is also the heatmap: square, centred on the area, row 0 north.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HeatLayout {
    cell_m: f64,
    side: u32,
    center_lat: f64,
    center_lon: f64,
}

impl HeatLayout {
    fn for_area(lat: f64, lon: f64, radius_m: f64) -> HeatLayout {
        let cell_m = (2.0 * radius_m / f64::from(MAX_HEAT_SIDE)).max(MIN_CELL_M);
        let side = ((2.0 * radius_m / cell_m).ceil() as u32).clamp(1, MAX_HEAT_SIDE);
        HeatLayout { cell_m, side, center_lat: lat, center_lon: lon }
    }

    fn len(&self) -> usize {
        (self.side * self.side) as usize
    }

    /// Lat/lon of the centre of heat cell `i`, or None if it's outside the area circle.
    fn candidate(&self, i: usize, radius_m: f64) -> Option<(f64, f64)> {
        let half = f64::from(self.side) / 2.0;
        let (col, row) = ((i as u32 % self.side) as f64, (i as u32 / self.side) as f64);
        let east = (col + 0.5 - half) * self.cell_m;
        let north = (half - row - 0.5) * self.cell_m;
        if east.hypot(north) > radius_m {
            return None;
        }
        Some(offset_latlon(self.center_lat, self.center_lon, east, north))
    }

    fn heat_grid(&self, lat: f64, lon: f64) -> HeatGrid {
        HeatGrid {
            center_lat: lat,
            center_lon: lon,
            cell_m: self.cell_m,
            width: self.side,
            height: self.side,
            values: vec![None; self.len()],
        }
    }
}

/// The point `east`, `north` metres from (lat, lon), with the same flat formula as `DsmGrid`.
fn offset_latlon(lat: f64, lon: f64, east: f64, north: f64) -> (f64, f64) {
    (lat + (north / EARTH_R).to_degrees(), lon + (east / (EARTH_R * lat.to_radians().cos())).to_degrees())
}

/// Ground distance between two nearby points, metres.
fn metres_between(a: (f64, f64), b: (f64, f64)) -> f64 {
    let north = (b.0 - a.0).to_radians() * EARTH_R;
    let east = (b.1 - a.1).to_radians() * EARTH_R * a.0.to_radians().cos();
    east.hypot(north)
}

/// Where someone could stand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Site {
    pub ground_m: f64,
    pub standing: Standing,
    /// Surface height above the lowest surface nearby (street level).
    pub above_street_m: f64,
}

/// The standing place at (lat, lon), or None if there's no data or it's too rough to stand on.
pub fn site(grid: &DsmGrid, lat: f64, lon: f64) -> Option<Site> {
    let ground_m = f64::from(grid.height_at(lat, lon)?);
    let (c, r) = grid.cell_of(lat, lon);
    let (c, r) = (c as i64, r as i64);
    let cells = |m: f64| (m / grid.meta.cell_m).round().max(1.0) as i64;

    let (lo, hi) = patch_range(grid, c, r, cells(STAND_RADIUS_M))?;
    if hi - lo > MAX_ROUGHNESS_M {
        return None;
    }
    let street = local_min(grid, c, r, cells(STREET_SEARCH_M)).unwrap_or(ground_m);
    let above_street_m = ground_m - street;
    let standing = if above_street_m >= ROOF_MIN_M { Standing::Roof } else { Standing::Ground };
    Some(Site { ground_m, standing, above_street_m })
}

/// (min, max) height in the square of `rad` cells around (c, r); None if any of it is
/// nodata or off the grid.
fn patch_range(grid: &DsmGrid, c: i64, r: i64, rad: i64) -> Option<(f64, f64)> {
    let mut range = (f64::INFINITY, f64::NEG_INFINITY);
    for rr in r - rad..=r + rad {
        for cc in c - rad..=c + rad {
            let h = cell(grid, cc, rr)?;
            range = (range.0.min(h), range.1.max(h));
        }
    }
    Some(range)
}

/// Lowest height with data in the square of `rad` cells around (c, r).
fn local_min(grid: &DsmGrid, c: i64, r: i64, rad: i64) -> Option<f64> {
    let mut low: Option<f64> = None;
    for rr in r - rad..=r + rad {
        for cc in c - rad..=c + rad {
            if let Some(h) = cell(grid, cc, rr) {
                low = Some(low.map_or(h, |l| l.min(h)));
            }
        }
    }
    low
}

fn cell(grid: &DsmGrid, c: i64, r: i64) -> Option<f64> {
    let (w, h) = (i64::from(grid.meta.width), i64::from(grid.meta.height));
    if c < 0 || r < 0 || c >= w || r >= h {
        return None;
    }
    let v = grid.data[(r * w + c) as usize];
    (v != grid.meta.nodata).then_some(f64::from(v))
}

fn on_grid(grid: &DsmGrid, lat: f64, lon: f64) -> bool {
    let (c, r) = grid.cell_of(lat, lon);
    c >= 0.0 && r >= 0.0 && c < f64::from(grid.meta.width) && r < f64::from(grid.meta.height)
}

fn standing_ok(wanted: Standing, got: Standing) -> bool {
    wanted == Standing::Any || wanted == got
}

// ---- picking and spots ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
struct Scored {
    /// Heat cell, or `layout.len() + k` for the k-th marched point.
    cell: usize,
    lat: f64,
    lon: f64,
    site: Site,
    raw: f64,
}

/// The best `n` candidates with a positive score, each at least `spacing_m` from those
/// already picked. Ties go to the earlier (more northern, then western) cell, so it's stable.
fn pick(scored: &[Scored], n: usize, spacing_m: f64) -> Vec<Scored> {
    let mut order: Vec<&Scored> = scored.iter().filter(|s| s.raw > 0.0).collect();
    order.sort_by(|a, b| b.raw.total_cmp(&a.raw).then(a.cell.cmp(&b.cell)));
    let mut picked: Vec<Scored> = Vec::with_capacity(n);
    for s in order {
        if picked.len() == n {
            break;
        }
        if picked.iter().all(|p| metres_between((p.lat, p.lon), (s.lat, s.lon)) >= spacing_m) {
            picked.push(*s);
        }
    }
    picked
}

struct SpotContext<'a> {
    brief: &'a Brief,
    goals: &'a [Ready],
    funnel: &'a Funnel<'a>,
    scene: &'a Scene<'a>,
    date: NaiveDate,
    utc_offset_s: i32,
    sky: &'a SkyWindow,
}

/// A finished spot, with the exact `SunBlock` from its skyline and the reasons.
fn make_spot(ctx: &SpotContext, s: &Scored, rank: u32, score_01: f64) -> Spot {
    let sky = ctx.sky;
    let points: Vec<HorizonPoint> = (0..sky.elev.len())
        .map(|i| HorizonPoint { az_deg: normalize_az(sky.az_of(i)), elev_deg: sky.elev[i], dist_m: sky.dist[i] })
        .collect();
    let sun = sun_block(&points, s.lat, s.lon, ctx.date, ctx.utc_offset_s, ctx.brief.event);
    let look = score::look(ctx.scene.track, sky);
    let place = match s.site.standing {
        Standing::Roof => format!("Roof {:.0} m above the street", s.site.above_street_m),
        _ => "At street level".to_string(),
    };
    let mut probe = Probe::new(ctx.scene, s.lat, s.lon, s.site, None);
    probe.look = Some(look);
    let mut value = |c: Call, choice: &[usize]| probe.value(c, choice);
    let picks = ctx.funnel.choose(&mut value);
    let scores: Vec<Rated> = ctx
        .goals
        .iter()
        .zip(&ctx.funnel.goals)
        .zip(picks)
        .map(|((g, run), k)| rate(g, &run.choices[k], ctx.scene.objects, &mut value))
        .collect();
    let why = if ctx.brief.goals.is_empty() {
        score::why(ctx.brief.framing, ctx.brief.event, &look, sky, &sun, place)
    } else {
        let golden = format!("{} min of golden hour with the sun in view", sun.golden_min_visible);
        scores.iter().map(|r| r.why.clone()).chain([golden, place]).collect()
    };
    let (lat, lon) = (round5(s.lat), round5(s.lon));
    Spot {
        id: spot_id(lat, lon, s.site.standing),
        rank,
        score: (score_01 * 1000.0).round() / 1000.0,
        lat,
        lon,
        standing: s.site.standing,
        ground_m: s.site.ground_m,
        eye_m: s.site.ground_m + EYE_M,
        sun,
        why,
        building_id: None,
        scores,
    }
}

// ---- the funnel -------------------------------------------------------------------------

/// One goal in the funnel.
struct GoalRun<'a> {
    goal: &'a Ready,
    /// Which objects the goal can mean (`objects::bind`); one empty choice if it names none.
    choices: &'a Choices,
    /// What must be above 0 for a choice to pass: the function's own `must`, and its score if
    /// the goal is a must. Cheapest tier first. Empty: the goal never drops a candidate.
    checks: Vec<&'a Node>,
}

/// The brief's goals, arranged for the candidate loop.
struct Funnel<'a> {
    goals: Vec<GoalRun<'a>>,
    /// The goals with checks, cheapest tier first, so a candidate is dropped before anything
    /// expensive is computed.
    gated: Vec<usize>,
    /// (goal, weight) of the goals that rank. If no goal has a weight, all goals rank equally.
    ranked: Vec<(usize, f64)>,
}

impl<'a> Funnel<'a> {
    fn new(goals: &'a [Ready], choices: &'a [Choices]) -> Funnel<'a> {
        let runs: Vec<GoalRun> = goals
            .iter()
            .zip(choices)
            .map(|(g, choices)| {
                let mut checks: Vec<&Node> = g.cost.must.iter().chain(g.must.then_some(&g.cost.score)).collect();
                checks.sort_by_key(|n| n.tier());
                GoalRun { goal: g, choices, checks }
            })
            .collect();
        let mut gated: Vec<usize> = (0..runs.len()).filter(|&i| !runs[i].checks.is_empty()).collect();
        gated.sort_by_key(|&i| runs[i].checks.iter().map(|n| n.tier()).max());
        let mut ranked: Vec<(usize, f64)> = (0..goals.len()).filter(|&i| goals[i].weight > 0.0).map(|i| (i, goals[i].weight)).collect();
        if ranked.is_empty() {
            ranked = (0..goals.len()).map(|i| (i, 1.0 / goals.len() as f64)).collect();
        }
        Funnel { goals: runs, gated, ranked }
    }

    /// The candidate's ranking score, or None if a gate drops it. `value(call, choice)` is a
    /// feature's number with the goal's objects as picked by `choice`.
    fn rank(&self, value: &mut impl FnMut(Call, &[usize]) -> f64) -> Option<f64> {
        let passing = self.passing(value)?;
        // reduce, not a fold from 0.0: one goal of weight 1 gives back its score bit for bit.
        Some(self.ranked.iter().map(|&(i, w)| w * self.best(i, passing[i], value).1).reduce(|a, b| a + b).unwrap_or(0.0))
    }

    /// For each goal, the choice of objects that scores best among those passing its checks
    /// (or among all, if none pass): what a spot's reasons are about.
    fn choose(&self, value: &mut impl FnMut(Call, &[usize]) -> f64) -> Vec<usize> {
        let passing = self.passing(value).unwrap_or_else(|| vec![u64::MAX; self.goals.len()]);
        (0..self.goals.len()).map(|i| self.best(i, if passing[i] == 0 { u64::MAX } else { passing[i] }, value).0).collect()
    }

    /// Per goal, a bit mask of the choices that pass its checks (all bits for an ungated goal).
    /// None as soon as a gated goal has no passing choice.
    fn passing(&self, value: &mut impl FnMut(Call, &[usize]) -> f64) -> Option<Vec<u64>> {
        let mut passing = vec![u64::MAX; self.goals.len()];
        for &i in &self.gated {
            let run = &self.goals[i];
            let mut mask = 0u64;
            for (k, choice) in run.choices.iter().enumerate() {
                if run.checks.iter().all(|n| score_of(n, &mut |c| value(c, choice)) > 0.0) {
                    mask |= 1 << k;
                }
            }
            if mask == 0 {
                return None;
            }
            passing[i] = mask;
        }
        Some(passing)
    }

    /// (choice, score) of goal `i`'s best choice among those in `mask`.
    fn best(&self, i: usize, mask: u64, value: &mut impl FnMut(Call, &[usize]) -> f64) -> (usize, f64) {
        let run = &self.goals[i];
        let mut best = (0, f64::NEG_INFINITY);
        for (k, choice) in run.choices.iter().enumerate().filter(|(k, _)| mask & (1 << k) != 0) {
            let s = score_of(&run.goal.cost.score, &mut |c| value(c, choice));
            if s > best.1 {
                best = (k, s);
            }
        }
        best
    }
}

/// Each goal's object choices, or every goal whose objects aren't there.
fn bind_goals(goals: &[Ready], objects: &[Object], area: &Area) -> Result<Vec<Choices>, SearchError> {
    let mut out = Vec::with_capacity(goals.len());
    let mut errors = Vec::new();
    for (i, g) in goals.iter().enumerate() {
        match objects::bind(&g.refs, objects, (area.lat, area.lon), area.radius_m) {
            Ok(c) => out.push(c),
            Err(msg) => errors.push(ExprError { at: format!("goals[{i}]"), msg }),
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(SearchError::BadGoals(errors))
    }
}

/// Extra candidates inside the area: for every feature in a goal that compares two objects
/// (`bearing_diff`, `in_frame`), the line from b through a and beyond, for the goal's first
/// `objects::MAX_MARCH_CHOICES` choices. Capped at `objects::MAX_MARCH_POINTS`.
fn march_points(goals: &[Ready], choices: &[Choices], objects: &[Object], area: &Area) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for (g, choices) in goals.iter().zip(choices) {
        let mut pairs: Vec<(Slot, Slot)> = Vec::new();
        for node in g.cost.must.iter().chain([&g.cost.score]) {
            node.each_feature(&mut |c| {
                if let [Some(a), Some(b)] = c.objs {
                    if !pairs.contains(&(a, b)) {
                        pairs.push((a, b));
                    }
                }
            });
        }
        for choice in choices.iter().take(objects::MAX_MARCH_CHOICES) {
            for &(a, b) in &pairs {
                let (a, b) = (&objects[choice[usize::from(a)]], &objects[choice[usize::from(b)]]);
                for p in objects::march(a, b) {
                    if metres_between((area.lat, area.lon), p) > area.radius_m {
                        continue;
                    }
                    out.push(p);
                    if out.len() == objects::MAX_MARCH_POINTS {
                        return out;
                    }
                }
            }
        }
    }
    out
}

/// What every candidate's features are computed from.
struct Scene<'a> {
    grid: &'a DsmGrid,
    track: &'a SunTrack,
    objects: &'a [Object],
    lens_deg: f64,
    /// Highest height in the grid (lets skyline rays stop early).
    top_m: f64,
    /// Azimuths in the skyline window.
    n_az: usize,
}

/// One candidate's features, computed when first asked for and kept: the `Look` (one skyline
/// trace) for Skyline features, one line-of-sight march per object for `visible`.
struct Probe<'a> {
    scene: &'a Scene<'a>,
    site: Site,
    view: View,
    look: Option<Look>,
    /// Where to trace the skyline. None if `look` is already set.
    sky: Option<&'a mut SkyWindow>,
    /// (object index, visible) so far.
    seen: Vec<(usize, f64)>,
}

impl<'a> Probe<'a> {
    fn new(scene: &'a Scene<'a>, lat: f64, lon: f64, site: Site, sky: Option<&'a mut SkyWindow>) -> Probe<'a> {
        let view = View { lat, lon, eye_m: site.ground_m + EYE_M, lens_deg: scene.lens_deg };
        Probe { scene, site, view, look: None, sky, seen: Vec::new() }
    }

    /// One feature, with the goal's objects as picked by `choice` (slot -> object index).
    fn value(&mut self, c: Call, choice: &[usize]) -> f64 {
        let objects = self.scene.objects;
        let index = |k: usize| c.objs[k].map(|s| choice[usize::from(s)]);
        let objs = [index(0).map(|i| &objects[i]), index(1).map(|i| &objects[i])];
        if let Some(v) = features::point_value(c.f, &self.site, &self.view, objs) {
            return v;
        }
        match c.f.tier() {
            Tier::Point => unreachable!("every Point feature has a point_value"),
            Tier::Ray => {
                let Some(i) = index(0) else { return f64::NAN };
                if let Some(&(_, v)) = self.seen.iter().find(|(j, _)| *j == i) {
                    return v;
                }
                let v = features::ray_value(c.f, self.scene.grid, &self.view, Some(&objects[i])).expect("Ray feature");
                self.seen.push((i, v));
                v
            }
            Tier::Skyline => {
                let (scene, view) = (self.scene, self.view);
                let sky = &mut self.sky;
                let look = self.look.get_or_insert_with(|| {
                    let sky = sky.as_deref_mut().expect("a probe without a skyline buffer has its look set");
                    fill_skyline(scene.grid, view.lat, view.lon, view.eye_m, scene.top_m, scene.n_az, sky);
                    score::look(scene.track, sky)
                });
                features::sun_value(c.f, look, scene.track, &view, objs[0]).expect("Skyline feature")
            }
        }
    }
}

/// A spot's score and reason for one goal, for the objects in `choice`: the function's `about`,
/// the numbers it used, and which objects it means.
fn rate(goal: &Ready, choice: &[usize], objects: &[Object], value: &mut impl FnMut(Call, &[usize]) -> f64) -> Rated {
    let score = score_of(&goal.cost.score, &mut |c| value(c, choice));
    let mut used: Vec<Call> = Vec::new();
    goal.cost.score.each_feature(&mut |c| {
        if !used.contains(&c) {
            used.push(c);
        }
    });
    let numbers: Vec<String> = used
        .iter()
        .take(4)
        .map(|&c| {
            let v = value(c, choice);
            let name = goal.cost.call_text(c);
            if v.abs() <= 1.0 {
                format!("{name} {v:.2}")
            } else {
                format!("{name} {v:.1}")
            }
        })
        .collect();
    let mut why = if numbers.is_empty() { goal.cost.about.clone() } else { format!("{} ({})", goal.cost.about, numbers.join(", ")) };
    // Which objects it used, unless the reason already says (an object named by id).
    let names: Vec<String> = choice
        .iter()
        .enumerate()
        .map(|(s, &o)| (goal.cost.label(s as Slot), &objects[o].id))
        .filter(|(label, id)| label != *id)
        .map(|(label, id)| format!("{label}: {id}"))
        .collect();
    if !names.is_empty() {
        why = format!("{why} [{}]", names.join(", "));
    }
    Rated { goal: goal.name.clone(), score: (score * 1000.0).round() / 1000.0, why }
}

/// Stable id for a place and standing, as in the contract: `s-34.04912--118.25087-roof`.
pub fn spot_id(lat: f64, lon: f64, standing: Standing) -> String {
    let kind = match standing {
        Standing::Roof => "roof",
        Standing::Ground => "ground",
        Standing::Any => "any",
    };
    format!("s-{lat:.5}-{lon:.5}-{kind}")
}

fn round5(x: f64) -> f64 {
    (x * 1e5).round() / 1e5
}


#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{Area, DsmMeta, DsmSource, Framing, NODATA};

    const LAT: f64 = 34.049;
    const LON: f64 = -118.251;
    const PDT: i32 = -7 * 3600;

    /// A flat plain of 1 m cells, `n` wide, at 0 m; `paint(east_m, north_m)` raises cells
    /// (metres from the centre, so tests can place things by distance and direction).
    fn grid(n: u32, paint: impl Fn(f64, f64) -> Option<f32>) -> DsmGrid {
        let half = f64::from(n) / 2.0;
        let mut data = Vec::with_capacity((n * n) as usize);
        for r in 0..n {
            for c in 0..n {
                let (east, north) = (f64::from(c) + 0.5 - half, half - f64::from(r) - 0.5);
                data.push(paint(east, north).unwrap_or(0.0));
            }
        }
        let source = DsmSource {
            provider: "synthetic".into(),
            collection: None,
            captured_from: None,
            captured_to: None,
            native_res_m: 1.0,
        };
        let meta = DsmMeta { center_lat: LAT, center_lon: LON, cell_m: 1.0, width: n, height: n, nodata: NODATA, source };
        DsmGrid { meta, data }
    }

    /// A box `w` x `w` metres, `h` high, centred `dist` metres from the centre towards `az`.
    fn tower(az: f64, dist: f64, w: f64, h: f32) -> impl Fn(f64, f64) -> Option<f32> {
        let (cx, cy) = (dist * az.to_radians().sin(), dist * az.to_radians().cos());
        move |e, n| ((e - cx).abs() < w / 2.0 && (n - cy).abs() < w / 2.0).then_some(h)
    }

    fn brief(radius_m: f64, standing: Standing, framing: Framing) -> Brief {
        Brief {
            text: String::new(),
            area: Area { lat: LAT, lon: LON, radius_m },
            date: "2026-06-21".into(),
            tz: "America/Los_Angeles".into(),
            event: Event::Sunset,
            standing,
            framing,
            max_spots: 10,
            goals: Vec::new(),
            lens_deg: contracts::DEFAULT_LENS_DEG,
        }
    }

    fn sunset_az() -> f64 {
        sunset(LAT, LON, NaiveDate::from_ymd_opt(2026, 6, 21).unwrap()).unwrap().azimuth_deg
    }

    /// (east, north) metres of a spot from the area centre.
    fn en(s: &Spot) -> (f64, f64) {
        ((s.lon - LON).to_radians() * EARTH_R * LAT.to_radians().cos(), (s.lat - LAT).to_radians() * EARTH_R)
    }

    fn heat_at(h: &HeatGrid, s: &Spot) -> Option<f32> {
        let (e, n) = en(s);
        let half = f64::from(h.width) / 2.0;
        let (c, r) = ((half + e / h.cell_m).floor() as usize, (half - n / h.cell_m).floor() as usize);
        h.values[r * h.width as usize + c]
    }

    #[test]
    fn behind_ranks_spots_east_of_the_tower_and_clear_ranks_them_low() {
        // One 25 m tower, 30 m square, at the centre of a 300 m area.
        let g = grid(900, tower(0.0, 0.0, 30.0, 25.0));
        let behind = search(&g, &brief(300.0, Standing::Ground, Framing::Behind), PDT).unwrap();
        let top = &behind.spots[0];
        let (e, n) = en(top);
        // The tower is in the sunset direction from the spot, i.e. the spot is ESE of the tower.
        let az_from_tower = e.atan2(n).to_degrees().rem_euclid(360.0);
        let behind_dir = (sunset_az() + 180.0).rem_euclid(360.0);
        assert!(score::wrap180(az_from_tower - behind_dir).abs() < 10.0, "top spot at az {az_from_tower} from tower");
        assert!(top.sun.hidden_at.is_some());
        assert!(top.why[0].starts_with("Sun drops behind a building"), "{:?}", top.why);

        let clear = search(&g, &brief(300.0, Standing::Ground, Framing::Clear), PDT).unwrap();
        let clear_there = heat_at(&clear.heat, top).unwrap();
        assert!(clear_there < 0.5, "clear score at the best 'behind' spot: {clear_there}");
        assert!(clear.spots[0].sun.hidden_at.is_none(), "{:?}", clear.spots[0].sun);
    }

    #[test]
    fn between_finds_the_spot_looking_through_the_gap() {
        // Two 40 m towers, 20 m wide, 250 m out towards the sunset, with a 20 m gap between them.
        let az = sunset_az();
        let side = az + 90.0;
        let (gx, gy) = (250.0 * az.to_radians().sin(), 250.0 * az.to_radians().cos());
        let at = |off: f64| (gx + off * side.to_radians().sin(), gy + off * side.to_radians().cos());
        let (a, b) = (at(-20.0), at(20.0));
        let g = grid(800, move |e, n| {
            let near = |(x, y): (f64, f64)| (e - x).abs() < 10.0 && (n - y).abs() < 10.0;
            (near(a) || near(b)).then_some(40.0)
        });
        let out = search(&g, &brief(200.0, Standing::Ground, Framing::Between), PDT).unwrap();
        let top = &out.spots[0];
        // Distance of the top spot from the line through the gap along the sunset direction.
        let (e, n) = en(top);
        let off_line = (e - gx) * side.to_radians().sin() + (n - gy) * side.to_radians().cos();
        assert!(off_line.abs() < 15.0, "top spot {off_line:.1} m off the gap line; {:?}", top.why);
        assert!(top.why[0].contains("gap between buildings"), "{:?}", top.why);
    }

    #[test]
    fn standing_filter_is_respected() {
        // A 15 m flat-roofed block, 80 m square, at the centre.
        let g = grid(400, tower(0.0, 0.0, 80.0, 15.0));
        let roof = search(&g, &brief(100.0, Standing::Roof, Framing::Clear), PDT).unwrap();
        assert!(!roof.spots.is_empty());
        assert!(roof.spots.iter().all(|s| s.standing == Standing::Roof && s.ground_m == 15.0));
        assert!(roof.spots[0].why.last().unwrap().starts_with("Roof 15 m above the street"), "{:?}", roof.spots[0].why);
        let ground = search(&g, &brief(100.0, Standing::Ground, Framing::Clear), PDT).unwrap();
        assert!(!ground.spots.is_empty());
        assert!(ground.spots.iter().all(|s| s.standing == Standing::Ground && s.ground_m == 0.0));
        let any = search(&g, &brief(100.0, Standing::Any, Framing::Clear), PDT).unwrap();
        assert_eq!(any.searched, roof.searched + ground.searched);
        for s in &ground.spots {
            assert!((s.eye_m - 1.7).abs() < 1e-9);
            assert!(s.id.ends_with("-ground"));
        }
    }

    #[test]
    fn spots_are_spaced_apart_and_heat_is_in_range() {
        let g = grid(900, tower(0.0, 0.0, 30.0, 25.0));
        let out = search(&g, &brief(300.0, Standing::Any, Framing::Clear), PDT).unwrap();
        assert_eq!(out.spots.len(), 10);
        let spacing = MIN_SPACING_M.max(2.0 * out.heat.cell_m);
        for (i, a) in out.spots.iter().enumerate() {
            assert_eq!(a.rank, i as u32 + 1);
            for b in &out.spots[i + 1..] {
                assert!(metres_between((a.lat, a.lon), (b.lat, b.lon)) >= spacing - 1.0);
                assert!(a.score >= b.score);
            }
        }
        let h = &out.heat;
        assert!(h.width <= MAX_HEAT_SIDE && h.height <= MAX_HEAT_SIDE);
        assert_eq!(h.values.len(), (h.width * h.height) as usize);
        assert!(h.values.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
        assert!(h.values.contains(&Some(1.0)));
        assert!(h.values[0].is_none(), "corner is outside the circle");
        assert_eq!(out.spots[0].score, 1.0);
    }

    #[test]
    fn a_tower_1_km_outside_the_area_hides_the_sun() {
        // 50 m area; a 120 m block, 300 m square, centred 1050 m out towards the sunset (1 km past the
        // area's edge), wide enough that no candidate can see round it.
        // The grid is exactly what skyline_need_m asks for, so every candidate sees it.
        let n = (2.0 * skyline_need_m(50.0)).ceil() as u32;
        let open = search(&grid(n, |_, _| None), &brief(50.0, Standing::Ground, Framing::Clear), PDT).unwrap();
        assert!(open.spots[0].sun.hidden_at.is_none(), "{:?}", open.spots[0].sun);
        // With the block, nothing sees a clear sunset; "behind" ranks the spots where it sets behind the block.
        let g = grid(n, tower(sunset_az(), 1050.0, 300.0, 120.0));
        let towered = search(&g, &brief(50.0, Standing::Ground, Framing::Behind), PDT).unwrap();
        assert!(!towered.spots.is_empty());
        for s in &towered.spots {
            let sun = &s.sun;
            assert!(sun.hidden_at.is_some() && sun.hidden_at < sun.flat_sunset, "{sun:?}");
            // The block's near face is 900 m from the centre, 850-950 m from a spot: atan(118.3 / 950..850) = 7.1-7.9 deg.
            // (The old 700 m fetch margin ended 750 m out, so this face was off the grid.)
            let elev = sun.hidden_elev_deg.unwrap();
            assert!((7.0..8.0).contains(&elev), "{sun:?}");
        }
    }

    #[test]
    fn small_area_uses_5_m_cells() {
        let g = grid(200, |_, _| None);
        let out = search(&g, &brief(50.0, Standing::Any, Framing::Clear), PDT).unwrap();
        assert_eq!((out.heat.cell_m, out.heat.width), (5.0, 20));
    }

    #[test]
    fn sunrise_search_works() {
        let g = grid(900, tower(0.0, 0.0, 30.0, 25.0));
        let b = Brief { event: Event::Sunrise, ..brief(300.0, Standing::Ground, Framing::Behind) };
        let out = search(&g, &b, PDT).unwrap();
        let top = &out.spots[0];
        assert!(top.sun.flat_sunset.as_deref().unwrap().starts_with("2026-06-21T05:4"), "{:?}", top.sun);
        // Sunrise is ENE, so the spots that see it come up behind the tower are WSW of it.
        let (e, n) = en(top);
        assert!(e < 0.0, "({e}, {n})");
        assert!(top.why[0].starts_with("Sun rises from behind a building"), "{:?}", top.why);
    }

    #[test]
    fn bad_input_is_an_error() {
        let g = grid(100, |_, _| None);
        let b = Brief { date: "21/06/2026".into(), ..brief(50.0, Standing::Any, Framing::Clear) };
        assert!(matches!(search(&g, &b, PDT), Err(SearchError::BadDate(_))));
        let b = Brief { area: Area { lat: 40.0, lon: LON, radius_m: 50.0 }, ..brief(50.0, Standing::Any, Framing::Clear) };
        assert_eq!(search(&g, &b, PDT), Err(SearchError::AreaOffGrid));
        assert_eq!(search(&g, &brief(0.0, Standing::Any, Framing::Clear), PDT), Err(SearchError::BadArea));
    }

    #[test]
    fn rough_surfaces_are_not_standing_spots() {
        // A 1 m checkerboard of 0 and 5 m heights (like trees or rubble) is not standable.
        let g = grid(100, |e, n| ((e.floor() as i64 + n.floor() as i64) % 2 == 0).then_some(5.0));
        assert_eq!(site(&g, LAT, LON), None);
        let flat = grid(100, |_, _| None);
        assert_eq!(site(&flat, LAT, LON).map(|s| s.standing), Some(Standing::Ground));
    }

    #[test]
    fn spot_ids_match_the_contract() {
        assert_eq!(spot_id(34.04912, -118.25087, Standing::Roof), "s-34.04912--118.25087-roof");
    }

    #[test]
    fn fixture_grid_search_runs() {
        // tests may read files; the library itself never does
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let meta = DsmGrid::parse_meta(&std::fs::read_to_string(dir.join("dsm_grid.json")).unwrap()).unwrap();
        let g = DsmGrid::from_parts(meta.meta, &std::fs::read(dir.join(&meta.data_file)).unwrap()).unwrap();
        for framing in [Framing::Clear, Framing::Between, Framing::Behind] {
            let out = search(&g, &brief(30.0, Standing::Any, framing), PDT).unwrap();
            assert!(out.searched > 0);
            assert!(out.spots.iter().all(|s| s.sun.flat_sunset.is_some()));
        }
    }

    /// Timing on a city-like 1.6 km grid. Run with
    /// `cargo test -p golden-core --release perf_city -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn perf_city_grid() {
        // 40 m blocks on a 12 m street grid; heights 10..120 m from a hash of the block.
        let g = grid(1600, |e, n| {
            let (bx, by) = ((e + 800.0).div_euclid(52.0) as i64, (n + 800.0).div_euclid(52.0) as i64);
            let in_block = (e + 800.0).rem_euclid(52.0) >= 12.0 && (n + 800.0).rem_euclid(52.0) >= 12.0;
            let h = 10 + (bx * 7919 + by * 104_729).rem_euclid(111) as i32;
            in_block.then_some(h as f32)
        });
        for framing in [Framing::Clear, Framing::Between, Framing::Behind] {
            let t = std::time::Instant::now();
            let out = search(&g, &brief(400.0, Standing::Any, framing), PDT).unwrap();
            eprintln!("PERF {framing:?}: {} candidates, {} spots, {:?}", out.searched, out.spots.len(), t.elapsed());
            for s in out.spots.iter().take(2) {
                eprintln!("  {} {:.2} {:?}", s.id, s.score, s.why);
            }
        }
    }

    // ---- goals and the funnel ----

    fn with_goals(b: Brief, goals: &str) -> Brief {
        Brief { goals: serde_json::from_str(goals).unwrap(), ..b }
    }

    #[test]
    fn no_goals_is_the_framing_goal() {
        let g = grid(400, tower(0.0, 0.0, 80.0, 15.0));
        for framing in [Framing::Clear, Framing::Between, Framing::Behind] {
            let old = search(&g, &brief(100.0, Standing::Any, framing), PDT).unwrap();
            let name = crate::library::sun_goal_name(framing);
            let new = search(&g, &with_goals(brief(100.0, Standing::Any, framing), &format!(r#"[{{"use":"{name}"}}]"#)), PDT).unwrap();
            assert_eq!(old.heat, new.heat);
            let pick = |o: &SearchOutput| o.spots.iter().map(|s| (s.id.clone(), s.score, s.scores.clone())).collect::<Vec<_>>();
            assert_eq!(pick(&old), pick(&new));
            assert!(old.spots.iter().all(|s| s.scores.len() == 1 && s.scores[0].goal == name && s.scores[0].score > 0.0));
        }
    }

    #[test]
    fn a_point_gate_drops_candidates_and_keeps_them_off_the_ranking() {
        // Same block as the standing test: a "roof" must leaves only roof spots, like standing=roof.
        let g = grid(400, tower(0.0, 0.0, 80.0, 15.0));
        let b = with_goals(
            brief(100.0, Standing::Any, Framing::Clear),
            r#"[{"use":"sun_clear"},{"def":{"name":"on_a_roof","about":"Standing on a roof","score":{"f":"roof"}},"must":true}]"#,
        );
        let out = search(&g, &b, PDT).unwrap();
        let roof = search(&g, &brief(100.0, Standing::Roof, Framing::Clear), PDT).unwrap();
        assert!(!out.spots.is_empty());
        assert!(out.spots.iter().all(|s| s.standing == Standing::Roof));
        let ids = |o: &SearchOutput| o.spots.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&out), ids(&roof), "a weight-0 must doesn't change the ranking");
        let s = &out.spots[0];
        assert_eq!(s.scores.iter().map(|r| r.goal.as_str()).collect::<Vec<_>>(), ["sun_clear", "on_a_roof"]);
        assert_eq!(s.scores[1].score, 1.0);
        assert_eq!(s.scores[1].why, "Standing on a roof (roof 1.00)");
        assert_eq!(s.why[0], s.scores[0].why, "with goals, the reasons lead with the goals");
        // Dropped ground candidates were searched and show as 0 on the heatmap.
        let any = search(&g, &brief(100.0, Standing::Any, Framing::Clear), PDT).unwrap();
        assert_eq!(out.searched, any.searched);
        assert!(out.heat.values.iter().flatten().filter(|&&v| v == 0.0).count() >= (any.searched - roof.searched) as usize);
    }

    #[test]
    fn gates_run_cheapest_first_and_skylines_are_lazy() {
        let goals: Vec<contracts::Goal> = serde_json::from_str(
            r#"[{"use":"sun_clear","must":true},{"def":{"name":"roof","about":"On a roof","score":{"f":"roof"}},"must":true}]"#,
        )
        .unwrap();
        let ready = crate::library::ready(&goals, &crate::library::seeds()).unwrap();
        let choices = vec![vec![vec![]]; ready.len()];
        let funnel = Funnel::new(&ready, &choices);
        let tiers: Vec<Tier> = funnel.gated.iter().map(|&i| funnel.goals[i].goal.cost.tier()).collect();
        assert_eq!(tiers, [Tier::Point, Tier::Skyline]);
        let ground = Site { ground_m: 0.0, standing: Standing::Ground, above_street_m: 0.0 };
        let g = grid(10, |_, _| None);
        let track = SunTrack { samples: Vec::new(), step_s: TRACK_STEP_S };
        let scene = Scene { grid: &g, track: &track, objects: &[], lens_deg: 60.0, top_m: 0.0, n_az: 0 };
        // No skyline buffer: tracing one would panic. A ground spot fails the roof gate first.
        let mut probe = Probe::new(&scene, LAT, LON, ground, None);
        assert_eq!(funnel.rank(&mut |c, ch| probe.value(c, ch)), None);
    }

    // ---- objects ----

    /// A 30 m square, 60 m tower 200 m north and a 5 m kiosk ("fountain") 100 m north, plus
    /// the objects for them. Optionally a 15 m wall across x = 40..80, y = -60 (hides the tower
    /// from part of the south-east).
    fn plaza(wall: bool) -> (DsmGrid, Vec<Object>) {
        let g = grid(800, |e, n| {
            if e.abs() < 15.0 && (n - 200.0).abs() < 15.0 {
                Some(60.0)
            } else if e.abs() < 3.0 && (n - 100.0).abs() < 3.0 {
                Some(5.0)
            } else if wall && (40.0..80.0).contains(&e) && (n + 60.0).abs() < 1.0 {
                Some(15.0)
            } else {
                None
            }
        });
        let at = |id: &str, kind: &str, north: f64, top: f64, r: f64| {
            let (lat, lon) = objects::offset((LAT, LON), 0.0, north);
            Object { id: id.into(), kind: kind.into(), lat, lon, base_m: 0.0, top_m: top, radius_m: r, source: "test".into() }
        };
        (g, vec![at("costar:1", "building", 200.0, 60.0, 15.0), at("vexcel:fountain:1", "fountain", 100.0, 5.0, 3.0)])
    }

    fn object_search(g: &DsmGrid, objs: &[Object], radius_m: f64, goals: &str) -> Result<SearchOutput, SearchError> {
        let b = with_goals(brief(radius_m, Standing::Ground, Framing::Clear), goals);
        search_with(g, &b, &Inputs { library: &crate::library::seeds(), objects: objs }, PDT)
    }

    fn view_of(s: &Spot) -> View {
        View { lat: s.lat, lon: s.lon, eye_m: s.eye_m, lens_deg: 60.0 }
    }

    #[test]
    fn in_front_puts_the_fountain_before_the_tower() {
        let (g, objs) = plaza(false);
        let goals = r#"[{"use":"in_front","args":{"a":{"kind":"fountain"},"b":{"id":"costar:1"}},"must":true}]"#;
        let out = object_search(&g, &objs, 300.0, goals).unwrap();
        assert_eq!(out.spots.len(), 10);
        for s in &out.spots {
            let v = view_of(s);
            assert!(objects::dist(&v, &objs[1]) < 0.8 * objects::dist(&v, &objs[0]) + 1.0, "{}: fountain clearly nearer", s.id);
            assert!(objects::in_frame(&v, &objs[1], &objs[0]) > 0.0, "{}: both in frame", s.id);
            assert!(objects::bearing(&v, &objs[0]) < 60.0 || objects::bearing(&v, &objs[0]) > 300.0, "{}: looking north", s.id);
        }
        let top = &out.spots[0];
        assert_eq!(top.scores.len(), 1);
        assert!(top.scores[0].why.starts_with("a in the foreground of b"), "{}", top.scores[0].why);
        assert!(top.scores[0].why.contains("visible(b)"), "{}", top.scores[0].why);
        assert!(top.scores[0].why.ends_with("[a: vexcel:fountain:1, b: costar:1]"), "{}", top.scores[0].why);
        // Marching added candidates beyond the heat grid, south of the fountain.
        let plain = search(&g, &brief(300.0, Standing::Ground, Framing::Clear), PDT).unwrap();
        assert!(out.searched > plain.searched + 50, "{} vs {}", out.searched, plain.searched);
        assert_eq!(out.heat.values.len(), plain.heat.values.len());
    }

    #[test]
    fn lined_up_spots_sit_on_the_line_from_the_tower_through_the_fountain() {
        let (g, objs) = plaza(false);
        let goals = r#"[{"use":"lined_up","args":{"a":{"kind":"fountain"},"b":{"kind":"building"}},"must":true}]"#;
        let out = object_search(&g, &objs, 300.0, goals).unwrap();
        assert!(!out.spots.is_empty());
        for s in &out.spots {
            let v = view_of(s);
            assert!(objects::bearing_diff(&v, &objs[1], &objs[0]).abs() < 10.0, "{}", s.id);
            let (e, n) = en(s);
            assert!(n < 100.0 - 3.0 && e.abs() < 60.0, "south of the fountain, near the line: {e:.0},{n:.0}");
        }
        let (e, _) = en(&out.spots[0]);
        assert!(e.abs() < 8.0, "the best one is right on the line (a marched point): {e}");
    }

    #[test]
    fn a_visibility_gate_drops_spots_behind_the_wall() {
        let (g, objs) = plaza(true);
        let goals = r#"[{"use":"fills","args":{"a":{"id":"costar:1"}}},
            {"def":{"name":"tower_in_view","about":"The tower is mostly in view",
                    "score":{"op":"gt","x":{"f":"visible","obj":{"id":"costar:1"}},"than":{"num":0.5}}},"must":true}]"#;
        let out = object_search(&g, &objs, 300.0, goals).unwrap();
        assert!(!out.spots.is_empty());
        for s in &out.spots {
            let v = view_of(s);
            // Spot positions are rounded to ~1 m, so allow one step (0.2) below the gate.
            assert!(objects::visible(&g, &v, &objs[0]) >= 0.4, "{}", s.id);
            assert_eq!(s.scores[1].score, 1.0);
            assert_eq!(s.scores[1].why, "The tower is mostly in view (visible(costar:1) 1.00)", "an object by id isn't named twice");
        }
        // Right behind the wall (60, -80) the tower is hidden: that heat cell is 0.
        let h = &out.heat;
        let (c, r) = (((f64::from(h.width) / 2.0) + 60.0 / h.cell_m) as usize, ((f64::from(h.width) / 2.0) + 80.0 / h.cell_m) as usize);
        assert_eq!(h.values[r * h.width as usize + c], Some(0.0));
    }

    #[test]
    fn kind_near_picks_the_object_near_the_other() {
        // Two fountains: one by the tower, one 250 m south. The area is around the southern one.
        // Without a constraint each spot names whichever fountain works best from there.
        let (g, mut objs) = plaza(false);
        let (lat, lon) = objects::offset((LAT, LON), 0.0, -250.0);
        objs.push(Object { id: "vexcel:fountain:2".into(), lat, lon, ..objs[1].clone() });
        let run = |goals: &str| {
            let mut b = with_goals(brief(100.0, Standing::Ground, Framing::Clear), goals);
            b.area = Area { lat, lon, radius_m: 100.0 };
            search_with(&g, &b, &Inputs { library: &crate::library::seeds(), objects: &objs }, PDT).unwrap()
        };
        let out = run(r#"[{"use":"in_front","args":{"a":{"kind":"fountain"},"b":{"id":"costar:1"}}}]"#);
        assert!(!out.spots.is_empty());
        let named = |s: &Spot| if s.scores[0].why.contains("a: vexcel:fountain:2,") { &objs[2] } else { &objs[1] };
        for s in &out.spots {
            let v = view_of(s);
            assert!(objects::dist(&v, named(s)) < 0.8 * objects::dist(&v, &objs[0]) + 1.0, "{}: the named fountain is in front", s.id);
        }
        // `within_m` alone: within 50 m of the area's centre, so the southern one.
        let out = run(r#"[{"use":"in_front","args":{"a":{"kind":"fountain","within_m":50},"b":{"id":"costar:1"}}}]"#);
        assert!(!out.spots.is_empty());
        assert!(out.spots.iter().all(|s| s.scores[0].why.contains("a: vexcel:fountain:2,")), "{:?}", out.spots[0].scores);
        // `near` the tower: the northern one.
        let out = run(r#"[{"use":"in_front","args":{"a":{"kind":"fountain","near":{"id":"costar:1"},"within_m":150},"b":{"id":"costar:1"}}}]"#);
        assert!(!out.spots.is_empty());
        assert!(out.spots.iter().all(|s| s.scores[0].why.contains("a: vexcel:fountain:1,")), "{:?}", out.spots[0].scores);
    }

    #[test]
    fn missing_objects_and_bad_lens_are_errors() {
        let (g, objs) = plaza(false);
        let goals = r#"[{"use":"sun_clear"},{"use":"fills","args":{"a":{"kind":"swimming pool"}}},{"use":"fills","args":{"a":{"id":"costar:2"}}}]"#;
        match object_search(&g, &objs, 300.0, goals) {
            Err(SearchError::BadGoals(errs)) => {
                let text: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
                assert_eq!(text.len(), 2, "{text:?}");
                assert!(text[0].starts_with("goals[1]: no object of kind \"swimming pool\"") && text[0].contains("building x1, fountain x1"), "{text:?}");
                assert!(text[1].starts_with("goals[2]: no object with id \"costar:2\""), "{text:?}");
            }
            other => panic!("expected BadGoals, got {other:?}"),
        }
        let b = Brief { lens_deg: 0.0, ..brief(100.0, Standing::Any, Framing::Clear) };
        assert_eq!(search(&g, &b, PDT), Err(SearchError::BadLens(0.0)));
    }

    #[test]
    fn runnable_checks_the_library_and_the_objects() {
        let (_, objs) = plaza(false);
        let lib = crate::library::seeds();
        let area = Area { lat: LAT, lon: LON, radius_m: 300.0 };
        let goal = |s: &str| -> Goal { serde_json::from_str(s).unwrap() };
        let none: [Object; 0] = [];
        let with = |objects| Inputs { library: &lib, objects };
        let front = goal(r#"{"use":"in_front","args":{"a":{"kind":"fountain"},"b":{"kind":"building"}}}"#);
        assert_eq!(runnable(&goal(r#"{"use":"sun_clear"}"#), &with(&none[..]), &area), Ok(()));
        assert_eq!(runnable(&front, &with(&objs[..]), &area), Ok(()));
        // a seed, but no fountain known here: not runnable, and the reason says so
        let errs = runnable(&front, &with(&none[..]), &area).unwrap_err();
        assert!(errs[0].to_string().starts_with("goals[0]: no object of kind \"fountain\""), "{errs:?}");
        let errs = runnable(&goal(r#"{"use":"golden_retriever"}"#), &with(&objs[..]), &area).unwrap_err();
        assert!(errs[0].to_string().starts_with("goals[0].use: no cost function"), "{errs:?}");
    }

    #[test]
    fn bad_goals_are_reported_not_searched() {
        let g = grid(200, |_, _| None);
        let b = with_goals(brief(50.0, Standing::Any, Framing::Clear), r#"[{"use":"sun_clear"},{"use":"golden_retriever"}]"#);
        match search(&g, &b, PDT) {
            Err(SearchError::BadGoals(errs)) => {
                assert_eq!(errs.len(), 1);
                assert!(errs[0].to_string().starts_with("goals[1].use: no cost function \"golden_retriever\""), "{errs:?}");
            }
            other => panic!("expected BadGoals, got {other:?}"),
        }
    }
}
