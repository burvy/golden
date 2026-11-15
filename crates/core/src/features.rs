//! Features: the numbers cost functions are written over (docs/metrics.md v2).
//!
//! Each feature is a pure number about one candidate standpoint, with a cost **tier**: Point
//! features only need the standpoint, Skyline features need its skyline over the sun's path
//! (the expensive part of a search). The funnel uses tiers to run cheap gates first and to build
//! a skyline only when an expression asks for one.
//!
//! The sun features are today's `score.rs` numbers, unchanged. Object features (`dist`,
//! `bearing`, `bearing_diff`, `size_deg` at Point tier, `visible` at Ray tier,
//! `sun_bearing_diff` at Skyline tier) come next: they slot in as new `Feature` arms that carry
//! a resolved object, plus rows in `FEATURES`.

use crate::score::{self, Look};
use crate::search::Site;
use contracts::Standing;

/// How much work a feature (or an expression) needs per candidate. Ordered cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// The standpoint itself (and, later, the object list).
    Point,
    /// A few line-of-sight marches through the height map (object visibility, next task).
    Ray,
    /// The skyline over the sun's path.
    Skyline,
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
}

/// One row of the feature table: what the agent sees in `list_features`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FeatureInfo {
    pub feature: Feature,
    pub name: &'static str,
    pub tier: Tier,
    pub about: &'static str,
}

pub const FEATURES: &[FeatureInfo] = &[
    FeatureInfo { feature: Feature::GroundM, name: "ground_m", tier: Tier::Point, about: "surface height where you stand, metres" },
    FeatureInfo { feature: Feature::EyeM, name: "eye_m", tier: Tier::Point, about: "eye height (surface + 1.7 m), metres" },
    FeatureInfo { feature: Feature::Roof, name: "roof", tier: Tier::Point, about: "1 on a roof, 0 at street level" },
    FeatureInfo {
        feature: Feature::SunClear,
        name: "sun_clear",
        tier: Tier::Skyline,
        about: "0..1, sun in view through golden hour and down to the horizon",
    },
    FeatureInfo {
        feature: Feature::SunBetween,
        name: "sun_between",
        tier: Tier::Skyline,
        about: "0..1, sun sets into a narrow gap (1.5-6 deg) between tall sides",
    },
    FeatureInfo {
        feature: Feature::SunBehind,
        name: "sun_behind",
        tier: Tier::Skyline,
        about: "0..1, sun drops behind a near building 1.5-6 deg up (silhouette)",
    },
    FeatureInfo {
        feature: Feature::SunVisibleMin,
        name: "sun_visible_min",
        tier: Tier::Skyline,
        about: "golden-hour minutes with the sun in view",
    },
    FeatureInfo {
        feature: Feature::SunHiddenElev,
        name: "sun_hidden_elev",
        tier: Tier::Skyline,
        about: "sun elevation when it goes behind the skyline, deg (0 = never hidden, 20 = hidden all along)",
    },
    FeatureInfo {
        feature: Feature::GapWidthDeg,
        name: "gap_width_deg",
        tier: Tier::Skyline,
        about: "width of the gap the sun sets into, deg (0 = no gap)",
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

/// A Point-tier feature's value at a standpoint. None for features of a higher tier.
pub fn point_value(f: Feature, site: &Site, eye_above_m: f64) -> Option<f64> {
    match f {
        Feature::GroundM => Some(site.ground_m),
        Feature::EyeM => Some(site.ground_m + eye_above_m),
        Feature::Roof => Some(if site.standing == Standing::Roof { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// A Skyline-tier (sun) feature's value from the candidate's `Look`. None for other tiers.
pub fn sun_value(f: Feature, look: &Look) -> Option<f64> {
    match f {
        Feature::SunClear => Some(score::score_clear(look)),
        Feature::SunBetween => Some(score::score_between(look)),
        Feature::SunBehind => Some(score::score_behind(look)),
        Feature::SunVisibleMin => Some(look.golden_visible_s as f64 / 60.0),
        Feature::SunHiddenElev => Some(look.hidden_elev_deg),
        Feature::GapWidthDeg => Some(look.gap.map_or(0.0, |g| g.width_deg)),
        _ => None,
    }
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
    fn point_features_need_no_skyline() {
        let site = Site { ground_m: 40.0, standing: Standing::Roof, above_street_m: 12.0 };
        assert_eq!(point_value(Feature::EyeM, &site, 1.7), Some(41.7));
        assert_eq!(point_value(Feature::Roof, &site, 1.7), Some(1.0));
        assert_eq!(point_value(Feature::SunClear, &site, 1.7), None);
        assert!(FEATURES.iter().all(|i| (i.tier == Tier::Point) == point_value(i.feature, &site, 1.7).is_some()));
    }
}
