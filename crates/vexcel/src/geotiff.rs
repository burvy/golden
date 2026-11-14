//! Decode a Vexcel DSM GeoTIFF (bytes in, plain struct out; no IO).
//!
//! What Vexcel sends (measured): 1 band float32, big-endian, 256 px internal tiles,
//! Adobe deflate (32946), EPSG:3857, PixelIsArea, GDAL nodata "-9999", and an XMP
//! packet (tag 700) with the collection name and capture dates.

use std::io::Cursor;
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;

use crate::Error;

/// A decoded DSM raster in EPSG:3857.
#[derive(Debug, Clone, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// 3857 x of the west edge and y of the north edge of pixel (0, 0).
    pub west: f64,
    pub north: f64,
    /// Pixel size in 3857 units.
    pub scale_x: f64,
    pub scale_y: f64,
    pub nodata: f32,
    pub data: Vec<f32>,
    pub info: XmpInfo,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct XmpInfo {
    pub collection: Option<String>,
    pub captured_from: Option<String>,
    pub captured_to: Option<String>,
}

pub fn decode(bytes: &[u8]) -> Result<Raster, Error> {
    let mut limits = Limits::default();
    limits.decoding_buffer_size = 512 * 1024 * 1024; // a level 1 tile is ~85 MB of f32
    let mut dec = Decoder::new(Cursor::new(bytes))?.with_limits(limits);
    let (width, height) = dec.dimensions()?;
    let scale = dec.get_tag_f64_vec(Tag::ModelPixelScaleTag)?;
    let tie = dec.get_tag_f64_vec(Tag::ModelTiepointTag)?;
    if scale.len() < 2 || tie.len() < 6 {
        return Err(Error::BadData("GeoTIFF without pixel scale / tiepoint".into()));
    }
    let nodata = dec
        .find_tag(Tag::GdalNodata)?
        .and_then(|v| v.into_string().ok())
        .and_then(|s| s.trim_end_matches('\0').trim().parse().ok())
        .unwrap_or(contracts::NODATA);
    let xmp = dec.find_tag(Tag::Unknown(700))?.and_then(|v| v.into_string().ok()).unwrap_or_default();
    let data = match dec.read_image()? {
        DecodingResult::F32(v) => v,
        _ => return Err(Error::BadData("DSM is not float32".into())),
    };
    // tiepoint maps raster (i, j) to model (x, y); PixelIsArea, so raster (0, 0) is the corner
    let (i, j, x, y) = (tie[0], tie[1], tie[3], tie[4]);
    Ok(Raster {
        width,
        height,
        west: x - i * scale[0],
        north: y + j * scale[1],
        scale_x: scale[0],
        scale_y: scale[1],
        nodata,
        data,
        info: parse_xmp(&xmp),
    })
}

/// Pull the few fields we keep out of Vexcel's XMP packet.
pub fn parse_xmp(xmp: &str) -> XmpInfo {
    XmpInfo {
        collection: xml_field(xmp, "Vexcel:collection"),
        captured_from: xml_field(xmp, "Vexcel:first-capture-date"),
        captured_to: xml_field(xmp, "Vexcel:last-capture-date"),
    }
}

fn xml_field(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&format!("</{tag}>"))?;
    Some(xml[start..end].to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tiff::encoder::{colortype::Gray32Float, TiffEncoder};

    /// A small GeoTIFF shaped like Vexcel's (corner tiepoint form).
    pub(crate) fn make_tiff(w: u32, h: u32, west: f64, north: f64, scale: f64, data: &[f32]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut enc = TiffEncoder::new(&mut buf).unwrap();
            let mut img = enc.new_image::<Gray32Float>(w, h).unwrap();
            img.encoder().write_tag(Tag::ModelPixelScaleTag, &[scale, scale, 0.0][..]).unwrap();
            img.encoder().write_tag(Tag::ModelTiepointTag, &[0.0, 0.0, 0.0, west, north, 0.0][..]).unwrap();
            img.encoder().write_tag(Tag::GdalNodata, "-9999").unwrap();
            img.encoder()
                .write_tag(Tag::Unknown(700), "<Vexcel:collection>test-coll</Vexcel:collection>")
                .unwrap();
            img.write_data(data).unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn decodes_georeference_and_values() {
        let data: Vec<f32> = (0..12).map(|v| v as f32).collect();
        let r = decode(&make_tiff(4, 3, -100.0, 50.0, 0.5, &data)).unwrap();
        assert_eq!((r.width, r.height), (4, 3));
        assert_eq!((r.west, r.north, r.scale_x), (-100.0, 50.0, 0.5));
        assert_eq!(r.nodata, -9999.0);
        assert_eq!(r.data, data);
        assert_eq!(r.info.collection.as_deref(), Some("test-coll"));
    }

    #[test]
    fn centre_tiepoint_is_moved_to_corner() {
        // Vexcel puts the tiepoint at the raster centre, e.g. (485, 487.5)
        let mut buf = Cursor::new(Vec::new());
        {
            let mut enc = TiffEncoder::new(&mut buf).unwrap();
            let mut img = enc.new_image::<Gray32Float>(2, 2).unwrap();
            img.encoder().write_tag(Tag::ModelPixelScaleTag, &[0.5, 0.5, 0.0][..]).unwrap();
            img.encoder().write_tag(Tag::ModelTiepointTag, &[1.0, 1.0, 0.0, 10.0, 20.0, 0.0][..]).unwrap();
            img.write_data(&[0.0f32; 4]).unwrap();
        }
        let r = decode(&buf.into_inner()).unwrap();
        assert_eq!((r.west, r.north), (9.5, 20.5));
    }

    #[test]
    fn xmp_fields() {
        let x = "<a><Vexcel:first-capture-date>Sun Apr 19 2026</Vexcel:first-capture-date>\
                 <Vexcel:collection>n34w118-us-ca-losangeles-2026</Vexcel:collection></a>";
        let i = parse_xmp(x);
        assert_eq!(i.collection.as_deref(), Some("n34w118-us-ca-losangeles-2026"));
        assert_eq!(i.captured_from.as_deref(), Some("Sun Apr 19 2026"));
        assert_eq!(i.captured_to, None);
    }
}
