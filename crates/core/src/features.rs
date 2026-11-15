//! Features: the numbers cost functions are written over (docs/metrics.md v2).
//!
//! Each feature is a pure number about one candidate standpoint, with a cost **tier**: Point
//! features only need the standpoint, Skyline features need its skyline over the sun's path
//! (the expensive part of a search). The funnel uses tiers to run cheap gates first and to build
//! a skyline only when an expression asks for one.
//!
//! The sun features are today's `score.rs` numbers, unchanged. Object features measure objects
//! from the standpoint (`objects.rs`): `dist`, `bearing`, `bearing_diff`, `size_deg`, `in_frame`
//! at Point tier, `visible` at Ray tier, `sun_bearing_diff` at Skyline tier. An expression names
//! their objects with `obj` (or `a` and `b`); a `Call` is a feature plus which of the cost
//! function's objects it looks at.
//!
//! An AI tier (task 058) would go after Skyline: a new `Tier` arm, `Feature` arms whose value
//! comes from a model's verdict, looked up like any other feature.

use contracts::{DsmGrid, Object, Standing};

use crate::objects::{self, View};
use crate::score::{self, Look, SunTrack};
use crate::search::Site;

/// How much work a feature (or an expression) needs per candidate. Ordered cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// The standpoint itself, the brief and the objects' positions.
    Point,
    /// A line-of-sight march through the height map per object (`visible`).
    Ray,
    /// The skyline over the sun's path.
    Skyline,
    // Later (task 058): `Ai`, a model call such as a vision question, ordered last so it only runs
    // on candidates every cheaper gate kept. Its value would come from an input map the server
    // fills, looked up like any other feature.
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Point => "point",
            Tier::Ray => "ray",
            Tier::Skyline => "skyline",
        }
    }
}

/// A feature an expression can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    GroundM,
    EyeM,
    Roof,
    SunClear,
    SunBetween,
    SunBehind,
    SunVisibleMin,
    SunHiddenElev,
    GapWidthDeg,
    LensDeg,
    Dist,
    Bearing,
    BearingDiff,
    SizeDeg,
    InFrame,
    Visible,
    SunBearingDiff,
}

/// Which objects a feature takes, and under which keys in the expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Takes {
    Nothing,
    /// `"obj"`.
    One,
    /// `"a"` and `"b"`.
    Two,
}

/// Index into a cost function's object list (`expr::Checked::refs`).
pub type Slot = u8;

/// A feature as used in an expression: the feature and the objects it looks at.
/// `objs` is `[obj, None]` for one object, `[a, b]` for two, `[None, None]` for none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Call {
    pub f: Feature,
    pub objs: [Option<Slot>; 2],
}

impl Call {
    /// A feature that takes no objects.
    pub fn plain(f: Feature) -> Call {
        Call { f, objs: [None, None] }
    }
}

/// One row of the feature table: what the agent sees in `list_features`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FeatureInfo {
    pub feature: Feature,
    pub name: &'static str,
    pub tier: Tier,
    pub takes: Takes,
    pub about: &'static str,
}

