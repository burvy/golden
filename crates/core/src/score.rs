//! How good one spot's view is for a brief: pure functions on a skyline and the sun's path.
//!
//! Search works in two passes. The fast pass, here, ranks every candidate. It uses a
//! `SunTrack` (the sun's path from the flat sunset back into the day, worked out once per
//! search) and a `SkyWindow` (the candidate's skyline over just the azimuths that path
//! crosses). `look` boils the two down to a few numbers: when the sun goes behind something,
//! how many golden-hour minutes it stays in view, and whether it sets into a gap. `score` then
//! turns those into a 0..1 fit for each framing. The exact `SunBlock` shown on a spot card
//! still comes from `sunblock::sun_block`. `why` words that up for filmmakers.
//!
//! The blocking rule matches `sunblock`: something hides the sun when the skyline in the sun's
//! direction is higher than the sun and also above the flat horizon (0°).
//!
//! Everything is written for sunset. For sunrise the track runs forward in time from the flat
//! sunrise, so "hidden" means "not yet risen over the skyline" and the same code applies.

use chrono::{DateTime, Utc};
use contracts::sun::GOLDEN_HOUR_MAX_ELEV_DEG;
use contracts::{Event, Framing, SunBlock};

/// The track ends once the sun is this high: obstructions above it don't matter to a sunset shot.
pub const TRACK_TOP_DEG: f64 = 20.0;
/// A skyline at least this high next to the sun counts as one side of a gap.
pub const FLANK_MIN_DEG: f64 = 3.0;
/// How far either side of the sun to look for the two sides of a gap.
pub const GAP_SEARCH_DEG: f64 = 15.0;
/// Past the edge of a flank, how far to look for its full height.
pub const FLANK_LOOK_DEG: f64 = 3.0;

/// The sun at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackSample {
    pub t: DateTime<Utc>,
    pub az_deg: f64,
    pub elev_deg: f64,
}

/// The sun's path, starting at the flat-horizon event and stepping into the day (back in time
/// for sunset, forward for sunrise), `step_s` apart, up to `TRACK_TOP_DEG`.
#[derive(Debug, Clone, PartialEq)]
pub struct SunTrack {
    pub samples: Vec<TrackSample>,
    pub step_s: i64,
}

/// A skyline over a window of azimuths: `elev[i]` and `dist[i]` are the skyline at
/// `center_az + lo_deg + i * step_deg`. Outside the window the skyline counts as flat (0°).
#[derive(Debug, Clone, PartialEq)]
pub struct SkyWindow {
    pub center_az: f64,
    pub lo_deg: f64,
    pub step_deg: f64,
    pub elev: Vec<f64>,
    pub dist: Vec<Option<f64>>,
}

impl SkyWindow {
    pub fn az_of(&self, i: usize) -> f64 {
        self.center_az + self.lo_deg + i as f64 * self.step_deg
    }

    /// Fractional index of `az` in the window, or None outside it.
    fn index_of(&self, az: f64) -> Option<f64> {
        let x = (wrap180(az - self.center_az) - self.lo_deg) / self.step_deg;
        let last = self.elev.len().checked_sub(1)? as f64;
        (0.0..=last).contains(&x).then_some(x)
    }

    /// Skyline elevation at `az`, linear between neighbouring points; 0 outside the window.
    pub fn elev_at(&self, az: f64) -> f64 {
        let Some(x) = self.index_of(az) else { return 0.0 };
        let i = x.floor() as usize;
        let f = x - i as f64;
        match self.elev.get(i + 1) {
            Some(next) => self.elev[i] + (next - self.elev[i]) * f,
            None => self.elev[i],
        }
    }

    /// Distance to the skyline at the nearest point to `az`.
    pub fn dist_at(&self, az: f64) -> Option<f64> {
        self.index_of(az).and_then(|x| self.dist[x.round() as usize])
    }
}

/// When the sun goes behind the skyline, walking along the track from the flat event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hidden {
    /// In view right down to the flat horizon.
    Never,
    /// `samples[i]` is the last instant in view (sunrise: the first).
    At(usize),
    /// Behind the skyline over the whole track, i.e. already hidden at `TRACK_TOP_DEG`.
    AllTrack,
}

/// A low stretch of skyline that the sun sets into, with something taller on both sides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gap {
    pub width_deg: f64,
    pub left_elev_deg: f64,
    pub right_elev_deg: f64,
    /// Distance to the nearer side.
    pub near_dist_m: Option<f64>,
}

/// What one spot sees of the sun, in numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub hidden: Hidden,
    /// Sun elevation when it goes behind the skyline: 0 for `Never`, `TRACK_TOP_DEG` for `AllTrack`.
    pub hidden_elev_deg: f64,
    /// Distance to what hides it.
    pub hidden_dist_m: Option<f64>,
    /// Golden-hour seconds with the sun up (+6° down to the flat event), and how many in view.
    pub golden_total_s: i64,
    pub golden_visible_s: i64,
    pub gap: Option<Gap>,
}

