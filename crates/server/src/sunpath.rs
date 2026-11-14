//! `SunPath` (docs/contracts.md#sunpath): sunrise, sunset, golden hours and the sun's track over one local
//! day, over a flat horizon. Pure (no clock, no IO); built on `core::sun`, so its sunset matches search's.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use contracts::sun::{GoldenHour, SunEvent, SunPath, SunPoint, TimeRange, GOLDEN_HOUR_MAX_ELEV_DEG};
use golden_core::sun::{sun_position, sunset};

use crate::tz;

/// Lower edge of golden hour (contract: elevation between −4° and +6°).
pub const GOLDEN_HOUR_MIN_ELEV_DEG: f64 = -4.0;

pub fn sun_path(lat: f64, lon: f64, date: NaiveDate, zone: Tz, step_min: u32) -> SunPath {
    let start = tz::day_start(zone, date);
    let end = tz::day_start(zone, date + Duration::days(1));
    let el = |t: DateTime<Utc>| sun_position(lat, lon, t).elevation_deg;
    let az = |t: DateTime<Utc>| sun_position(lat, lon, t).azimuth_deg;

    // "Sun on the horizon" = the elevation core reports at its own sunset, so both always agree.
    let horizon_el = sunset(lat, lon, date).map(|s| el(s.time)).unwrap_or(-0.833);
    let noon = solar_noon(start, end, &el);

    let rise = crossing(start, noon, horizon_el, true, &el);
    let set = crossing(noon, end, horizon_el, false, &el);
    let event = |t: DateTime<Utc>| SunEvent { t: tz::iso(zone, t), az_deg: round2(az(t)) };

    let morning = band(start, noon, true, &el).map(|(a, b)| range(zone, a, b));
    let evening = band(noon, end, false, &el).map(|(a, b)| range(zone, a, b));

    let step = Duration::minutes(i64::from(step_min.clamp(1, 60)));
    let from = rise.map(|t| t - Duration::minutes(30)).unwrap_or(start).max(start);
    let to = set.map(|t| t + Duration::minutes(30)).unwrap_or(end).min(end);
    let mut points = Vec::new();
    let mut t = floor_to(from, step, start);
    while t <= to {
        if t >= from {
            points.push(SunPoint { t: tz::iso(zone, t), az_deg: round2(az(t)), elev_deg: round2(el(t)) });
        }
        t += step;
    }

    SunPath {
        lat,
        lon,
        date: date.format("%Y-%m-%d").to_string(),
        tz: zone.name().to_string(),
        sunrise: rise.map(event),
        sunset: set.map(event),
        golden_hour: GoldenHour { morning, evening },
        points,
    }
}

