//! WZ property types — the value nodes in the WZ object tree.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::wz::mcv::McvHeader;
use crate::wz::types::WzPngFormat;

/// Compressed Canvas pixel bytes, either owned or referencing a shared source buffer.
///
/// `Loaded` is the eager form produced by [`parse_wz_image`](crate::wz::image::parse_wz_image)
/// and all build/edit/xml paths. `Ref` is produced by
/// [`parse_wz_image_lazy`](crate::wz::image::parse_wz_image_lazy): it records an
/// `offset`/`len` into a shared `Arc<[u8]>` (the original image bytes) instead of
/// copying, so parsing a large IMG does not duplicate every frame's pixel data.
/// Both forms deref to `&[u8]`, so consumers read them identically.
#[derive(Debug, Clone)]
pub enum CanvasData {
    Loaded(Vec<u8>),
    Ref { src: Arc<[u8]>, offset: usize, len: usize },
}

impl CanvasData {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            CanvasData::Loaded(v) => v,
            // offset/len are recorded from real read positions, so the range is valid
            // by construction; fall back to empty if a corrupt Ref ever escapes.
            CanvasData::Ref { src, offset, len } => {
                src.get(*offset..*offset + *len).unwrap_or(&[])
            }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            CanvasData::Loaded(v) => v.len(),
            CanvasData::Ref { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

impl std::ops::Deref for CanvasData {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl AsRef<[u8]> for CanvasData {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Default for CanvasData {
    fn default() -> Self {
        CanvasData::Loaded(Vec::new())
    }
}

// Serializes identically to the old `Vec<u8>` field (a byte sequence); always
// deserializes back into the owned `Loaded` form.
impl Serialize for CanvasData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_bytes().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CanvasData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(CanvasData::Loaded(Vec::<u8>::deserialize(deserializer)?))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WzProperty {
    Null,           // 0x00
    Short(i16),     // 0x02
    Int(i32),       // 0x03
    Long(i64),      // 0x14
    Float(f32),     // 0x04
    Double(f64),    // 0x05
    String(String), // 0x08

    SubProperty {
        properties: Vec<(String, WzProperty)>,
    },

    Canvas {
        width: i32,
        height: i32,
        format: WzPngFormat,
        /// WZ scale exponent (`format2`). The pixel data is stored at
        /// `(width >> scale) × (height >> scale)` and must be nearest-neighbor
        /// upscaled to `(width, height)`. Almost always 0. See
        /// [`crate::image::decode_canvas_pixels`].
        #[serde(default)]
        scale: u8,
        properties: Vec<(String, WzProperty)>,
        png_data: CanvasData, // compressed pixel bytes (owned or shared-ref), not yet decoded
    },

    Vector {
        x: i32,
        y: i32,
    },

    Convex {
        points: Vec<(String, WzProperty)>,
    },

    Sound {
        duration_ms: i32,
        data: Vec<u8>,
        header: Vec<u8>,
    },

    Uol(String),

    Lua(Vec<u8>),

    RawData {
        raw_type: u8,
        properties: Vec<(String, WzProperty)>,
        data: Vec<u8>,
    },

    Video {
        video_type: u8,
        properties: Vec<(String, WzProperty)>,
        #[serde(default)]
        data_offset: u64,
        #[serde(default)]
        data_length: u32,
        mcv_header: Option<McvHeader>,
        video_data: Option<Vec<u8>>,
    },
}

impl WzProperty {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            WzProperty::Short(v) => Some(*v as i64),
            WzProperty::Int(v) => Some(*v as i64),
            WzProperty::Long(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            WzProperty::Float(v) => Some(*v as f64),
            WzProperty::Double(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            WzProperty::String(s) => Some(s),
            WzProperty::Uol(s) => Some(s),
            _ => None,
        }
    }

    pub fn children(&self) -> Option<&[(String, WzProperty)]> {
        match self {
            WzProperty::SubProperty { properties, .. } => Some(properties),
            WzProperty::Canvas { properties, .. } => Some(properties),
            WzProperty::Convex { points } => Some(points),
            WzProperty::Video { properties, .. } => Some(properties),
            WzProperty::RawData { properties, .. } if !properties.is_empty() => Some(properties),
            _ => None,
        }
    }

    pub fn get(&self, name: &str) -> Option<&WzProperty> {
        self.children()?
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, p)| p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── as_int ─────────────────────────────────────────────────────

    #[test]
    fn test_as_int_short() {
        assert_eq!(WzProperty::Short(42).as_int(), Some(42));
        assert_eq!(WzProperty::Short(-1).as_int(), Some(-1));
    }

    #[test]
    fn test_as_int_int() {
        assert_eq!(WzProperty::Int(100_000).as_int(), Some(100_000));
        assert_eq!(WzProperty::Int(-1).as_int(), Some(-1));
    }

    #[test]
    fn test_as_int_long() {
        assert_eq!(WzProperty::Long(i64::MAX).as_int(), Some(i64::MAX));
        assert_eq!(WzProperty::Long(i64::MIN).as_int(), Some(i64::MIN));
    }

    #[test]
    fn test_as_int_returns_none_for_non_integers() {
        assert_eq!(WzProperty::Null.as_int(), None);
        assert_eq!(WzProperty::Float(1.0).as_int(), None);
        assert_eq!(WzProperty::String("42".into()).as_int(), None);
    }

    // ── as_float ───────────────────────────────────────────────────

    #[test]
    fn test_as_float_float() {
        let v = WzProperty::Float(1.5).as_float().unwrap();
        assert!((v - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_as_float_double() {
        assert_eq!(WzProperty::Double(2.5).as_float(), Some(2.5));
    }

    #[test]
    fn test_as_float_returns_none_for_non_floats() {
        assert_eq!(WzProperty::Int(1).as_float(), None);
        assert_eq!(WzProperty::Null.as_float(), None);
    }

    // ── as_str ─────────────────────────────────────────────────────

    #[test]
    fn test_as_str_string() {
        assert_eq!(WzProperty::String("hello".into()).as_str(), Some("hello"));
    }

    #[test]
    fn test_as_str_uol() {
        assert_eq!(WzProperty::Uol("../link".into()).as_str(), Some("../link"));
    }

    #[test]
    fn test_as_str_returns_none_for_non_strings() {
        assert_eq!(WzProperty::Int(1).as_str(), None);
        assert_eq!(WzProperty::Null.as_str(), None);
    }

    // ── children ───────────────────────────────────────────────────

    #[test]
    fn test_children_sub_property() {
        let prop = WzProperty::SubProperty {
            properties: vec![("a".into(), WzProperty::Int(1))],
        };
        let kids = prop.children().unwrap();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].0, "a");
    }

    #[test]
    fn test_children_canvas() {
        let prop = WzProperty::Canvas {
            width: 1,
            height: 1,
            format: WzPngFormat::Bgra8888,
            scale: 0,
            properties: vec![("origin".into(), WzProperty::Vector { x: 0, y: 0 })],
            png_data: CanvasData::Loaded(vec![]),
        };
        assert_eq!(prop.children().unwrap().len(), 1);
    }

    #[test]
    fn test_children_video() {
        let prop = WzProperty::Video {
            video_type: 0,
            properties: vec![("fps".into(), WzProperty::Int(30))],
            data_offset: 0,
            data_length: 0,
            mcv_header: None,
            video_data: None,
        };
        assert_eq!(prop.children().unwrap().len(), 1);
    }

    #[test]
    fn test_children_returns_none_for_leaf() {
        assert!(WzProperty::Null.children().is_none());
        assert!(WzProperty::Int(1).children().is_none());
        assert!(WzProperty::String("x".into()).children().is_none());
        assert!(WzProperty::Vector { x: 0, y: 0 }.children().is_none());
    }

    // ── get ────────────────────────────────────────────────────────

    #[test]
    fn test_get_finds_child() {
        let prop = WzProperty::SubProperty {
            properties: vec![
                ("x".into(), WzProperty::Int(10)),
                ("y".into(), WzProperty::Int(20)),
            ],
        };
        assert_eq!(prop.get("x").unwrap().as_int(), Some(10));
        assert_eq!(prop.get("y").unwrap().as_int(), Some(20));
    }

    #[test]
    fn test_get_returns_none_for_missing() {
        let prop = WzProperty::SubProperty {
            properties: vec![("x".into(), WzProperty::Int(10))],
        };
        assert!(prop.get("z").is_none());
    }

    #[test]
    fn test_get_returns_none_on_leaf() {
        assert!(WzProperty::Int(1).get("anything").is_none());
    }
}
