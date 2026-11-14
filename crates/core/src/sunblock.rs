//! When the skyline hides the sun: turns a `HorizonProfile`'s points into a `SunBlock`
//! (docs/contracts.md#horizonprofile, the `sun` field).
//!
//! The idea in one line: walk the sun along its path with `sun::sun_position`, and at each
//! instant compare its elevation with the skyline's elevation in the sun's direction.
//! Crossings are found by stepping a minute at a time, then bisecting down to a second.
//!
//! Rules: only things *above* the flat horizon can block the sun (see `blocks_sun`).
//! `golden_min_visible` counts minutes with the sun's disc in view, so only the part of the
//! golden hour between +6° and the flat sunset (sunrise: flat sunrise and +6°). The -4°..0°
//! afterglow is golden light but no sun, so it doesn't count.
//!
//! Pure: the caller passes the date and the UTC offset in; nothing here reads a clock or a
//! time-zone database (the server resolves the IANA zone to an offset).

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use contracts::sun::GOLDEN_HOUR_MAX_ELEV_DEG;
use contracts::{Event, HorizonPoint, SunBlock};

use crate::sun::{sun_position, sunset, SunPosition};

/// Apparent (refracted) elevation of the sun's centre when its top edge touches a flat
/// horizon. NOAA's sunset is the geometric centre at -0.833°; NOAA's own refraction at that
/// angle is +0.397°, so `sun_position` reports -0.436° at the sunset instant.
const FLAT_HORIZON_APPARENT_ELEV_DEG: f64 = -0.436;

/// Coarse step for walking along the sun's path; crossings are then bisected to 1 s.
const WALK_STEP_S: i64 = 60;
/// Never walk further than a day.
const MAX_WALK_STEPS: i64 = 24 * 60;
/// Sampling step when counting golden-hour seconds in view.
const GOLDEN_SAMPLE_S: i64 = 5;

/// When buildings hide (sunset) or reveal (sunrise) the sun at `lat`, `lon` on the local day
/// `date`, given the skyline `points` seen from there.
///
/// Times in the result are ISO 8601 with the fixed offset `utc_offset_s` (e.g. -25200 for PDT).
/// - Sunset: `hidden_at` is the last moment the sun's centre is above the skyline before the
///   flat sunset, or `None` if it is still in view at the flat sunset.
/// - Sunrise: `hidden_at` is the first moment it appears over the skyline after the flat
///   sunrise, or `None` if it is in view right at the flat sunrise.
///
/// If the sun is behind the skyline all the way to solar noon, `hidden_at` is solar noon
/// ("hidden since at least noon"). `golden_min_visible` is the minutes between the sun at +6°
/// and the flat sunset (sunrise: flat sunrise and +6°) with the sun in view. Near the poles, with no sunset that day, everything is
/// `None` and `golden_min_visible` is 0.
pub fn sun_block(
    points: &[HorizonPoint],
    lat: f64,
    lon: f64,
    date: NaiveDate,
    utc_offset_s: i32,
    event: Event,
) -> SunBlock {
    let offset = FixedOffset::east_opt(utc_offset_s).unwrap_or(FixedOffset::east_opt(0).unwrap());
    let empty = SunBlock {
        date: date.format("%Y-%m-%d").to_string(),
        flat_sunset: None,
        hidden_at: None,
        hidden_az_deg: None,
        hidden_elev_deg: None,
        golden_min_visible: 0,
    };
    let Ok(sunset) = sunset(lat, lon, date) else {
        return empty;
    };
    let sky = Skyline::new(points);
    let sun_at = |t: DateTime<Utc>| sun_position(lat, lon, t);
    let in_view = |t: DateTime<Utc>| !blocks_sun(&sky, sun_at(t));

    let noon = solar_noon_before(&sun_at, sunset.time);
    // `flat` is the flat-horizon event; `toward_noon` is the direction in time away from it
    // into the daytime (back for sunset, forward for sunrise).
    let (flat, toward_noon) = match event {
        Event::Sunset => (sunset.time, -1),
        Event::Sunrise => (flat_sunrise(&sun_at, noon), 1),
    };
    let step = Duration::seconds(WALK_STEP_S * toward_noon);

    let hidden_at = if in_view(flat) {
        None
    } else {
        // Walk into the day until the sun shows; the edge of that blocked stretch is the answer.
        let found = step_until(flat, step, in_view)
            .filter(|&(_, shown)| (shown - noon).num_seconds() * toward_noon <= 0);
        Some(match found {
            Some((blocked, shown)) => bisect(blocked, shown, in_view),
            None => noon,
        })
    };

    let (from, to) = golden_window_with_sun_up(&sun_at, flat, toward_noon);
    SunBlock {
        flat_sunset: Some(iso(flat, offset)),
        hidden_at: hidden_at.map(|t| iso(t, offset)),
        hidden_az_deg: hidden_at.map(|t| sun_at(t).azimuth_deg),
        hidden_elev_deg: hidden_at.map(|t| sun_at(t).elevation_deg),
        golden_min_visible: minutes_in_view(from, to, in_view),
        ..empty
    }
}

