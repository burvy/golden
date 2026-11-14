//! `GET /api/sun`: the real `SunPath` (docs/contracts.md#sunpath) for one local day.
//!
//! Built on `golden_core::sun::sun_position` (NOAA, refraction included). Every event is a moment the
//! sun's apparent elevation crosses a level: sunrise/sunset at -0.436° (top edge on a flat horizon, the
//! same convention as core's `sunset` and `sunblock`), golden hour at -4° and +6°. We sample the local day
//! every 5 minutes, then bisect each crossing to the second.

use axum::{extract::rejection::QueryRejection, extract::Query, response::IntoResponse, response::Response, Json};
use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use contracts::sun::{GoldenHour, SunEvent, SunPath, SunPoint, TimeRange, GOLDEN_HOUR_MAX_ELEV_DEG, GOLDEN_HOUR_MIN_ELEV_DEG};
use contracts::search::DEFAULT_TZ;
use golden_core::sun::sun_position;
use serde::Deserialize;

use crate::api::{check_lat_lon, check_range, parse_date, parse_tz, round2, ApiErr, ApiResult};
use crate::stubs;

/// Apparent elevation of the sun's centre when its top edge touches a flat horizon (NOAA's -0.833°
/// geometric, minus NOAA's refraction there). Matches core's `sunset` to the second.
const RISE_SET_ELEV_DEG: f64 = -0.436;
const SAMPLE_MIN: i64 = 5;
/// `points` start this long before sunrise and end this long after sunset.
const MARGIN_MIN: i64 = 30;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SunParams {
    lat: f64,
    lon: f64,
    date: String,
    #[serde(default = "default_tz")]
    tz: String,
    #[serde(default = "default_step")]
    step_min: u32,
}

fn default_tz() -> String {
    DEFAULT_TZ.to_string()
}
fn default_step() -> u32 {
    10
}

pub async fn get_sun(q: Result<Query<SunParams>, QueryRejection>) -> ApiResult<Response> {
    let Query(q) = q?;
    check_lat_lon(q.lat, q.lon)?;
    let date = parse_date(&q.date)?;
    let tz = parse_tz(&q.tz)?;
    check_range("step_min", q.step_min as f64, 1.0, 60.0)?;
    if stubs::on() {
        return Ok(stubs::SUN_PATH.into_response());
    }
    match sun_path(q.lat, q.lon, date, tz, q.step_min) {
        Some(p) => Ok(Json(p).into_response()),
        None if noon_elev(q.lat, q.lon, date, tz) > 0.0 => Err(ApiErr::impossible("no sunrise or sunset: the sun stays up all day")),
        None => Err(ApiErr::impossible("no sunrise or sunset: the sun never rises")),
    }
}

/// The sun over the local day `date` in `tz`. `None` if it neither rises nor sets that day (polar day or
/// night). On a day with only one of the two, the other is `null` and `points` run to the day's edge.
pub fn sun_path(lat: f64, lon: f64, date: NaiveDate, tz: Tz, step_min: u32) -> Option<SunPath> {
    let (start, end) = local_day(tz, date);
    let elev = |t: DateTime<Utc>| sun_position(lat, lon, t).elevation_deg;
    let az = |t: DateTime<Utc>| sun_position(lat, lon, t).azimuth_deg;

    let rise_set = crossings(&elev, start, end, RISE_SET_ELEV_DEG);
    let sunrise = rise_set.iter().find(|c| c.rising).map(|c| c.t);
    let sunset = rise_set.iter().find(|c| !c.rising).map(|c| c.t);
    if sunrise.is_none() && sunset.is_none() {
        return None;
    }

    let low = crossings(&elev, start, end, GOLDEN_HOUR_MIN_ELEV_DEG);
    let high = crossings(&elev, start, end, GOLDEN_HOUR_MAX_ELEV_DEG);
    let first = |cs: &[Crossing], rising: bool| cs.iter().find(|c| c.rising == rising).map(|c| c.t);
    let range = |from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>| match (from, to) {
        (Some(from), Some(to)) if from < to => Some(TimeRange { from: iso(from, tz), to: iso(to, tz) }),
        _ => None,
    };
    let golden_hour = GoldenHour {
        morning: range(first(&low, true), first(&high, true)),
        evening: range(first(&high, false), first(&low, false)),
    };

    let margin = Duration::minutes(MARGIN_MIN);
    let from = sunrise.map_or(start, |t| (t - margin).max(start));
    let to = sunset.map_or(end, |t| (t + margin).min(end));
    let step = Duration::minutes(step_min as i64);
    let points = std::iter::successors(Some(start), |t| Some(*t + step))
        .take_while(|t| *t <= to)
        .filter(|t| *t >= from)
        .map(|t| SunPoint { t: iso(t, tz), az_deg: round2(az(t)), elev_deg: round2(elev(t)) })
        .collect();

    let event = |t: DateTime<Utc>| SunEvent { t: iso(t, tz), az_deg: round2(az(t)) };
    Some(SunPath {
        lat,
        lon,
        date: date.format("%Y-%m-%d").to_string(),
        tz: tz.name().to_string(),
        sunrise: sunrise.map(event),
        sunset: sunset.map(event),
        golden_hour,
        points,
    })
}

#[derive(Debug, Clone, Copy)]
struct Crossing {
    t: DateTime<Utc>,
    /// The sun is going up through the level.
    rising: bool,
}