fn range(zone: Tz, a: DateTime<Utc>, b: DateTime<Utc>) -> TimeRange {
    TimeRange { from: tz::iso(zone, a), to: tz::iso(zone, b) }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Highest sun of the day, to the minute.
fn solar_noon(start: DateTime<Utc>, end: DateTime<Utc>, el: &impl Fn(DateTime<Utc>) -> f64) -> DateTime<Utc> {
    let mut best = (start, f64::MIN);
    let mut t = start;
    while t < end {
        let e = el(t);
        if e > best.1 {
            best = (t, e);
        }
        t += Duration::minutes(1);
    }
    best.0
}

/// First moment in [a, b] where the elevation crosses `level` going up (`rising`) or down, to the second.
fn crossing(a: DateTime<Utc>, b: DateTime<Utc>, level: f64, rising: bool, el: &impl Fn(DateTime<Utc>) -> f64) -> Option<DateTime<Utc>> {
    let above = |t| el(t) >= level;
    let mut t = a;
    while t < b {
        let n = (t + Duration::minutes(1)).min(b);
        if above(t) != rising && above(n) == rising {
            return Some(bisect(t, n, |x| above(x) == rising));
        }
        t = n;
    }
    None
}

/// Smallest time in (lo, hi] where `f` is true, given f(lo) false and f(hi) true. One-second precision.
fn bisect(mut lo: DateTime<Utc>, mut hi: DateTime<Utc>, f: impl Fn(DateTime<Utc>) -> bool) -> DateTime<Utc> {
    while hi - lo > Duration::seconds(1) {
        let mid = lo + (hi - lo) / 2;
        if f(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Golden hour on one side of noon: while −4° ≤ elevation ≤ +6°. Morning runs up from −4° to +6°;
/// evening runs down from +6° to −4°. Clipped to [a, b] (noon) when the sun never gets past one edge.
fn band(a: DateTime<Utc>, b: DateTime<Utc>, morning: bool, el: &impl Fn(DateTime<Utc>) -> f64) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let (lo, hi) = (GOLDEN_HOUR_MIN_ELEV_DEG, GOLDEN_HOUR_MAX_ELEV_DEG);
    let in_band = |t| (lo..=hi).contains(&el(t));
    if morning {
        let from = crossing(a, b, lo, true, el).or_else(|| in_band(a).then_some(a))?;
        let to = crossing(from, b, hi, true, el).unwrap_or(b);
        Some((from, to))
    } else {
        let from = crossing(a, b, hi, false, el).or_else(|| in_band(a).then_some(a))?;
        let to = crossing(from, b, lo, false, el).unwrap_or(b);
        Some((from, to))
    }
}

/// Round `t` down onto the `step` grid that starts at `origin` (local midnight), so points land on :00, :10...
fn floor_to(t: DateTime<Utc>, step: Duration, origin: DateTime<Utc>) -> DateTime<Utc> {
    let s = step.num_seconds().max(1);
    let k = (t - origin).num_seconds().div_euclid(s);
    origin + Duration::seconds(k * s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn la_solstice() -> SunPath {
        sun_path(34.0522, -118.2437, NaiveDate::from_ymd_opt(2026, 6, 21).unwrap(), tz::parse("America/Los_Angeles").unwrap(), 10)
    }

    fn minutes(iso: &str) -> i64 {
        let t = DateTime::parse_from_rfc3339(iso).unwrap();
        i64::from(t.format("%H").to_string().parse::<u32>().unwrap()) * 60 + i64::from(t.format("%M").to_string().parse::<u32>().unwrap())
    }

    #[test]
    fn la_solstice_matches_noaa() {
        let p = la_solstice();
        // NOAA: sunrise 05:42, sunset 20:07 PDT, sunset azimuth ~299.3°
        let rise = p.sunrise.unwrap();
        let set = p.sunset.unwrap();
        assert!((minutes(&rise.t) - (5 * 60 + 42)).abs() <= 1, "{}", rise.t);
        assert!((minutes(&set.t) - (20 * 60 + 7)).abs() <= 1, "{}", set.t);
        assert!(set.t.ends_with("-07:00"));
        assert!((set.az_deg - 299.3).abs() < 0.3);
    }

    #[test]
    fn golden_hours_bracket_sunrise_and_sunset() {
        let p = la_solstice();
        let ev = p.golden_hour.evening.unwrap();
        let mo = p.golden_hour.morning.unwrap();
        let set = minutes(&p.sunset.unwrap().t);
        assert!(minutes(&ev.from) < set && set < minutes(&ev.to));
        assert!(minutes(&mo.from) < minutes(&mo.to));
        // roughly an hour each side, as in the fixture (19:19 to 20:24)
        // the sun drops ~0.18°/min near an LA June sunset: 6° to -4° takes under an hour (19:31 -> 20:26)
        assert!((minutes(&ev.to) - minutes(&ev.from) - 54).abs() <= 3, "{} -> {}", ev.from, ev.to);
        assert!((minutes(&ev.from) - (19 * 60 + 31)).abs() <= 1);
    }

    #[test]
    fn points_cover_half_an_hour_either_side_on_the_step_grid() {
        let p = la_solstice();
        let first = minutes(&p.points[0].t);
        assert_eq!(first % 10, 0);
        assert!(first <= minutes(&p.sunrise.unwrap().t) - 30 + 10);
        assert!(p.points.windows(2).all(|w| minutes(&w[1].t) - minutes(&w[0].t) == 10));
    }

    #[test]
    fn polar_night_has_no_events() {
        let p = sun_path(78.22, 15.65, NaiveDate::from_ymd_opt(2026, 12, 21).unwrap(), tz::parse("Arctic/Longyearbyen").unwrap(), 30);
        assert!(p.sunrise.is_none() && p.sunset.is_none());
        assert!(!p.points.is_empty());
    }

    /// Prints the LA solstice path, used to write fixtures/sun_path.json. `cargo test -p server print_fixture -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_fixture() {
        println!("{}", serde_json::to_string_pretty(&la_solstice()).unwrap());
    }
}