/// Same rule as `sunblock`: only a skyline above both the sun and the flat horizon hides it.
pub fn blocks(sky: &SkyWindow, s: &TrackSample) -> bool {
    sky.elev_at(s.az_deg) > s.elev_deg.max(0.0)
}

/// Sum up what the sun does seen from a spot with skyline `sky`.
pub fn look(track: &SunTrack, sky: &SkyWindow) -> Look {
    let samples = &track.samples;
    let hidden = match samples.first() {
        None => Hidden::Never,
        Some(first) if !blocks(sky, first) => Hidden::Never,
        Some(_) => match samples.iter().position(|s| !blocks(sky, s)) {
            Some(i) => Hidden::At(i),
            None => Hidden::AllTrack,
        },
    };
    let (hidden_elev_deg, hidden_dist_m) = match hidden {
        Hidden::Never => (0.0, None),
        Hidden::At(i) => (samples[i].elev_deg, sky.dist_at(samples[i].az_deg)),
        Hidden::AllTrack => (TRACK_TOP_DEG, None),
    };

    let golden = samples.iter().filter(|s| s.elev_deg <= GOLDEN_HOUR_MAX_ELEV_DEG);
    let (total, visible) = golden.fold((0, 0), |(n, v), s| (n + 1, v + i64::from(!blocks(sky, s))));

    let gap = match hidden {
        Hidden::Never => samples.first().and_then(|s| gap_at(sky, s.az_deg)),
        Hidden::At(i) => gap_at(sky, samples[i].az_deg),
        Hidden::AllTrack => None,
    };
    Look {
        hidden,
        hidden_elev_deg,
        hidden_dist_m,
        golden_total_s: total * track.step_s,
        golden_visible_s: visible * track.step_s,
        gap,
    }
}

/// The gap around `az` (where the sun is when it was last seen), if the skyline is low there
/// and rises to `FLANK_MIN_DEG` on both sides within `GAP_SEARCH_DEG`.
pub fn gap_at(sky: &SkyWindow, az: f64) -> Option<Gap> {
    if sky.elev_at(az) >= FLANK_MIN_DEG {
        return None;
    }
    let (left_off, left_elev, left_dist) = flank(sky, az, -1.0)?;
    let (right_off, right_elev, right_dist) = flank(sky, az, 1.0)?;
    let near_dist_m = match (left_dist, right_dist) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    Some(Gap { width_deg: left_off + right_off, left_elev_deg: left_elev, right_elev_deg: right_elev, near_dist_m })
}

/// Walk from `az` in direction `side` (-1 left, +1 right) to the first skyline at least
/// `FLANK_MIN_DEG` high. Returns (how far, the flank's highest elevation just past that, its distance).
fn flank(sky: &SkyWindow, az: f64, side: f64) -> Option<(f64, f64, Option<f64>)> {
    let steps = (GAP_SEARCH_DEG / sky.step_deg).ceil() as usize;
    let off = (1..=steps).map(|k| k as f64 * sky.step_deg).find(|o| sky.elev_at(az + side * o) >= FLANK_MIN_DEG)?;
    let edge = az + side * off;
    let look_steps = (FLANK_LOOK_DEG / sky.step_deg).ceil() as usize;
    let top = (0..=look_steps).map(|k| sky.elev_at(edge + side * k as f64 * sky.step_deg)).fold(f64::MIN, f64::max);
    Some((off, top, sky.dist_at(edge)))
}

/// How well a look fits a framing, 0..1 before normalising across the search.
pub fn score(framing: Framing, look: &Look) -> f64 {
    match framing {
        Framing::Clear => score_clear(look),
        Framing::Behind => score_behind(look),
        Framing::Between => score_between(look),
    }
}

/// Clear: as much golden hour in view as possible, and the sun visible down to the horizon.
pub fn score_clear(look: &Look) -> f64 {
    let golden = if look.golden_total_s > 0 { look.golden_visible_s as f64 / look.golden_total_s as f64 } else { 0.0 };
    0.7 * golden + 0.3 * ramp(look.hidden_elev_deg, 6.0, 0.0)
}

/// Behind: the sun goes behind a building while low in a golden sky (best between 1.5° and
/// 6° up), and the building is near, so it reads as a big silhouette.
pub fn score_behind(look: &Look) -> f64 {
    if !matches!(look.hidden, Hidden::At(_)) {
        return 0.0;
    }
    let low = plateau(look.hidden_elev_deg, 0.3, 1.5, 6.0, 15.0);
    let near = look.hidden_dist_m.map_or(0.0, |d| ramp(d, 1000.0, 150.0));
    low * (0.5 + 0.5 * near)
}