/// True when something on the skyline stands in front of the sun. The flat horizon itself
/// never blocks: a skyline at or below 0° only hides a sun that is already below the earth's
/// horizon, and that is the flat sunset, not a building.
fn blocks_sun(sky: &Skyline, sun: SunPosition) -> bool {
    sky.elev_at(sun.azimuth_deg) > sun.elevation_deg.max(0.0)
}

// ---- skyline lookup ---------------------------------------------------------------------

/// Horizon points sorted by azimuth, with the spacing that counts as "covered".
struct Skyline {
    /// (azimuth in 0..360, elevation), sorted by azimuth.
    points: Vec<(f64, f64)>,
    /// Two neighbours further apart than this leave a gap that is outside the profile.
    max_gap_deg: f64,
}

impl Skyline {
    fn new(points: &[HorizonPoint]) -> Self {
        let mut pts: Vec<(f64, f64)> =
            points.iter().map(|p| (p.az_deg.rem_euclid(360.0), p.elev_deg)).collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let step = typical_step_deg(&pts);
        Skyline { points: pts, max_gap_deg: step * 1.5 }
    }

    /// Skyline elevation at `az_deg`, linearly interpolated between the two neighbouring
    /// points (wrapping 359° -> 0°). Directions outside the profile count as flat (0°).
    fn elev_at(&self, az_deg: f64) -> f64 {
        let pts = &self.points;
        if pts.is_empty() {
            return 0.0;
        }
        let az = az_deg.rem_euclid(360.0);
        // First point at or after `az`; the one before it (wrapping) is at or before.
        let i = pts.partition_point(|p| p.0 < az);
        let after = pts[i % pts.len()];
        let before = pts[(i + pts.len() - 1) % pts.len()];
        if after.0 == az {
            return after.1;
        }
        let d_before = (az - before.0).rem_euclid(360.0);
        let d_after = (after.0 - az).rem_euclid(360.0);
        let gap = d_before + d_after;
        if pts.len() == 1 || gap > self.max_gap_deg {
            return 0.0;
        }
        before.1 + (after.1 - before.1) * d_before / gap
    }
}

/// The usual spacing between neighbouring azimuths (median of the gaps, not wrapping), so a
/// partial profile like 240°..320° is not stretched across the 320° -> 240° gap.
fn typical_step_deg(sorted: &[(f64, f64)]) -> f64 {
    let mut gaps: Vec<f64> =
        sorted.windows(2).map(|w| w[1].0 - w[0].0).filter(|g| *g > 0.0).collect();
    if gaps.is_empty() {
        return 0.0;
    }
    gaps.sort_by(f64::total_cmp);
    gaps[gaps.len() / 2]
}

// ---- walking along the sun's path -------------------------------------------------------

/// Steps from `start` by `step` until `hit(t)` is true. Returns the last instant where it was
/// false and the first where it is true, or `None` if that doesn't happen within a day.
fn step_until(
    start: DateTime<Utc>,
    step: Duration,
    hit: impl Fn(DateTime<Utc>) -> bool,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let mut prev = start;
    for _ in 0..MAX_WALK_STEPS {
        let next = prev + step;
        if hit(next) {
            return Some((prev, next));
        }
        prev = next;
    }
    None
}

/// Narrows down where `hit` switches from false (at `miss`) to true (at `hit_t`), to the
/// whole second. Works in either direction of time. Returns the first whole second on the
/// `hit` side (all our instants are whole seconds, so sub-second parts are dropped).
fn bisect(
    miss: DateTime<Utc>,
    hit_t: DateTime<Utc>,
    hit: impl Fn(DateTime<Utc>) -> bool,
) -> DateTime<Utc> {
    let at = |s: i64| DateTime::from_timestamp(s, 0).expect("timestamp in range");
    let (mut miss_s, mut hit_s) = (miss.timestamp(), hit_t.timestamp());
    while (hit_s - miss_s).abs() > 1 {
        let mid = miss_s + (hit_s - miss_s) / 2;
        if hit(at(mid)) {
            hit_s = mid;
        } else {
            miss_s = mid;
        }
    }
    at(hit_s)
}

