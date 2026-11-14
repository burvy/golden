//! Sun position and sunset, ported from the NOAA Solar Calculator
//! (https://gml.noaa.gov/grad/solcalc/, `main.js`). Same formulas, same constants,
//! so our numbers match NOAA's to the second.
//!
//! Conventions (see DESIGN.md): degrees everywhere, azimuth clockwise from true north,
//! longitude positive east, times in UTC. The caller passes the date in; nothing here
//! reads a clock.

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};

/// When the sun sets and where on the horizon it goes down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sunset {
    /// Moment the sun's upper edge touches a flat, sea-level horizon (with refraction).
    pub time: DateTime<Utc>,
    /// Direction of the sun at that moment, degrees clockwise from true north (west is 270).
    pub azimuth_deg: f64,
}

/// Why there is no sunset on a given day (only happens near the poles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoSunset {
    /// The sun stays up all day (midnight sun).
    PolarDay,
    /// The sun never comes up (polar night).
    PolarNight,
}

/// Where the sun is in the sky, as seen from a point on the ground.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunPosition {
    /// Degrees clockwise from true north.
    pub azimuth_deg: f64,
    /// Degrees above the horizon, corrected for atmospheric refraction (negative = below).
    pub elevation_deg: f64,
}

/// Sunset on `date` (the local calendar day at that place) at `lat`, `lon`.
///
/// The returned time is UTC, so for western longitudes it often falls on the next
/// UTC day (LA's 20:07 PDT sunset on June 21 is 03:07Z on June 22).
pub fn sunset(lat: f64, lon: f64, date: NaiveDate) -> Result<Sunset, NoSunset> {
    let jd = julian_day(date);
    // NOAA does two passes: estimate, then recompute the sun's state at the estimate.
    let first = sunset_utc_minutes(jd, lat, lon)?;
    let minutes = sunset_utc_minutes(jd + first / 1440.0, lat, lon)?;

    let midnight = date.and_hms_opt(0, 0, 0).expect("midnight exists").and_utc();
    let time = midnight + Duration::seconds((minutes * 60.0).round() as i64);
    let azimuth_deg = sun_position(lat, lon, time).azimuth_deg;
    Ok(Sunset { time, azimuth_deg })
}

/// Sun direction and elevation at instant `t` seen from `lat`, `lon` (NOAA `calcAzEl`).
pub fn sun_position(lat: f64, lon: f64, t: DateTime<Utc>) -> SunPosition {
    let jd = julian_day(t.date_naive()) + minutes_since_midnight(t) / 1440.0;
    let jc = julian_century(jd);
    let eq_time = equation_of_time(jc);
    let decl = sun_declination(jc);

    // Solar time of day in minutes, wrapped to 0..1440; hour angle is 0 at solar noon.
    let true_solar_time = (minutes_since_midnight(t) + eq_time + 4.0 * lon).rem_euclid(1440.0);
    let hour_angle = true_solar_time / 4.0 - 180.0;

    let (lat_r, decl_r) = (lat.to_radians(), decl.to_radians());
    let cos_zenith = (lat_r.sin() * decl_r.sin()
        + lat_r.cos() * decl_r.cos() * hour_angle.to_radians().cos())
    .clamp(-1.0, 1.0);
    let zenith = cos_zenith.acos().to_degrees();

    let az_denom = lat_r.cos() * zenith.to_radians().sin();
    let mut azimuth = if az_denom.abs() > 0.001 {
        let cos_az = ((lat_r.sin() * zenith.to_radians().cos()) - decl_r.sin()) / az_denom;
        let az = 180.0 - cos_az.clamp(-1.0, 1.0).acos().to_degrees();
        if hour_angle > 0.0 { -az } else { az }
    } else if lat > 0.0 {
        180.0
    } else {
        0.0
    };
    if azimuth < 0.0 {
        azimuth += 360.0;
    }

    let geometric_elevation = 90.0 - zenith;
    SunPosition {
        azimuth_deg: azimuth,
        elevation_deg: geometric_elevation + refraction_deg(geometric_elevation),
    }
}

// ---- NOAA building blocks (names follow main.js) -------------------------------------