pub const FEATURES: &[FeatureInfo] = &[
    FeatureInfo { feature: Feature::GroundM, name: "ground_m", tier: Tier::Point, takes: Takes::Nothing, about: "surface height where you stand, metres" },
    FeatureInfo { feature: Feature::EyeM, name: "eye_m", tier: Tier::Point, takes: Takes::Nothing, about: "eye height (surface + 1.7 m), metres" },
    FeatureInfo { feature: Feature::Roof, name: "roof", tier: Tier::Point, takes: Takes::Nothing, about: "1 on a roof, 0 at street level" },
    FeatureInfo {
        feature: Feature::SunClear,
        name: "sun_clear",
        tier: Tier::Skyline,
        takes: Takes::Nothing,
        about: "0..1, sun in view through golden hour and down to the horizon",
    },
    FeatureInfo {
        feature: Feature::SunBetween,
        name: "sun_between",
        tier: Tier::Skyline,
        takes: Takes::Nothing,
        about: "0..1, sun sets into a narrow gap (1.5-6 deg) between tall sides",
    },
    FeatureInfo {
        feature: Feature::SunBehind,
        name: "sun_behind",
        tier: Tier::Skyline,
        takes: Takes::Nothing,
        about: "0..1, sun drops behind a near building 1.5-6 deg up (silhouette)",
    },
    FeatureInfo {
        feature: Feature::SunVisibleMin,
        name: "sun_visible_min",
        tier: Tier::Skyline,
        takes: Takes::Nothing,
        about: "golden-hour minutes with the sun in view",
    },
    FeatureInfo {
        feature: Feature::SunHiddenElev,
        name: "sun_hidden_elev",
        tier: Tier::Skyline,
        takes: Takes::Nothing,
        about: "sun elevation when it goes behind the skyline, deg (0 = never hidden, 20 = hidden all along)",
    },
    FeatureInfo {
        feature: Feature::GapWidthDeg,
        name: "gap_width_deg",
        tier: Tier::Skyline,
        takes: Takes::Nothing,
        about: "width of the gap the sun sets into, deg (0 = no gap)",
    },
    FeatureInfo {
        feature: Feature::LensDeg,
        name: "lens_deg",
        tier: Tier::Point,
        takes: Takes::Nothing,
        about: "the brief's horizontal lens angle, deg (60 = about 30 mm on full frame)",
    },
    FeatureInfo {
        feature: Feature::Dist,
        name: "dist",
        tier: Tier::Point,
        takes: Takes::One,
        about: "ground distance from you to obj's centre, metres",
    },
    FeatureInfo {
        feature: Feature::Bearing,
        name: "bearing",
        tier: Tier::Point,
        takes: Takes::One,
        about: "compass bearing from you to obj, deg 0..360 (0 north, 90 east)",
    },
    FeatureInfo {
        feature: Feature::BearingDiff,
        name: "bearing_diff",
        tier: Tier::Point,
        takes: Takes::Two,
        about: "bearing of a minus bearing of b, deg -180..180; > 0 means a is right of b in the frame, 0 means lined up",
    },
    FeatureInfo {
        feature: Feature::SizeDeg,
        name: "size_deg",
        tier: Tier::Point,
        takes: Takes::One,
        about: "how tall obj looks: angle from its base to its top, deg",
    },
    FeatureInfo {
        feature: Feature::InFrame,
        name: "in_frame",
        tier: Tier::Point,
        takes: Takes::Two,
        about: "0..1, 1 when a and b are within half the lens angle of each other, 0 when a whole lens angle apart",
    },
    FeatureInfo {
        feature: Feature::Visible,
        name: "visible",
        tier: Tier::Ray,
        takes: Takes::One,
        about: "0..1, share of obj (5 heights from base to top on its near side) you can see over the height map",
    },
    FeatureInfo {
        feature: Feature::SunBearingDiff,
        name: "sun_bearing_diff",
        tier: Tier::Skyline,
        takes: Takes::One,
        about: "sun azimuth where you last see it (it sets or goes behind the skyline) minus obj's bearing, deg -180..180",
    },
];

impl Feature {
    pub fn info(self) -> &'static FeatureInfo {
        FEATURES.iter().find(|i| i.feature == self).expect("every feature has a row")
    }

    pub fn tier(self) -> Tier {
        self.info().tier
    }

    pub fn name(self) -> &'static str {
        self.info().name
    }

    pub fn by_name(name: &str) -> Option<Feature> {
        FEATURES.iter().find(|i| i.name == name).map(|i| i.feature)
    }
}

/// A Point-tier feature's value at a standpoint, with the call's objects already looked up.
/// None for features of a higher tier. A missing object (which `expr::check` rules out) gives NaN,
/// which scores 0.
pub fn point_value(f: Feature, site: &Site, view: &View, objs: [Option<&Object>; 2]) -> Option<f64> {
    let one = |g: fn(&View, &Object) -> f64| Some(objs[0].map_or(f64::NAN, |a| g(view, a)));
    let two = |g: fn(&View, &Object, &Object) -> f64| match objs {
        [Some(a), Some(b)] => Some(g(view, a, b)),
        _ => Some(f64::NAN),
    };
    match f {
        Feature::GroundM => Some(site.ground_m),
        Feature::EyeM => Some(view.eye_m),
        Feature::Roof => Some(if site.standing == Standing::Roof { 1.0 } else { 0.0 }),
        Feature::LensDeg => Some(view.lens_deg),
        Feature::Dist => one(objects::dist),
        Feature::Bearing => one(objects::bearing),
        Feature::SizeDeg => one(objects::size_deg),
        Feature::BearingDiff => two(objects::bearing_diff),
        Feature::InFrame => two(objects::in_frame),
        _ => None,
    }
}

