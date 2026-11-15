//! Pure: read a typed brief ("sun between two towers from a rooftop in Princeton, late June") into the
//! fields of a `Brief`. This is the offline mock's "understanding", and the fallback when Claude fails.
//! No clock: the caller passes today's local date.

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use contracts::{Area, Event, Framing, Standing};

/// A named place we know without any lookup: centre, a sensible search radius, time zone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preset {
    pub name: &'static str,
    pub words: &'static [&'static str],
    pub lat: f64,
    pub lon: f64,
    pub radius_m: f64,
    pub tz: &'static str,
}

/// Demo places (all have Vexcel coverage). The first is the default.
pub const PRESETS: &[Preset] = &[
    Preset { name: "Downtown LA", words: &["downtown la", "dtla", "downtown los angeles", "los angeles", "bunker hill", " la "], lat: 34.0490, lon: -118.2510, radius_m: 300.0, tz: "America/Los_Angeles" },
    Preset { name: "Princeton", words: &["princeton", "nassau hall", "nassau st"], lat: 40.3487, lon: -74.6593, radius_m: 300.0, tz: "America/New_York" },
    Preset { name: "Santa Monica", words: &["santa monica"], lat: 34.0100, lon: -118.4960, radius_m: 300.0, tz: "America/Los_Angeles" },
    Preset { name: "Downtown Houston", words: &["houston"], lat: 29.7604, lon: -95.3698, radius_m: 300.0, tz: "America/Chicago" },
];

pub fn preset_in(text: &str) -> Option<&'static Preset> {
    let t = format!(" {} ", text.to_lowercase());
    PRESETS.iter().find(|p| p.words.iter().any(|w| t.contains(w)))
}

pub fn preset_named(name: &str) -> Option<&'static Preset> {
    preset_in(name).or_else(|| PRESETS.iter().find(|p| p.name.eq_ignore_ascii_case(name.trim())))
}

impl Preset {
    pub fn area(&self) -> Area {
        Area { lat: self.lat, lon: self.lon, radius_m: self.radius_m, name: None }
    }
}

pub fn event(text: &str) -> Event {
    let t = text.to_lowercase();
    if ["sunrise", "dawn", "morning", "rising sun"].iter().any(|w| t.contains(w)) {
        Event::Sunrise
    } else {
        Event::Sunset
    }
}

pub fn standing(text: &str) -> Standing {
    let t = text.to_lowercase();
    if ["roof", "terrace", "top floor", "balcony", "deck"].iter().any(|w| t.contains(w)) {
        Standing::Roof
    } else if ["street", "ground", "sidewalk", "park", "lawn", "plaza", "walk"].iter().any(|w| t.contains(w)) {
        Standing::Ground
    } else {
        Standing::Any
    }
}

