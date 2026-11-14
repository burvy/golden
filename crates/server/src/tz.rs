//! Time zones: contract times are local with the place's offset (docs/contracts.md "Conventions").

use chrono::{DateTime, NaiveDate, Offset, TimeZone, Utc};
use chrono_tz::Tz;

pub const DEFAULT_TZ: &str = "America/Los_Angeles";

pub fn parse(name: &str) -> Result<Tz, String> {
    name.parse::<Tz>().map_err(|_| format!("unknown time zone {name:?} (use an IANA name like America/New_York)"))
}

/// UTC offset in seconds that applies on `date` (taken at local noon, so DST days are right for the evening).
pub fn offset_s(tz: Tz, date: NaiveDate) -> i32 {
    let noon = date.and_hms_opt(12, 0, 0).expect("noon exists");
    tz.offset_from_local_datetime(&noon).single().map(|o| o.fix().local_minus_utc()).unwrap_or(0)
}

/// `2026-06-21T20:07:41-07:00`
pub fn iso(tz: Tz, t: DateTime<Utc>) -> String {
    t.with_timezone(&tz).format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

/// Local midnight at the start of `date`, in UTC.
pub fn day_start(tz: Tz, date: NaiveDate) -> DateTime<Utc> {
    let midnight = date.and_hms_opt(0, 0, 0).expect("midnight exists");
    tz.from_local_datetime(&midnight)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&midnight))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_follow_dst() {
        let la = parse("America/Los_Angeles").unwrap();
        assert_eq!(offset_s(la, NaiveDate::from_ymd_opt(2026, 6, 21).unwrap()), -7 * 3600);
        assert_eq!(offset_s(la, NaiveDate::from_ymd_opt(2026, 12, 21).unwrap()), -8 * 3600);
        assert!(parse("Mars/Olympus").is_err());
    }

    #[test]
    fn iso_has_the_local_offset() {
        let ny = parse("America/New_York").unwrap();
        let t = Utc.with_ymd_and_hms(2026, 11, 14, 21, 43, 5).unwrap();
        assert_eq!(iso(ny, t), "2026-11-14T16:43:05-05:00");
        assert_eq!(day_start(ny, NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()), Utc.with_ymd_and_hms(2026, 11, 14, 5, 0, 0).unwrap());
    }
}