/// UTC minutes after midnight of day `jd` at which the sun sets (NOAA `calcSunriseSetUTC`).
fn sunset_utc_minutes(jd: f64, lat: f64, lon: f64) -> Result<f64, NoSunset> {
    let jc = julian_century(jd);
    let eq_time = equation_of_time(jc);
    let hour_angle = sunrise_hour_angle_deg(lat, sun_declination(jc))?;
    Ok(720.0 - 4.0 * (lon - hour_angle) - eq_time)
}

/// Hour angle of sunrise in degrees; sunset is the same angle after solar noon.
/// 90.833° = 90° + 34' refraction + 16' solar radius.
fn sunrise_hour_angle_deg(lat: f64, decl: f64) -> Result<f64, NoSunset> {
    let (lat_r, decl_r) = (lat.to_radians(), decl.to_radians());
    let cos_ha = 90.833_f64.to_radians().cos() / (lat_r.cos() * decl_r.cos())
        - lat_r.tan() * decl_r.tan();
    if cos_ha < -1.0 {
        Err(NoSunset::PolarDay)
    } else if cos_ha > 1.0 {
        Err(NoSunset::PolarNight)
    } else {
        Ok(cos_ha.acos().to_degrees())
    }
}

/// Julian day at 00:00 UTC of `date` (NOAA `getJD`, Gregorian calendar).
fn julian_day(date: NaiveDate) -> f64 {
    let (mut year, mut month) = (date.year() as f64, date.month() as f64);
    if month <= 2.0 {
        year -= 1.0;
        month += 12.0;
    }
    let a = (year / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (year + 4716.0)).floor() + (30.6001 * (month + 1.0)).floor() + date.day() as f64 + b
        - 1524.5
}

fn minutes_since_midnight(t: DateTime<Utc>) -> f64 {
    let since = t - t.date_naive().and_hms_opt(0, 0, 0).expect("midnight exists").and_utc();
    since.num_milliseconds() as f64 / 60_000.0
}

fn julian_century(jd: f64) -> f64 {
    (jd - 2451545.0) / 36525.0
}

fn geom_mean_long_sun(t: f64) -> f64 {
    (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0)
}

fn geom_mean_anomaly_sun(t: f64) -> f64 {
    357.52911 + t * (35999.05029 - 0.0001537 * t)
}

fn eccentricity_earth_orbit(t: f64) -> f64 {
    0.016708634 - t * (0.000042037 + 0.0000001267 * t)
}

fn sun_eq_of_center(t: f64) -> f64 {
    let m = geom_mean_anomaly_sun(t).to_radians();
    m.sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + (2.0 * m).sin() * (0.019993 - 0.000101 * t)
        + (3.0 * m).sin() * 0.000289
}

fn sun_apparent_long(t: f64) -> f64 {
    let true_long = geom_mean_long_sun(t) + sun_eq_of_center(t);
    let omega = 125.04 - 1934.136 * t;
    true_long - 0.00569 - 0.00478 * omega.to_radians().sin()
}

fn obliquity_correction(t: f64) -> f64 {
    let seconds = 21.448 - t * (46.8150 + t * (0.00059 - t * 0.001813));
    let mean_obliquity = 23.0 + (26.0 + seconds / 60.0) / 60.0;
    let omega = 125.04 - 1934.136 * t;
    mean_obliquity + 0.00256 * omega.to_radians().cos()
}

fn sun_declination(t: f64) -> f64 {
    let e = obliquity_correction(t).to_radians();
    let lambda = sun_apparent_long(t).to_radians();
    (e.sin() * lambda.sin()).asin().to_degrees()
}

/// Equation of time in minutes (solar time minus mean time).
fn equation_of_time(t: f64) -> f64 {
    let epsilon = obliquity_correction(t).to_radians();
    let l0 = geom_mean_long_sun(t).to_radians();
    let e = eccentricity_earth_orbit(t);
    let m = geom_mean_anomaly_sun(t).to_radians();
    let y = (epsilon / 2.0).tan().powi(2);

    let etime = y * (2.0 * l0).sin() - 2.0 * e * m.sin()
        + 4.0 * e * y * m.sin() * (2.0 * l0).cos()
        - 0.5 * y * y * (4.0 * l0).sin()
        - 1.25 * e * e * (2.0 * m).sin();
    etime.to_degrees() * 4.0
}