pub fn framing(text: &str) -> Framing {
    let t = text.to_lowercase();
    if ["between", "gap", "framed", "corridor", "canyon", "through"].iter().any(|w| t.contains(w)) {
        Framing::Between
    } else if ["behind", "silhouette", "backlit", "glow"].iter().any(|w| t.contains(w)) {
        Framing::Behind
    } else {
        Framing::Clear
    }
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// The shoot day from the text, relative to `today` (local). `None` if the text names no day.
pub fn date(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let t = text.to_lowercase();
    let words: Vec<&str> = t.split(|c: char| !c.is_alphanumeric() && c != '-').filter(|w| !w.is_empty()).collect();
    // an ISO date wins
    if let Some(d) = words.iter().find_map(|w| NaiveDate::parse_from_str(w, "%Y-%m-%d").ok()) {
        return Some(d);
    }
    if t.contains("today") || t.contains("tonight") || t.contains("this evening") || t.contains("this morning") {
        return Some(today);
    }
    if t.contains("tomorrow") {
        return Some(today + Duration::days(1));
    }
    if t.contains("weekend") {
        return Some(next_weekday(today, Weekday::Sat));
    }
    const DAYS: [(&str, Weekday); 7] = [
        ("monday", Weekday::Mon), ("tuesday", Weekday::Tue), ("wednesday", Weekday::Wed), ("thursday", Weekday::Thu),
        ("friday", Weekday::Fri), ("saturday", Weekday::Sat), ("sunday", Weekday::Sun),
    ];
    if let Some((_, wd)) = DAYS.iter().find(|(n, _)| words.contains(n)) {
        return Some(next_weekday(today, *wd));
    }
    // "late June", "June 24", "24th of june", "mid-july"
    for (i, w) in words.iter().enumerate() {
        let Some(m) = MONTHS.iter().position(|p| w.starts_with(p) && (w.len() == 3 || is_month_word(w))) else { continue };
        let num = |s: Option<&&str>| s.and_then(|s| s.trim_end_matches(|c: char| c.is_alphabetic()).parse::<u32>().ok()).filter(|d| (1..=31).contains(d));
        let before = if i > 0 { words.get(i - 1) } else { None };
        let before2 = if i > 1 { words.get(i - 2) } else { None };
        let day = num(words.get(i + 1))
            .or_else(|| num(before))
            .or_else(|| if before == Some(&"of") { num(before2) } else { None })
            .unwrap_or_else(|| {
                let lead = |k: &str| before.is_some_and(|b| b.contains(k)) || before2.is_some_and(|b| b.contains(k)) || w.contains(k);
                if lead("early") || lead("start") {
                    5
                } else if lead("late") || lead("end") {
                    24
                } else {
                    15
                }
            });
        let month = m as u32 + 1;
        let this_year = NaiveDate::from_ymd_opt(today.year(), month, day);
        return match this_year {
            Some(d) if d >= today => Some(d),
            _ => NaiveDate::from_ymd_opt(today.year() + 1, month, day),
        };
    }
    None
}

fn is_month_word(w: &str) -> bool {
    const FULL: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    FULL.iter().any(|f| f.starts_with(w)) || w == "sept"
}

/// The next `wd` on or after `today`.
fn next_weekday(today: NaiveDate, wd: Weekday) -> NaiveDate {
    let ahead = (7 + wd.num_days_from_monday() as i64 - today.weekday().num_days_from_monday() as i64) % 7;
    today + Duration::days(ahead)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 14).unwrap() // a Saturday
    }

    #[test]
    fn reads_the_shot() {
        let t = "sun setting between two towers, from a rooftop, late June";
        assert_eq!(event(t), Event::Sunset);
        assert_eq!(standing(t), Standing::Roof);
        assert_eq!(framing(t), Framing::Between);
        assert_eq!(event("sunrise over the lake"), Event::Sunrise);
        assert_eq!(standing("from the street"), Standing::Ground);
        assert_eq!(framing("a silhouette of the chapel"), Framing::Behind);
        assert_eq!(framing("sunset"), Framing::Clear);
    }

    #[test]
    fn reads_dates() {
        let d = |s| date(s, today());
        assert_eq!(d("late June"), NaiveDate::from_ymd_opt(2027, 6, 24));
        assert_eq!(d("June 3rd"), NaiveDate::from_ymd_opt(2027, 6, 3));
        assert_eq!(d("on the 20th of december"), NaiveDate::from_ymd_opt(2026, 12, 20));
        assert_eq!(d("early dec"), NaiveDate::from_ymd_opt(2026, 12, 5));
        assert_eq!(d("tonight"), Some(today()));
        assert_eq!(d("tomorrow"), NaiveDate::from_ymd_opt(2026, 11, 15));
        assert_eq!(d("this weekend"), Some(today()));
        assert_eq!(d("on tuesday"), NaiveDate::from_ymd_opt(2026, 11, 17));
        assert_eq!(d("2026-06-24 please"), NaiveDate::from_ymd_opt(2026, 6, 24));
        assert_eq!(d("sunset between towers"), None);
        // "may" the verb is still read as May; acceptable for a mock
        assert!(d("mayonnaise").is_none());
    }

    #[test]
    fn finds_places() {
        assert_eq!(preset_in("a rooftop in Princeton").unwrap().name, "Princeton");
        assert_eq!(preset_in("dtla rooftop").unwrap().name, "Downtown LA");
        assert_eq!(preset_in("sunset in LA").unwrap().name, "Downtown LA");
        assert!(preset_in("sunset in Lagos").is_none());
        assert_eq!(preset_named("Houston").unwrap().tz, "America/Chicago");
    }
}