/// When the sun's elevation, walking from `from` in steps of `step`, first reaches `target`.
/// If the sun turns around before reaching it (it never gets that high or that low today),
/// returns the turning point instead: as close as it gets.
fn elev_crossing(
    sun_at: &impl Fn(DateTime<Utc>) -> SunPosition,
    from: DateTime<Utc>,
    step: Duration,
    target: f64,
) -> DateTime<Utc> {
    let elev = |t: DateTime<Utc>| sun_at(t).elevation_deg;
    let rising = elev(from + step) > elev(from);
    let reached = |t: DateTime<Utc>| if rising { elev(t) >= target } else { elev(t) <= target };
    let turned = |t: DateTime<Utc>| (elev(t + step) > elev(t)) != rising;
    match step_until(from, step, |t| reached(t) || turned(t)) {
        Some((before, after)) if reached(after) => bisect(before, after, reached),
        Some((_, after)) => after,
        None => from,
    }
}

/// Solar noon (to the minute) on the same day, found by walking back from `sunset`.
fn solar_noon_before(
    sun_at: &impl Fn(DateTime<Utc>) -> SunPosition,
    sunset: DateTime<Utc>,
) -> DateTime<Utc> {
    // The sun never reaches 90° + 1, so this stops at the top of its path.
    elev_crossing(sun_at, sunset, Duration::seconds(-WALK_STEP_S), 91.0)
}

/// Flat-horizon sunrise on the same day, found by walking back from solar noon.
/// (Private until sun.rs grows a public sunrise; matches NOAA's to within seconds.)
fn flat_sunrise(
    sun_at: &impl Fn(DateTime<Utc>) -> SunPosition,
    noon: DateTime<Utc>,
) -> DateTime<Utc> {
    let down = |t: DateTime<Utc>| sun_at(t).elevation_deg < FLAT_HORIZON_APPARENT_ELEV_DEG;
    match step_until(noon, Duration::seconds(-WALK_STEP_S), down) {
        // Report the last instant at or above the horizon, i.e. the moment it comes up.
        Some((up, below)) => bisect(below, up, |t| !down(t)),
        None => noon,
    }
}

/// The part of the golden hour with the sun above a flat horizon: from the sun at +6° to the
/// flat event `flat` (sunrise: the other way round). The -4°..0° afterglow on the far side
/// of `flat` is left out, because there is no sun to see. `toward_noon` is -1 for sunset
/// and +1 for sunrise. Returns (earlier, later).
fn golden_window_with_sun_up(
    sun_at: &impl Fn(DateTime<Utc>) -> SunPosition,
    flat: DateTime<Utc>,
    toward_noon: i64,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let into_day = Duration::seconds(WALK_STEP_S * toward_noon);
    let high = elev_crossing(sun_at, flat, into_day, GOLDEN_HOUR_MAX_ELEV_DEG);
    if high < flat { (high, flat) } else { (flat, high) }
}

/// Whole minutes between `from` and `to` (from <= to) during which `in_view` holds,
/// sampled every few seconds.
fn minutes_in_view(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    in_view: impl Fn(DateTime<Utc>) -> bool,
) -> u32 {
    let samples = (to - from).num_seconds() / GOLDEN_SAMPLE_S;
    let seen = (0..samples)
        .filter(|&i| in_view(from + Duration::seconds(i * GOLDEN_SAMPLE_S + GOLDEN_SAMPLE_S / 2)))
        .count() as i64;
    ((seen * GOLDEN_SAMPLE_S) as f64 / 60.0).round() as u32
}