/// Between: the sun sets into a narrow gap (best 1.5°-6° wide) with tall sides, and stays in
/// view in it to within 2° of the horizon.
pub fn score_between(look: &Look) -> f64 {
    let Some(gap) = look.gap else { return 0.0 };
    let in_view = ramp(look.hidden_elev_deg, 6.0, 2.0);
    let width = plateau(gap.width_deg, 0.5, 1.5, 6.0, 15.0);
    let tall = ramp(gap.left_elev_deg.min(gap.right_elev_deg), FLANK_MIN_DEG, 12.0);
    in_view * width * (0.5 + 0.5 * tall)
}

/// 0 at `zero_at`, 1 at `one_at`, linear between, clamped. Works either way round.
pub fn ramp(x: f64, zero_at: f64, one_at: f64) -> f64 {
    ((x - zero_at) / (one_at - zero_at)).clamp(0.0, 1.0)
}

/// 0 outside `a..d`, 1 on `b..c`, linear on the slopes.
pub fn plateau(x: f64, a: f64, b: f64, c: f64, d: f64) -> f64 {
    ramp(x, a, b).min(ramp(x, d, c))
}

/// Angle folded into (-180, 180].
pub fn wrap180(a: f64) -> f64 {
    let w = a.rem_euclid(360.0);
    if w > 180.0 {
        w - 360.0
    } else {
        w
    }
}

/// 16-point compass name for an azimuth: 292 -> "WNW".
pub fn compass(az: f64) -> &'static str {
    const NAMES: [&str; 16] =
        ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
    NAMES[((az.rem_euclid(360.0) / 22.5).round() as usize) % 16]
}

/// `HH:MM` local from an ISO 8601 time like `2026-06-21T20:07:41-07:00`.
pub fn hhmm(iso: &str) -> &str {
    iso.get(11..16).unwrap_or(iso)
}

