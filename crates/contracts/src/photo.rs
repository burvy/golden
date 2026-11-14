//! `PhotoMeta` (docs/contracts.md#photometa): a Vexcel aerial image of the view.

use serde::{Deserialize, Serialize};

pub const DEFAULT_PHOTO_PX: u32 = 1024;
pub const MAX_PHOTO_PX: u32 = 2048;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhotoMeta {
    pub id: String,
    pub kind: PhotoKind,
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    /// Direction the camera looks (oblique); None for ortho.
    pub look_az_deg: Option<f64>,
    /// `YYYY-MM-DD`.
    pub captured: Option<String>,
    /// Must be shown next to the image.
    pub attribution: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhotoKind {
    #[default]
    Oblique,
    Ortho,
}