/// ISO 8601 with the given fixed offset, whole seconds: `2026-06-21T20:07:41-07:00`.
fn iso(t: DateTime<Utc>, offset: FixedOffset) -> String {
    t.with_timezone(&offset).format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LA: (f64, f64) = (34.0522, -118.2437);
    const PDT: i32 = -7 * 3600;

    fn solstice() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 21).unwrap()
    }

    fn parse(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// A full 360° profile, 1° steps, at `elev` inside `from..=to` (degrees) and 0 elsewhere.
    fn wall(from: f64, to: f64, elev: f64) -> Vec<HorizonPoint> {
        (0..360)
            .map(|a| {
                let az = a as f64;
                let inside = az >= from && az <= to;
                HorizonPoint {
                    az_deg: az,
                    elev_deg: if inside { elev } else { 0.0 },
                    dist_m: inside.then_some(40.0),
                }
            })
            .collect()
    }

    fn flat() -> Vec<HorizonPoint> {
        wall(0.0, -1.0, 0.0)
    }

    fn la_sunset_block(points: &[HorizonPoint]) -> SunBlock {
        sun_block(points, LA.0, LA.1, solstice(), PDT, Event::Sunset)
    }

    fn la_sun(t: DateTime<Utc>) -> SunPosition {
        sun_position(LA.0, LA.1, t)
    }

    /// Minutes of the evening golden hour with the sun up, found independently of
    /// `sun_block`: scan every second from 18:00 PDT to the flat sunset for elevation <= +6°.
    fn la_evening_golden_minutes_sun_up() -> f64 {
        let start = parse("2026-06-21T18:00:00-07:00");
        let sunset = parse("2026-06-21T20:07:41-07:00");
        let secs = (0..(sunset - start).num_seconds())
            .filter(|&s| la_sun(start + Duration::seconds(s)).elevation_deg <= GOLDEN_HOUR_MAX_ELEV_DEG)
            .count();
        secs as f64 / 60.0
    }

    #[test]
    fn flat_skyline_never_hides_the_sun() {
        let b = la_sunset_block(&flat());
        assert_eq!(b.date, "2026-06-21");
        assert_eq!(b.flat_sunset.as_deref(), Some("2026-06-21T20:07:41-07:00"));
        assert_eq!(b.hidden_at, None);
        assert_eq!(b.hidden_az_deg, None);
        assert_eq!(b.hidden_elev_deg, None);
        // +6° to flat sunset only; the ~20 min of afterglow after sunset doesn't count.
        let full = la_evening_golden_minutes_sun_up();
        assert!((33.0..40.0).contains(&full), "+6° to sunset is ~36 min in LA in June: {full}");
        assert!((b.golden_min_visible as f64 - full).abs() <= 1.0, "{} vs {full}", b.golden_min_visible);
    }

    #[test]
    fn empty_profile_is_flat() {
        assert_eq!(la_sunset_block(&[]), la_sunset_block(&flat()));
    }

    #[test]
    fn wall_across_sunset_hides_the_sun_when_it_drops_to_10_degrees() {
        // The sun sets around az 299° and is near 290° at 10° up; the wall covers both.
        let b = la_sunset_block(&wall(270.0, 320.0, 10.0));
        let hidden = parse(b.hidden_at.as_deref().expect("wall should hide the sun"));
        let elev = la_sun(hidden).elevation_deg;
        // Near 10° the sun drops ~0.15°/min, so 0.01° is a few seconds.
        assert!((elev - 10.0).abs() < 0.01, "sun elevation at hidden_at: {elev}");
        assert!(la_sun(hidden + Duration::seconds(5)).elevation_deg < 10.0);
        assert!((b.hidden_elev_deg.unwrap() - elev).abs() < 1e-9);
        assert!((280.0..300.0).contains(&b.hidden_az_deg.unwrap()));
        // About 19:00 PDT, an hour before flat sunset; golden hour (sun below 6°) is all behind the wall.
        assert!(b.hidden_at.as_deref().unwrap().starts_with("2026-06-21T19:"), "{:?}", b.hidden_at);
        assert_eq!(b.golden_min_visible, 0);
    }

    #[test]
    fn wall_in_another_direction_changes_nothing() {
        let behind_you = wall(60.0, 120.0, 30.0);
        assert_eq!(la_sunset_block(&behind_you), la_sunset_block(&flat()));
    }

    #[test]
    fn low_wall_hides_only_part_of_golden_hour() {
        // A 3° wall: the sun goes behind it partway through golden hour.
        let b = la_sunset_block(&wall(270.0, 320.0, 3.0));
        let hidden = parse(b.hidden_at.as_deref().unwrap());
        assert!((la_sun(hidden).elevation_deg - 3.0).abs() < 0.01);
        let full = la_evening_golden_minutes_sun_up().round() as u32;
        assert!(b.golden_min_visible > 5 && b.golden_min_visible < full, "{}", b.golden_min_visible);
    }

    #[test]
    fn negative_skyline_from_a_roof_does_not_block() {
        // On a tall roof everything around is below eye level.
        let roof: Vec<HorizonPoint> = wall(0.0, 359.0, -2.0);
        assert_eq!(la_sunset_block(&roof).hidden_at, None);
        assert_eq!(la_sunset_block(&roof).golden_min_visible, la_sunset_block(&flat()).golden_min_visible);
    }

    #[test]
    fn sunrise_behind_a_wall_appears_when_it_climbs_past_10_degrees() {
        // LA solstice sunrise is ~05:42 PDT at az ~61°; the sun climbs through 10° near az 70°.
        let points = wall(40.0, 100.0, 10.0);
        let b = sun_block(&points, LA.0, LA.1, solstice(), PDT, Event::Sunrise);
        let rise = parse(b.flat_sunset.as_deref().unwrap());
        let noaa_rise = parse("2026-06-21T05:42:12-07:00"); // contracts.md SunPath example
        assert!((rise - noaa_rise).num_seconds().abs() <= 30, "flat sunrise {rise}");
        let shown = parse(b.hidden_at.as_deref().expect("wall should delay the sunrise"));
        assert!(shown > rise);
        let elev = la_sun(shown).elevation_deg;
        assert!((elev - 10.0).abs() < 0.01, "sun elevation when it appears: {elev}");
        assert!(la_sun(shown - Duration::seconds(5)).elevation_deg < 10.0);
        assert_eq!(b.golden_min_visible, 0);
    }

    #[test]
    fn sunrise_over_flat_skyline_is_in_view_at_once() {
        let b = sun_block(&flat(), LA.0, LA.1, solstice(), PDT, Event::Sunrise);
        assert_eq!(b.hidden_at, None);
        assert!((33..40).contains(&b.golden_min_visible), "{}", b.golden_min_visible);
    }

    #[test]
    fn sun_hidden_all_afternoon_reports_solar_noon() {
        let b = la_sunset_block(&wall(0.0, 359.0, 89.0));
        // LA solar noon on the solstice is ~12:57 PDT.
        assert!(b.hidden_at.as_deref().unwrap().starts_with("2026-06-21T12:5"), "{:?}", b.hidden_at);
    }

    #[test]
    fn polar_night_gives_an_empty_block() {
        let dec = NaiveDate::from_ymd_opt(2026, 12, 21).unwrap();
        let b = sun_block(&flat(), 69.6492, 18.9553, dec, 3600, Event::Sunset);
        assert_eq!((b.flat_sunset, b.hidden_at, b.golden_min_visible), (None, None, 0));
    }

    #[test]
    fn skyline_interpolates_between_neighbours() {
        let sky = Skyline::new(&wall(10.0, 10.0, 4.0));
        assert_eq!(sky.elev_at(10.0), 4.0);
        assert!((sky.elev_at(10.25) - 3.0).abs() < 1e-9);
        assert!((sky.elev_at(9.5) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn skyline_wraps_from_359_to_0() {
        let mut pts = flat();
        pts[359].elev_deg = 2.0;
        pts[0].elev_deg = 4.0;
        let sky = Skyline::new(&pts);
        assert!((sky.elev_at(359.5) - 3.0).abs() < 1e-9);
        assert!((sky.elev_at(-0.5) - 3.0).abs() < 1e-9);
        assert!((sky.elev_at(360.0) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn directions_outside_a_partial_profile_are_flat() {
        let pts: Vec<HorizonPoint> = (240..=320)
            .map(|a| HorizonPoint { az_deg: a as f64, elev_deg: 5.0, dist_m: Some(50.0) })
            .collect();
        let sky = Skyline::new(&pts);
        assert_eq!(sky.elev_at(280.5), 5.0);
        assert_eq!(sky.elev_at(330.0), 0.0);
        assert_eq!(sky.elev_at(10.0), 0.0);
    }

    #[test]
    fn bisect_finds_the_switch_to_the_second_both_ways() {
        let t0 = parse("2026-06-21T12:00:00Z");
        let switch = t0 + Duration::seconds(37);
        let after = |t: DateTime<Utc>| t >= switch;
        assert_eq!(bisect(t0, t0 + Duration::seconds(60), after), switch);
        let before = |t: DateTime<Utc>| t <= switch;
        assert_eq!(bisect(t0 + Duration::seconds(60), t0, before), switch);
    }
}