/// A Ray-tier feature's value: a line-of-sight march to `obj`. None for other tiers.
pub fn ray_value(f: Feature, grid: &DsmGrid, view: &View, obj: Option<&Object>) -> Option<f64> {
    match f {
        Feature::Visible => Some(obj.map_or(f64::NAN, |o| objects::visible(grid, view, o))),
        _ => None,
    }
}

/// A Skyline-tier (sun) feature's value from the candidate's `Look` (and `track`, the sun's path
/// it was computed on). None for other tiers.
pub fn sun_value(f: Feature, look: &Look, track: &SunTrack, view: &View, obj: Option<&Object>) -> Option<f64> {
    match f {
        Feature::SunClear => Some(score::score_clear(look)),
        Feature::SunBetween => Some(score::score_between(look)),
        Feature::SunBehind => Some(score::score_behind(look)),
        Feature::SunVisibleMin => Some(look.golden_visible_s as f64 / 60.0),
        Feature::SunHiddenElev => Some(look.hidden_elev_deg),
        Feature::GapWidthDeg => Some(look.gap.map_or(0.0, |g| g.width_deg)),
        Feature::SunBearingDiff => Some(obj.map_or(f64::NAN, |o| objects::sun_bearing_diff(view, sun_seen_az(track, look), o))),
        _ => None,
    }
}

/// Azimuth of the sun where it's last seen: where it goes behind the skyline, or the flat
/// sunset (sunrise) if it never does. If it's hidden all along, also the flat event.
pub fn sun_seen_az(track: &SunTrack, look: &Look) -> f64 {
    let at = match look.hidden {
        score::Hidden::At(i) => i,
        _ => 0,
    };
    track.samples.get(at).map_or(f64::NAN, |s| s.az_deg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_round_trip() {
        for (i, a) in FEATURES.iter().enumerate() {
            assert_eq!(Feature::by_name(a.name), Some(a.feature));
            assert!(FEATURES[i + 1..].iter().all(|b| b.name != a.name && b.feature != a.feature), "{} twice", a.name);
        }
        assert_eq!(Feature::by_name("nope"), None);
    }

    #[test]
    fn each_feature_has_a_value_at_its_tier_only() {
        let site = Site { ground_m: 40.0, standing: Standing::Roof, above_street_m: 12.0 };
        let view = View { lat: 34.049, lon: -118.251, eye_m: 41.7, lens_deg: 50.0 };
        let o = Object {
            id: "x".into(),
            kind: "tree".into(),
            lat: 34.05,
            lon: -118.251,
            base_m: 0.0,
            top_m: 10.0,
            radius_m: 2.0,
            source: "test".into(),
        };
        let objs = [Some(&o), Some(&o)];
        assert_eq!(point_value(Feature::EyeM, &site, &view, objs), Some(41.7));
        assert_eq!(point_value(Feature::Roof, &site, &view, objs), Some(1.0));
        assert_eq!(point_value(Feature::LensDeg, &site, &view, objs), Some(50.0));
        assert_eq!(point_value(Feature::BearingDiff, &site, &view, objs), Some(0.0));
        assert!(point_value(Feature::Dist, &site, &view, [None, None]).unwrap().is_nan(), "missing object scores 0");
        assert_eq!(point_value(Feature::SunClear, &site, &view, objs), None);
        let grid = DsmGrid {
            meta: contracts::DsmMeta {
                center_lat: 34.049,
                center_lon: -118.251,
                cell_m: 1.0,
                width: 1,
                height: 1,
                nodata: contracts::NODATA,
                source: contracts::DsmSource {
                    provider: "t".into(),
                    collection: None,
                    captured_from: None,
                    captured_to: None,
                    native_res_m: 1.0,
                },
            },
            data: vec![0.0],
        };
        let look = Look { hidden: score::Hidden::Never, hidden_elev_deg: 0.0, hidden_dist_m: None, golden_total_s: 0, golden_visible_s: 0, gap: None };
        let track = SunTrack { samples: Vec::new(), step_s: 10 };
        for i in FEATURES {
            let p = point_value(i.feature, &site, &view, objs).is_some();
            let r = ray_value(i.feature, &grid, &view, Some(&o)).is_some();
            let s = sun_value(i.feature, &look, &track, &view, Some(&o)).is_some();
            assert_eq!([p, r, s], [i.tier == Tier::Point, i.tier == Tier::Ray, i.tier == Tier::Skyline], "{}", i.name);
        }
    }
}