/// Every time in `[start, end)` that `elev` crosses `level`, to the second.
fn crossings(elev: &impl Fn(DateTime<Utc>) -> f64, start: DateTime<Utc>, end: DateTime<Utc>, level: f64) -> Vec<Crossing> {
    let step = Duration::minutes(SAMPLE_MIN);
    let mut out = Vec::new();
    let mut a = start;
    let mut above = elev(a) > level;
    while a < end {
        let b = (a + step).min(end);
        let b_above = elev(b) > level;
        if b_above != above {
            out.push(Crossing { t: bisect(elev, a, b, level, b_above), rising: b_above });
        }
        (a, above) = (b, b_above);
    }
    out
}

/// The moment in `(lo, hi]` where `elev` crosses `level`, rounded to the nearest second.
fn bisect(elev: &impl Fn(DateTime<Utc>) -> f64, mut lo: DateTime<Utc>, mut hi: DateTime<Utc>, level: f64, rising: bool) -> DateTime<Utc> {
    while hi - lo > Duration::milliseconds(100) {
        let mid = lo + (hi - lo) / 2;
        if (elev(mid) > level) == rising {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let mid = lo + (hi - lo) / 2;
    DateTime::from_timestamp((mid.timestamp_millis() as f64 / 1000.0).round() as i64, 0).unwrap_or(mid)
}

/// Local midnight to the next local midnight, in UTC.
fn local_day(tz: Tz, date: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    let midnight = |d: NaiveDate| {
        let naive = d.and_hms_opt(0, 0, 0).expect("midnight exists");
        tz.from_local_datetime(&naive).earliest().map_or_else(|| Utc.from_utc_datetime(&naive), |t| t.with_timezone(&Utc))
    };
    (midnight(date), midnight(date.succ_opt().unwrap_or(date)))
}

fn noon_elev(lat: f64, lon: f64, date: NaiveDate, tz: Tz) -> f64 {
    let (start, end) = local_day(tz, date);
    sun_position(lat, lon, start + (end - start) / 2).elevation_deg
}

/// ISO 8601 with the place's local offset, e.g. `2026-06-21T20:07:41-07:00`.
pub fn iso(t: DateTime<Utc>, tz: Tz) -> String {
    t.with_timezone(&tz).to_rfc3339_opts(SecondsFormat::Secs, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn la() -> Tz {
        "America/Los_Angeles".parse().unwrap()
    }

    fn secs(iso: &str) -> i64 {
        DateTime::parse_from_rfc3339(iso).unwrap().timestamp()
    }

    /// The contract fixture's times are NOAA's for downtown LA.
    #[test]
    fn la_solstice_matches_noaa() {
        let june21 = NaiveDate::from_ymd_opt(2026, 6, 21).unwrap();
        let p = sun_path(34.0522, -118.2437, june21, la(), 10).unwrap();
        let rise = p.sunrise.as_ref().unwrap();
        let set = p.sunset.as_ref().unwrap();
        // NOAA's sunrise formula lands 8 s after the moment its own sun position crosses the horizon
        // (05:42:04); we use the crossing, like core's sunblock does for sunrise searches.
        assert!((secs(&rise.t) - secs("2026-06-21T05:42:12-07:00")).abs() <= 10, "{}", rise.t);
        assert!((secs(&set.t) - secs("2026-06-21T20:07:41-07:00")).abs() <= 2, "{}", set.t);
        assert!(set.t.ends_with("-07:00"));
        assert!((rise.az_deg - 60.67).abs() < 0.05 && (set.az_deg - 299.34).abs() < 0.05);

        // and the same second as core's sunset (which /api/horizon and search use)
        let core = golden_core::sun::sunset(34.0522, -118.2437, june21).unwrap();
        assert!((secs(&set.t) - core.time.timestamp()).abs() <= 1);

        let eve = p.golden_hour.evening.as_ref().unwrap();
        assert!(secs(&eve.from) < secs(&set.t) && secs(&set.t) < secs(&eve.to));
        // +6° at 19:31:27, -4° at 20:25:54 (the fixture's 19:19:40-20:24:20 is made up)
        assert!((secs(&eve.from) - secs("2026-06-21T19:31:27-07:00")).abs() <= 2, "{eve:?}");
        assert!((secs(&eve.to) - secs("2026-06-21T20:25:54-07:00")).abs() <= 2, "{eve:?}");
        assert!(p.golden_hour.morning.is_some());
    }

    #[test]
    fn points_cover_sunrise_to_sunset_on_the_clock() {
        let p = sun_path(34.0522, -118.2437, NaiveDate::from_ymd_opt(2026, 6, 21).unwrap(), la(), 10).unwrap();
        assert_eq!(p.points.first().unwrap().t, "2026-06-21T05:20:00-07:00"); // sunrise 05:42 - 30 min, on a 10 min mark
        assert_eq!(p.points.last().unwrap().t, "2026-06-21T20:30:00-07:00");
        assert!(p.points.iter().any(|pt| pt.elev_deg > 75.0)); // ~79° at solar noon
    }

    #[test]
    fn winter_uses_standard_time() {
        let p = sun_path(34.0522, -118.2437, NaiveDate::from_ymd_opt(2026, 12, 21).unwrap(), la(), 30).unwrap();
        assert!(p.sunset.unwrap().t.starts_with("2026-12-21T16:"), "LA winter sunset is ~16:48 PST");
    }

    #[test]
    fn polar_days_have_no_path() {
        let tromso: Tz = "Europe/Oslo".parse().unwrap();
        assert!(sun_path(69.65, 18.96, NaiveDate::from_ymd_opt(2026, 6, 21).unwrap(), tromso, 10).is_none());
        assert!(sun_path(69.65, 18.96, NaiveDate::from_ymd_opt(2026, 12, 21).unwrap(), tromso, 10).is_none());
    }
}
