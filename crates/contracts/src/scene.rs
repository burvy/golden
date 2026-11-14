//! `SceneMeta` (docs/contracts.md#scenemeta--binary): 3D view heights in rings of decreasing detail.
//! Includes the pure u16 decimetre encoding so the server and tests agree on it.

use serde::{Deserialize, Serialize};

pub const SCENE_ENCODING: &str = "u16le_dm";
pub const SCENE_NODATA: u16 = u16::MAX;
/// Cells per ring side for `detail=phone` and `detail=desktop`.
pub const PHONE_RING_PX: u32 = 256;
pub const DESKTOP_RING_PX: u32 = 512;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneMeta {
    pub id: String,
    pub center_lat: f64,
    pub center_lon: f64,
    pub base_m: f64,
    pub encoding: String,
    pub nodata: u16,
    pub rings: Vec<SceneRing>,
    pub bytes: u64,
    pub data_url: String,
    pub attribution: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneRing {
    pub cell_m: f64,
    pub width: u32,
    pub height: u32,
    pub byte_offset: u64,
}

impl SceneRing {
    pub fn byte_len(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 2
    }
}

/// Height in metres -> u16 decimetres above `base_m`. Below base clamps to 0, above range to 65534.
pub fn encode_dm(height_m: f32, base_m: f64, nodata: f32) -> u16 {
    if height_m == nodata || !height_m.is_finite() {
        return SCENE_NODATA;
    }
    let dm = ((f64::from(height_m) - base_m) * 10.0).round();
    dm.clamp(0.0, f64::from(SCENE_NODATA - 1)) as u16
}

/// u16 decimetres -> height in metres, or None for nodata.
pub fn decode_dm(v: u16, base_m: f64) -> Option<f64> {
    (v != SCENE_NODATA).then(|| base_m + f64::from(v) / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimetre_round_trip() {
        let v = encode_dm(141.83, 70.0, -9999.0);
        assert_eq!(v, 718);
        assert!((decode_dm(v, 70.0).unwrap() - 141.8).abs() < 1e-9);
    }

    #[test]
    fn nodata_and_clamping() {
        assert_eq!(encode_dm(-9999.0, 70.0, -9999.0), SCENE_NODATA);
        assert_eq!(encode_dm(10.0, 70.0, -9999.0), 0);
        assert_eq!(encode_dm(1e6, 70.0, -9999.0), SCENE_NODATA - 1);
        assert_eq!(decode_dm(SCENE_NODATA, 70.0), None);
    }
}
