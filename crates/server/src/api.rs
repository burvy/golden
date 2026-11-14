//! Shared bits for every endpoint: the JSON error (`ApiError`), range checks, time zones.

use axum::{
    extract::rejection::{JsonRejection, QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{NaiveDate, Offset, TimeZone};
use chrono_tz::Tz;
use contracts::ApiError;

/// An error response: status + `{"error": "..."}` (docs/contracts.md, Errors).
#[derive(Debug)]
pub struct ApiErr(pub StatusCode, pub String);

pub type ApiResult<T> = Result<T, ApiErr>;

impl ApiErr {
    pub fn bad(msg: impl Into<String>) -> ApiErr {
        ApiErr(StatusCode::BAD_REQUEST, msg.into())
    }
    pub fn not_found(msg: impl Into<String>) -> ApiErr {
        ApiErr(StatusCode::NOT_FOUND, msg.into())
    }
    pub fn impossible(msg: impl Into<String>) -> ApiErr {
        ApiErr(StatusCode::UNPROCESSABLE_ENTITY, msg.into())
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        (self.0, Json(ApiError::new(self.1))).into_response()
    }
}

/// Bad query string (missing, unparsable or unknown parameter) -> 400 with serde's message.
impl From<QueryRejection> for ApiErr {
    fn from(r: QueryRejection) -> ApiErr {
        ApiErr::bad(r.body_text())
    }
}

/// Bad JSON body -> 400 (415 if it isn't sent as JSON).
impl From<JsonRejection> for ApiErr {
    fn from(r: JsonRejection) -> ApiErr {
        // axum says 422 for JSON that parses but has the wrong shape; the contract says 400 (bad or unknown field)
        let status = if r.status() == StatusCode::UNSUPPORTED_MEDIA_TYPE { r.status() } else { StatusCode::BAD_REQUEST };
        ApiErr(status, r.body_text())
    }
}

pub fn check_lat_lon(lat: f64, lon: f64) -> ApiResult<()> {
    if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
        Ok(())
    } else {
        Err(ApiErr::bad("lat must be in [-90, 90] and lon in [-180, 180]"))
    }
}

/// `value` must be in `[lo, hi]` (also rejects NaN).
pub fn check_range(name: &str, value: f64, lo: f64, hi: f64) -> ApiResult<()> {
    if (lo..=hi).contains(&value) {
        Ok(())
    } else {
        Err(ApiErr::bad(format!("{name} must be in [{lo}, {hi}], got {value}")))
    }
}

pub fn parse_tz(name: &str) -> ApiResult<Tz> {
    name.parse::<Tz>().map_err(|_| ApiErr::bad(format!("unknown time zone {name:?} (use an IANA name like America/Los_Angeles)")))
}

pub fn parse_date(s: &str) -> ApiResult<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| ApiErr::bad(format!("bad date {s:?}, expected YYYY-MM-DD")))
}

/// UTC offset in seconds at local noon of `date` (core takes a fixed offset for the day; noon is never in a
/// DST jump, and sunsets are hours away from one).
pub fn utc_offset_s(tz: Tz, date: NaiveDate) -> i32 {
    let noon = date.and_hms_opt(12, 0, 0).expect("noon exists");
    match tz.from_local_datetime(&noon).earliest() {
        Some(t) => t.offset().fix().local_minus_utc(),
        None => tz.offset_from_utc_datetime(&noon).fix().local_minus_utc(),
    }
}

pub fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_follow_daylight_saving() {
        let la = parse_tz("America/Los_Angeles").unwrap();
        assert_eq!(utc_offset_s(la, NaiveDate::from_ymd_opt(2026, 6, 21).unwrap()), -7 * 3600);
        assert_eq!(utc_offset_s(la, NaiveDate::from_ymd_opt(2026, 12, 21).unwrap()), -8 * 3600);
        assert!(parse_tz("Mars/Olympus").is_err());
    }
}