/// Plain-English reasons for a spot card, best first, from the exact `SunBlock` and the look.
/// `place` is the standing line, e.g. "Roof 46 m above the street".
pub fn why(framing: Framing, event: Event, look: &Look, sky: &SkyWindow, sun: &SunBlock, place: String) -> Vec<String> {
    let (sets, behind_verb) = match event {
        Event::Sunset => ("sets", "drops behind"),
        Event::Sunrise => ("rises", "rises from behind"),
    };
    let flat = sun.flat_sunset.as_deref().map(hhmm).unwrap_or("?");
    let hidden = sun.hidden_at.as_deref().map(hhmm);
    let blocker = || match (sun.hidden_az_deg, sun.hidden_elev_deg) {
        (Some(az), Some(elev)) => {
            let dist = sky.dist_at(az).map(|d| format!(" {d:.0} m")).unwrap_or_default();
            format!("a building{dist} {} at {}, {elev:.1}° up", compass(az), hidden.unwrap_or("?"))
        }
        _ => "a building".to_string(),
    };

    let mut out = Vec::new();
    match framing {
        Framing::Clear => out.push(match hidden {
            None => format!("Sun in view all the way down to the horizon ({sets} {flat})"),
            Some(_) => format!("Clear until the sun {behind_verb} {}", blocker()),
        }),
        Framing::Behind => out.push(match hidden {
            Some(_) => format!("Sun {behind_verb} {}", blocker()),
            None => format!("Nothing blocks the sun; it {sets} on the horizon at {flat}"),
        }),
        Framing::Between => out.push(match look.gap {
            Some(g) => {
                let metres = g.near_dist_m.map(|d| format!("{:.0} m ", d * g.width_deg.to_radians())).unwrap_or_default();
                format!(
                    "Sun {sets} into a {metres}({:.1}°) gap between buildings {:.0}° and {:.0}° high at {}",
                    g.width_deg, g.left_elev_deg, g.right_elev_deg, hidden.unwrap_or(flat)
                )
            }
            None => "No gap between buildings where the sun goes down".to_string(),
        }),
    }
    out.push(format!("{} min of golden hour with the sun in view", sun.golden_min_visible));
    out.push(place);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    const STEP: f64 = 0.5;

    /// A window around az 300, -20..20, at `f(az)`.
    fn sky(f: impl Fn(f64) -> (f64, Option<f64>)) -> SkyWindow {
        let mut w = SkyWindow { center_az: 300.0, lo_deg: -20.0, step_deg: STEP, elev: vec![], dist: vec![] };
        for i in 0..=80 {
            let (e, d) = f(w.az_of(i));
            w.elev.push(e);
            w.dist.push(d);
        }
        w
    }

    /// A made-up sun: sets at az 300, climbs 0.125° and moves 0.05° south (az down) per 10 s step.
    fn track() -> SunTrack {
        let t0 = DateTime::from_timestamp(1_782_097_661, 0).unwrap();
        let samples = (0..=200)
            .map(|k| TrackSample {
                t: t0 - Duration::seconds(10 * k),
                az_deg: 300.0 - 0.05 * k as f64,
                elev_deg: -0.5 + 0.125 * k as f64,
            })
            .collect();
        SunTrack { samples, step_s: 10 }
    }

    fn flat_sky() -> SkyWindow {
        sky(|_| (-1.0, Some(500.0)))
    }

    #[test]
    fn flat_skyline_is_never_hidden_and_sees_all_golden_hour() {
        let l = look(&track(), &flat_sky());
        assert_eq!(l.hidden, Hidden::Never);
        // Samples up to 6°: k = 0..=52 -> 53 samples of 10 s.
        assert_eq!((l.golden_total_s, l.golden_visible_s), (530, 530));
        assert!((score_clear(&l) - 1.0).abs() < 1e-9);
        assert_eq!(score_behind(&l), 0.0);
        assert_eq!(score_between(&l), 0.0);
    }

    #[test]
    fn wall_hides_the_sun_at_its_height() {
        let l = look(&track(), &sky(|_| (4.05, Some(120.0))));
        // Sun first clears 4.05° at k = 37 (elev 4.125).
        assert_eq!(l.hidden, Hidden::At(37));
        assert_eq!(l.hidden_elev_deg, 4.125);
        assert_eq!(l.hidden_dist_m, Some(120.0));
        assert!(score_behind(&l) > 0.9, "{}", score_behind(&l));
        assert!(score_clear(&l) < 0.4, "{}", score_clear(&l));
    }

    #[test]
    fn far_or_too_high_obstruction_scores_lower_for_behind() {
        let near = score_behind(&look(&track(), &sky(|_| (4.0, Some(120.0)))));
        let far = score_behind(&look(&track(), &sky(|_| (4.0, Some(2000.0)))));
        let high = score_behind(&look(&track(), &sky(|_| (14.0, Some(120.0)))));
        assert!(near > far && near > high, "{near} {far} {high}");
        assert_eq!(score_behind(&look(&track(), &sky(|_| (30.0, Some(50.0))))), 0.0);
    }

    #[test]
    fn gap_between_two_walls_is_found() {
        // Walls 10° high except a 4° wide gap centred on the sunset az (300).
        let s = sky(|az| if (298.0..=302.0).contains(&az) { (-1.0, Some(800.0)) } else { (10.0, Some(100.0)) });
        let l = look(&track(), &s);
        assert_eq!(l.hidden, Hidden::Never);
        let g = l.gap.expect("gap");
        assert!((g.width_deg - 5.0).abs() < 1e-9, "{g:?}"); // first >= 3° at ±2.5 from 300
        assert_eq!(g.near_dist_m, Some(100.0));
        assert!(score_between(&l) > 0.6, "{}", score_between(&l));
    }

    #[test]
    fn no_gap_when_one_side_is_open() {
        let s = sky(|az| if az > 302.0 { (10.0, Some(100.0)) } else { (-1.0, Some(800.0)) });
        assert_eq!(look(&track(), &s).gap, None);
    }

    #[test]
    fn interpolation_and_outside_the_window() {
        let s = sky(|az| (az - 300.0, Some(10.0)));
        assert!((s.elev_at(300.25) - 0.25).abs() < 1e-9);
        assert_eq!(s.elev_at(340.0), 0.0);
        assert_eq!(s.elev_at(100.0), 0.0);
    }

    #[test]
    fn empty_track_does_not_panic() {
        let l = look(&SunTrack { samples: vec![], step_s: 10 }, &flat_sky());
        assert_eq!((l.hidden, l.golden_total_s, l.gap), (Hidden::Never, 0, None));
    }

    #[test]
    fn small_helpers() {
        assert_eq!(ramp(5.0, 0.0, 10.0), 0.5);
        assert_eq!(ramp(5.0, 10.0, 0.0), 0.5);
        assert_eq!(ramp(-1.0, 0.0, 10.0), 0.0);
        assert_eq!(plateau(3.0, 0.0, 1.0, 6.0, 15.0), 1.0);
        assert_eq!(plateau(10.5, 0.0, 1.0, 6.0, 15.0), 0.5);
        assert_eq!(wrap180(350.0), -10.0);
        assert_eq!(wrap180(-350.0), 10.0);
        assert_eq!(compass(292.0), "WNW");
        assert_eq!(compass(359.0), "N");
        assert_eq!(hhmm("2026-06-21T20:07:41-07:00"), "20:07");
    }
}