/// Atmospheric refraction in degrees for a geometric elevation (NOAA `calcRefraction`).
fn refraction_deg(elev: f64) -> f64 {
    if elev > 85.0 {
        return 0.0;
    }
    let te = elev.to_radians().tan();
    let arcsec = if elev > 5.0 {
        58.1 / te - 0.07 / te.powi(3) + 0.000086 / te.powi(5)
    } else if elev > -0.575 {
        1735.0 + elev * (-518.2 + elev * (103.4 + elev * (-12.79 + elev * 0.711)))
    } else {
        -20.774 / te
    };
    arcsec / 3600.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Reference values produced by running NOAA's own solar calculator code
    /// (https://gml.noaa.gov/grad/solcalc/main.js, `calcSunriseSetUTC` x2 + `calcSunriseSet`)
    /// in Node. See docs/ray/DEVLOG.md for how to regenerate them.
    /// (name, lat, lon, date, expected UTC sunset, expected azimuth)
    const NOAA_CASES: &[(&str, f64, f64, (i32, u32, u32), &str, f64)] = &[
        ("LA summer solstice", 34.0522, -118.2437, (2026, 6, 21), "2026-06-22T03:07:41Z", 299.3350),
        ("LA winter solstice", 34.0522, -118.2437, (2026, 12, 21), "2026-12-22T00:47:44Z", 241.9465),
        ("LA March equinox", 34.0522, -118.2437, (2026, 3, 20), "2026-03-21T02:04:46Z", 270.7906),
        ("Houston July 4", 29.7604, -95.3698, (2026, 7, 4), "2026-07-05T01:25:57Z", 297.0435),
        ("Greenwich solstice", 51.4769, -0.0005, (2026, 6, 21), "2026-06-21T20:20:54Z", 311.0669),
        ("Sydney June", -33.8688, 151.2093, (2026, 6, 21), "2026-06-21T06:53:50Z", 297.9912),
    ];

    #[test]
    fn sunset_matches_noaa_calculator() {
        for &(name, lat, lon, (y, m, d), want_time, want_az) in NOAA_CASES {
            let got = sunset(lat, lon, date(y, m, d)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let want: DateTime<Utc> = want_time.parse().unwrap();
            let off_s = (got.time - want).num_seconds().abs();
            assert!(off_s <= 1, "{name}: time {} vs NOAA {want} ({off_s}s off)", got.time);
            let off_az = (got.azimuth_deg - want_az).abs();
            assert!(off_az < 0.01, "{name}: azimuth {} vs NOAA {want_az}", got.azimuth_deg);
        }
    }

    #[test]
    fn la_solstice_sunset_is_just_after_8pm_pdt() {
        // Plain-language check: LA, June 21, sunset ~20:08 local (UTC-7), sun in the WNW.
        let s = sunset(34.0522, -118.2437, date(2026, 6, 21)).unwrap();
        let local = s.time - Duration::hours(7);
        assert_eq!(local.format("%Y-%m-%d %H:%M").to_string(), "2026-06-21 20:07");
        assert!((290.0..310.0).contains(&s.azimuth_deg));
    }

    #[test]
    fn sun_is_on_the_horizon_at_sunset() {
        // At the sunset instant the sun's centre sits ~0.833° below a flat horizon
        // (refraction + half the solar disc). Allow a little slack for the 1 s rounding.
        let s = sunset(34.0522, -118.2437, date(2026, 6, 21)).unwrap();
        let pos = sun_position(34.0522, -118.2437, s.time);
        assert!(pos.elevation_deg.abs() < 0.5, "elevation {}", pos.elevation_deg);
    }

    #[test]
    fn polar_day_and_night_have_no_sunset() {
        let tromso = (69.6492, 18.9553);
        assert_eq!(sunset(tromso.0, tromso.1, date(2026, 6, 21)), Err(NoSunset::PolarDay));
        assert_eq!(sunset(tromso.0, tromso.1, date(2026, 12, 21)), Err(NoSunset::PolarNight));
    }

    #[test]
    fn julian_day_matches_known_epoch() {
        // J2000.0 is 2000-01-01 12:00 TT = JD 2451545.0, so midnight is 2451544.5.
        assert_eq!(julian_day(date(2000, 1, 1)), 2451544.5);
    }
}
